//! `settle_credit_v2` — instance #1, ATTESTED (PRODUCTION_GAP G1). Pure Rust so it is testable
//! natively AND runs unchanged inside the RISC Zero guest.
//!
//! THE VERSIONING LAW, AND THE TRUST-MODEL HARDENING, IN ONE GUEST. credit_v1's headline caveat
//! is that a proof guarantees computation over the *supplied* payment data, not its canonicity
//! (the keeper supplies it). credit_v2 closes that: it verifies, IN-GUEST, an issuer Ed25519
//! signature over the payment snapshot, with the issuer's public key committed in `terms_hash`.
//! "Trust the keeper's data" becomes "trust the issuer's signature" — a keeper can no longer
//! fabricate or omit payments and still produce a proof.
//!
//! This is a SEPARATE type with its OWN image_id. credit_v1's source is never touched (its
//! image_id stays pinned, the versioning law). credit_v2 REUSES credit_v1's exact generic
//! primitives (Poseidon commitment, sha256 roots, flat config_hash, the 116-byte journal), so
//! the SAME generic vault, settlement, and factory accept an attested instrument unchanged — only
//! the rule (add an attestation check) is new.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

// The generic surfaces + shared types come straight from credit_v1 — identical bytes on-chain.
pub use settle_credit_v1::{
    allocation_root, commitment, config_hash, derive_instrument_id, position_root, snapshot_root,
    Allocation, ConfigFields, Holder, Journal, Payment, Position,
};

pub const BPS_DENOM: i128 = 10_000;

/// credit_v2 terms: the coupon rate AND the issuer's Ed25519 public key whose signature must
/// attest the payment snapshot. Committed via `terms_hash`, so the trusted key is bound at deploy
/// (a prover cannot swap the key without changing `instrument_id`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Terms {
    pub coupon_rate_bps: u32,
    pub issuer_pubkey: [u8; 32],
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Inputs {
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
    // --- private witness ---
    pub snapshot: Vec<Holder>,
    pub payments: Vec<Payment>,
    /// Issuer's Ed25519 signature (64 bytes) over `payments_digest(payments)` — the attestation
    /// (G1). `Vec<u8>` because serde derives don't cover `[u8; 64]`; length is checked on verify.
    pub attestation: Vec<u8>,
    pub positions: Vec<Position>,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SettleError {
    NoDefault,
    InstrumentMismatch,
    SnapshotMismatch,
    TermsMismatch,
    DeadlineMismatch,
    PositionMismatch,
    Insolvent,
    Overflow,
    BadInput,
    /// The issuer signature over the payment snapshot did not verify — the data is not attested (G1).
    AttestationInvalid,
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// `terms_hash` = sha256(coupon_rate_bps_be ‖ issuer_pubkey). Binds BOTH the rate and the trusted
/// issuer key to the committed config — distinct from credit_v1's (which commits only the rate),
/// so credit_v2 is genuinely a different instrument type.
pub fn terms_hash(t: &Terms) -> [u8; 32] {
    let mut buf = [0u8; 36];
    buf[0..4].copy_from_slice(&t.coupon_rate_bps.to_be_bytes());
    buf[4..36].copy_from_slice(&t.issuer_pubkey);
    sha256(&buf)
}

/// `payments_digest` = sha256 chain over (holder ‖ amount_be ‖ paid_at_be ‖ clawed_back) — the
/// canonical message the issuer signs to attest the payment snapshot.
pub fn payments_digest(payments: &[Payment]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for p in payments {
        let mut h = Sha256::new();
        h.update(acc);
        h.update(p.holder);
        h.update(p.amount.to_be_bytes());
        h.update(p.paid_at.to_be_bytes());
        h.update([p.clawed_back as u8]);
        acc = h.finalize().into();
    }
    acc
}

fn verify_attestation(pubkey: &[u8; 32], msg: &[u8; 32], sig: &[u8]) -> bool {
    let sig: [u8; 64] = match sig.try_into() {
        Ok(s) => s,
        Err(_) => return false, // not a 64-byte signature
    };
    match VerifyingKey::from_bytes(pubkey) {
        Ok(vk) => vk.verify(msg, &Signature::from_bytes(&sig)).is_ok(),
        Err(_) => false,
    }
}

/// Run determination + payout. Identical to credit_v1 EXCEPT for one added gate: the payment
/// snapshot must be signed by the committed issuer key (G1).
pub fn settle(inputs: &Inputs) -> Result<(Vec<Allocation>, Journal), SettleError> {
    // 0. bind every payout-determining input to the committed instrument config (M1/M2).
    let ch = config_hash(&inputs.config);
    if derive_instrument_id(&inputs.type_id_xdr, inputs.rules_version, &ch) != inputs.instrument_id {
        return Err(SettleError::InstrumentMismatch);
    }
    if snapshot_root(&inputs.snapshot) != inputs.config.snapshot_root {
        return Err(SettleError::SnapshotMismatch);
    }
    if terms_hash(&inputs.terms) != inputs.config.terms_hash {
        return Err(SettleError::TermsMismatch);
    }

    // 0b. G1: the payments must be ATTESTED by the committed issuer key. This is the only
    // structural difference from credit_v1 — the data is no longer merely "supplied".
    let digest = payments_digest(&inputs.payments);
    if !verify_attestation(&inputs.terms.issuer_pubkey, &digest, &inputs.attestation) {
        return Err(SettleError::AttestationInvalid);
    }

    let committed_deadline = inputs
        .config
        .epoch_deadlines
        .iter()
        .find(|(e, _)| *e == inputs.epoch)
        .map(|(_, d)| *d)
        .ok_or(SettleError::DeadlineMismatch)?;
    if inputs.deadline != committed_deadline {
        return Err(SettleError::DeadlineMismatch);
    }
    let pos_root = position_root(&inputs.positions);
    if pos_root != inputs.position_root {
        return Err(SettleError::PositionMismatch);
    }
    let rate = inputs.terms.coupon_rate_bps as i128;

    // 1. owed / paid / shortfall per holder (identical to credit_v1)
    let mut sum_owed: i128 = 0;
    let mut sum_short: i128 = 0;
    for h in &inputs.snapshot {
        if h.balance < 0 {
            return Err(SettleError::BadInput);
        }
        if !h.has_trustline {
            continue;
        }
        let owed = h.balance.checked_mul(rate).ok_or(SettleError::Overflow)? / BPS_DENOM;
        sum_owed = sum_owed.checked_add(owed).ok_or(SettleError::Overflow)?;
        let mut paid: i128 = 0;
        if !h.frozen {
            for p in &inputs.payments {
                if p.holder == h.id && p.paid_at <= inputs.deadline && !p.clawed_back {
                    if p.amount < 0 {
                        return Err(SettleError::BadInput);
                    }
                    paid = paid.checked_add(p.amount).ok_or(SettleError::Overflow)?;
                }
            }
        }
        let short = if owed > paid { owed - paid } else { 0 };
        sum_short = sum_short.checked_add(short).ok_or(SettleError::Overflow)?;
    }

    // 2. trigger: missed iff Σ shortfall > 0, else unprovable
    if sum_short == 0 {
        return Err(SettleError::NoDefault);
    }

    // 3. pro-rata payout per buyer, capped at cover (identical to credit_v1)
    let mut allocs = Vec::new();
    let mut total: i128 = 0;
    for p in &inputs.positions {
        if p.cover <= 0 {
            return Err(SettleError::BadInput);
        }
        let raw = p.cover.checked_mul(sum_short).ok_or(SettleError::Overflow)? / sum_owed;
        let payout = if raw > p.cover { p.cover } else { raw };
        if payout > 0 {
            allocs.push(Allocation { buyer: p.buyer.clone(), amount: payout });
            total = total.checked_add(payout).ok_or(SettleError::Overflow)?;
        }
    }

    // 4. Σ payouts ≤ collateral (defensive; the binding check is on-chain)
    if total > inputs.collateral {
        return Err(SettleError::Insolvent);
    }

    let journal = Journal {
        instrument_id: inputs.instrument_id,
        epoch: inputs.epoch,
        deadline: inputs.deadline,
        position_root: pos_root,
        allocation_root: allocation_root(&allocs),
        total_payout: u64::try_from(total).map_err(|_| SettleError::Overflow)?,
    };
    Ok((allocs, journal))
}

#[cfg(test)]
mod test;
