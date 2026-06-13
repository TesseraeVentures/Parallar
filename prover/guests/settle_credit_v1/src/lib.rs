//! `settle_credit_v1` — the determination + payout LOGIC for instrument #1 (TECH_SPEC §4).
//!
//! Pure Rust so it is unit-testable natively AND runs unchanged inside the RISC Zero guest
//! (the guest's `main` reads inputs, calls [`settle`], and commits the returned [`Journal`]).
//!
//! Published, normative rules (the asymmetry matters):
//! - owed_h = balance_h × coupon_rate (bps). A holder with **no trustline is EXCLUDED** from
//!   Σ owed (the issuer cannot pay them — holder-side failure is not issuer default).
//! - A **frozen** holder counts as full **shortfall** (issuer revoked their trustline = default).
//! - A coupon **clawed back** within the window does not qualify (payments count net of clawback).
//! - Boundary: `paid_at ≤ deadline` qualifies.
//! - **missed iff Σ shortfall > 0, else panic** — a fully-paid epoch is unprovable (no proof
//!   can exist for a false claim).
//! - payout_b = cover_b × (Σ shortfall / Σ owed), **capped at cover_b** (integer floor; the
//!   remainder rounds in the sellers' favour). Σ payouts ≤ collateral.

use ark_ff::{BigInteger, PrimeField};
use sha2::{Digest, Sha256};
use zkhash::fields::bn256::FpBN256 as Fp;
use zkhash::poseidon2::poseidon2::Poseidon2;
use zkhash::poseidon2::poseidon2_instance_bn256::POSEIDON2_BN256_PARAMS;

pub const BPS_DENOM: i128 = 10_000;

#[derive(Clone, Debug)]
pub struct Holder {
    pub id: [u8; 32],
    pub balance: i128,
    pub has_trustline: bool,
    pub frozen: bool,
}

#[derive(Clone, Debug)]
pub struct Payment {
    pub holder: [u8; 32],
    pub amount: i128,
    pub paid_at: u64,
    pub clawed_back: bool,
}

#[derive(Clone, Debug)]
pub struct Position {
    pub buyer: [u8; 32],
    pub cover: i128,
    pub salt: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct Inputs {
    pub instrument_id: [u8; 32],
    pub epoch: u32,
    pub deadline: u64,
    pub coupon_rate_bps: u32,
    pub collateral: i128,
    pub snapshot: Vec<Holder>,
    pub snapshot_root: [u8; 32],
    pub payments: Vec<Payment>,
    pub positions: Vec<Position>,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub buyer: [u8; 32],
    pub amount: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub instrument_id: [u8; 32],
    pub epoch: u32,
    pub deadline: u64,
    pub position_root: [u8; 32],
    pub allocation_root: [u8; 32],
    pub total_payout: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettleError {
    /// Σ shortfall == 0: the trigger did not occur — no proof can exist (guest panics).
    NoDefault,
    SnapshotMismatch,
    PositionMismatch,
    Insolvent,
    Overflow,
}

// ---------- commitments & accumulators ----------

fn fp_from_bytes(b: &[u8]) -> Fp {
    Fp::from_be_bytes_mod_order(b)
}
fn fp_to_bytes32(f: &Fp) -> [u8; 32] {
    let be = f.into_bigint().to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - be.len()..].copy_from_slice(&be);
    out
}

/// Poseidon2 commitment `H(buyer ‖ cover ‖ salt)` over BN254 (the parity-settled params).
/// Computed by buyers off-chain and reproduced here when opening positions.
pub fn commitment(buyer: &[u8; 32], cover: i128, salt: &[u8; 32]) -> [u8; 32] {
    let perm = Poseidon2::new(&POSEIDON2_BN256_PARAMS);
    let state = [
        fp_from_bytes(buyer),
        fp_from_bytes(&cover.to_be_bytes()),
        fp_from_bytes(salt),
    ];
    fp_to_bytes32(&perm.permutation(&state)[0])
}

fn sha256_fold(acc: &[u8; 32], leaf: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(acc);
    h.update(leaf);
    h.finalize().into()
}

/// `position_root` = sha256 chain over the position commitments (matches the vault accumulator).
pub fn position_root(positions: &[Position]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for p in positions {
        acc = sha256_fold(&acc, &commitment(&p.buyer, p.cover, &p.salt));
    }
    acc
}

/// `snapshot_root` = sha256 chain over (id ‖ balance_be ‖ has_trustline ‖ frozen).
pub fn snapshot_root(holders: &[Holder]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for h in holders {
        let mut leaf = [0u8; 50];
        leaf[..32].copy_from_slice(&h.id);
        leaf[32..48].copy_from_slice(&h.balance.to_be_bytes());
        leaf[48] = h.has_trustline as u8;
        leaf[49] = h.frozen as u8;
        acc = sha256_fold(&acc, &leaf);
    }
    acc
}

/// `allocation_root` = sha256 chain over (buyer ‖ amount_be).
/// BRIDGE NOTE: the settlement contract must recompute this identically. The contract
/// currently folds `addr.to_xdr ‖ amount_be`; aligning the two encodings (raw-key vs XDR)
/// is the Sprint-2 host/contract wiring task before the real proof replaces the stub.
pub fn allocation_root(allocs: &[Allocation]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for a in allocs {
        let mut leaf = [0u8; 48];
        leaf[..32].copy_from_slice(&a.buyer);
        leaf[32..].copy_from_slice(&a.amount.to_be_bytes());
        acc = sha256_fold(&acc, &leaf);
    }
    acc
}

// ---------- the published settlement rules ----------

/// Run determination + payout. The guest treats any `Err` as a panic; `NoDefault` is the
/// load-bearing one — a fully-paid epoch produces no proof.
pub fn settle(inputs: &Inputs) -> Result<(Vec<Allocation>, Journal), SettleError> {
    // 1. verify reference commitments
    if snapshot_root(&inputs.snapshot) != inputs.snapshot_root {
        return Err(SettleError::SnapshotMismatch);
    }
    let pos_root = position_root(&inputs.positions);
    if pos_root != inputs.position_root {
        return Err(SettleError::PositionMismatch);
    }

    // 2. owed / paid / shortfall per holder
    let mut sum_owed: i128 = 0;
    let mut sum_short: i128 = 0;
    for h in &inputs.snapshot {
        if !h.has_trustline {
            continue; // excluded from owed — holder-side failure is not issuer default
        }
        let owed = h
            .balance
            .checked_mul(inputs.coupon_rate_bps as i128)
            .ok_or(SettleError::Overflow)?
            / BPS_DENOM;
        sum_owed = sum_owed.checked_add(owed).ok_or(SettleError::Overflow)?;

        // frozen holder cannot receive -> paid is 0 -> full owed is shortfall
        let paid: i128 = if h.frozen {
            0
        } else {
            inputs
                .payments
                .iter()
                .filter(|p| p.holder == h.id && p.paid_at <= inputs.deadline && !p.clawed_back)
                .map(|p| p.amount)
                .sum()
        };
        let short = if owed > paid { owed - paid } else { 0 };
        sum_short = sum_short.checked_add(short).ok_or(SettleError::Overflow)?;
    }

    // 3. trigger: missed iff Σ shortfall > 0, else unprovable
    if sum_short == 0 {
        return Err(SettleError::NoDefault);
    }
    // sum_owed > 0 here: sum_short > 0 implies some included holder was owed.

    // 4. pro-rata payout per buyer, capped at cover (integer floor)
    let mut allocs = Vec::new();
    let mut total: i128 = 0;
    for p in &inputs.positions {
        let raw = p
            .cover
            .checked_mul(sum_short)
            .ok_or(SettleError::Overflow)?
            / sum_owed;
        let payout = if raw > p.cover { p.cover } else { raw };
        if payout > 0 {
            allocs.push(Allocation { buyer: p.buyer, amount: payout });
            total = total.checked_add(payout).ok_or(SettleError::Overflow)?;
        }
    }

    // 5. Σ payouts ≤ collateral
    if total > inputs.collateral {
        return Err(SettleError::Insolvent);
    }

    let journal = Journal {
        instrument_id: inputs.instrument_id,
        epoch: inputs.epoch,
        deadline: inputs.deadline,
        position_root: pos_root,
        allocation_root: allocation_root(&allocs),
        total_payout: total as u64,
    };
    Ok((allocs, journal))
}

#[cfg(test)]
mod test;
