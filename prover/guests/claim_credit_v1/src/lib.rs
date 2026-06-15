//! `claim_credit_v1` — the ESCAPE-HATCH guest (PRODUCTION_GAP G2). Pure Rust; testable natively
//! and runs unchanged in the RISC Zero guest.
//!
//! "ZK as unconditional claimability." Where `settle_credit_v1` proves the WHOLE payout set (a
//! keeper settles everyone at once), this proves ONE buyer's allocation, so that buyer can be
//! paid without the keeper executing — the keeper has no power to withhold. The claimant supplies
//! the PUBLIC position commitments (recoverable from `buy_protection` tx history) and their OWN
//! opening; every other buyer's opening stays private. The output is a single-allocation journal
//! in the SAME 116-byte format, so a claimable settlement variant verifies it exactly like a full
//! settlement and pays just the claimant.
//!
//! Reuses credit_v1's exact primitives + determination math (credit_v1's source is untouched, so
//! its image_id stays pinned). Soundness bindings are identical: the claim is bound to the
//! committed config, snapshot, terms, deadline, and position_root — a claimant cannot fabricate a
//! payout or claim a position that is not in the committed root.

use sha2::{Digest, Sha256};

pub use settle_credit_v1::{
    commitment, config_hash, derive_instrument_id, snapshot_root, terms_hash, Allocation,
    ConfigFields, Holder, Journal, Payment, Position, Terms, BPS_DENOM,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClaimInputs {
    // --- instrument binding (public) ---
    pub type_id_xdr: Vec<u8>,
    pub rules_version: u32,
    pub config: ConfigFields,
    pub instrument_id: [u8; 32],
    // --- this settlement ---
    pub epoch: u32,
    pub deadline: u64,
    pub terms: Terms,
    pub collateral: i128,
    // --- determination witness ---
    pub snapshot: Vec<Holder>,
    pub payments: Vec<Payment>,
    // --- the position set: PUBLIC commitments (all buyers) + the claimant's own opening ---
    pub commitments: Vec<[u8; 32]>,
    pub claimant_index: u32,
    pub claimant: Position,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClaimError {
    /// Σ shortfall == 0: no default, nothing to claim (the guest panics — no proof can exist).
    NoDefault,
    InstrumentMismatch,
    SnapshotMismatch,
    TermsMismatch,
    DeadlineMismatch,
    /// `commitments` don't fold to the committed `position_root`.
    PositionMismatch,
    /// The claimant's opening does not match `commitments[claimant_index]` (not a real holder).
    CommitmentMismatch,
    /// `claimant_index` is out of range.
    BadIndex,
    BadInput,
    Overflow,
    Insolvent,
}

/// `position_root` over the raw commitments — the SAME sha256 chain credit_v1 / the vault fold.
fn root_from_commitments(commitments: &[[u8; 32]]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for c in commitments {
        let mut h = Sha256::new();
        h.update(acc);
        h.update(c);
        acc = h.finalize().into();
    }
    acc
}

/// `allocation_root` over a single allocation — credit_v1's fold, here with one leaf.
fn single_allocation_root(a: &Allocation) -> [u8; 32] {
    settle_credit_v1::allocation_root(core::slice::from_ref(a))
}

/// Prove the claimant's own allocation. Any `Err` is a guest panic (no proof). Reuses credit_v1's
/// determination so the claimant's amount is byte-identical to what a full settlement would pay.
pub fn claim(inputs: &ClaimInputs) -> Result<(Allocation, Journal), ClaimError> {
    // 0. bind config (M1/M2) — identical to settle_credit_v1.
    let ch = config_hash(&inputs.config);
    if derive_instrument_id(&inputs.type_id_xdr, inputs.rules_version, &ch) != inputs.instrument_id {
        return Err(ClaimError::InstrumentMismatch);
    }
    if snapshot_root(&inputs.snapshot) != inputs.config.snapshot_root {
        return Err(ClaimError::SnapshotMismatch);
    }
    if terms_hash(&inputs.terms) != inputs.config.terms_hash {
        return Err(ClaimError::TermsMismatch);
    }
    let committed_deadline = inputs
        .config
        .epoch_deadlines
        .iter()
        .find(|(e, _)| *e == inputs.epoch)
        .map(|(_, d)| *d)
        .ok_or(ClaimError::DeadlineMismatch)?;
    if inputs.deadline != committed_deadline {
        return Err(ClaimError::DeadlineMismatch);
    }

    // 1. the public commitment set must fold to the committed position_root.
    if root_from_commitments(&inputs.commitments) != inputs.position_root {
        return Err(ClaimError::PositionMismatch);
    }
    // 2. the claimant must be a real holder: their opening reproduces commitments[index].
    let idx = inputs.claimant_index as usize;
    if idx >= inputs.commitments.len() {
        return Err(ClaimError::BadIndex);
    }
    if inputs.claimant.cover <= 0 {
        return Err(ClaimError::BadInput);
    }
    let claim_commit = commitment(&inputs.claimant.buyer, inputs.claimant.cover, &inputs.claimant.salt);
    if claim_commit != inputs.commitments[idx] {
        return Err(ClaimError::CommitmentMismatch);
    }

    // 3. determination — IDENTICAL severity math to credit_v1 (owed / shortfall over the book).
    let rate = inputs.terms.coupon_rate_bps as i128;
    let mut sum_owed: i128 = 0;
    let mut sum_short: i128 = 0;
    for h in &inputs.snapshot {
        if h.balance < 0 {
            return Err(ClaimError::BadInput);
        }
        if !h.has_trustline {
            continue;
        }
        let owed = h.balance.checked_mul(rate).ok_or(ClaimError::Overflow)? / BPS_DENOM;
        sum_owed = sum_owed.checked_add(owed).ok_or(ClaimError::Overflow)?;
        let mut paid: i128 = 0;
        if !h.frozen {
            for p in &inputs.payments {
                if p.holder == h.id && p.paid_at <= inputs.deadline && !p.clawed_back {
                    if p.amount < 0 {
                        return Err(ClaimError::BadInput);
                    }
                    paid = paid.checked_add(p.amount).ok_or(ClaimError::Overflow)?;
                }
            }
        }
        let short = if owed > paid { owed - paid } else { 0 };
        sum_short = sum_short.checked_add(short).ok_or(ClaimError::Overflow)?;
    }
    if sum_short == 0 {
        return Err(ClaimError::NoDefault);
    }

    // 4. the claimant's pro-rata payout — same formula, capped at their cover.
    let cover = inputs.claimant.cover;
    let raw = cover.checked_mul(sum_short).ok_or(ClaimError::Overflow)? / sum_owed;
    let payout = if raw > cover { cover } else { raw };
    if payout <= 0 {
        return Err(ClaimError::NoDefault); // this buyer is owed nothing — nothing to claim
    }
    if payout > inputs.collateral {
        return Err(ClaimError::Insolvent);
    }

    let alloc = Allocation { buyer: inputs.claimant.buyer.clone(), amount: payout };
    let journal = Journal {
        instrument_id: inputs.instrument_id,
        epoch: inputs.epoch,
        deadline: inputs.deadline,
        position_root: inputs.position_root,
        allocation_root: single_allocation_root(&alloc),
        total_payout: u64::try_from(payout).map_err(|_| ClaimError::Overflow)?,
    };
    Ok((alloc, journal))
}

#[cfg(test)]
mod test;
