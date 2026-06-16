//! `solvency_v1` — the Option C purchase-time solvency proof (PRODUCTION_GAP G3). Pure Rust;
//! testable natively and runs unchanged in the RISC Zero guest.
//!
//! Option B (shipped) keeps `total_cover` as a PUBLIC aggregate and reveals each purchase's cover
//! in the buy transaction. Option C removes that leak: the vault keeps the running total as a
//! Poseidon COMMITMENT, and each purchase carries a proof that
//!   • the stored commitment opens to some hidden `old_total`,
//!   • `new_total = old_total + cover` (cover hidden, > 0),
//!   • `new_total ≤ collateral` (solvency preserved), and
//!   • the SAME hidden cover is the one bound in the buyer's position commitment
//! — revealing neither the cover nor the totals. The vault verifies the proof, checks the prev
//! commitment matches what it stored, and advances to the new commitment. Per-purchase cover never
//! appears in plaintext on-chain.
//!
//! Coordination (operational): the next buyer needs the current aggregate opening (old_total,
//! old_salt) to build the proof — supplied by the keeper/sequencer that orders purchases. That is
//! the production coordination model (a confidential running aggregate), documented under G3.

use ark_ff::{BigInteger, PrimeField};
use zkhash::fields::bn256::FpBN256 as Fp;
use zkhash::poseidon2::poseidon2::Poseidon2;
use zkhash::poseidon2::poseidon2_instance_bn256::POSEIDON2_BN256_PARAMS;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SolvencyInputs {
    // --- public ---
    pub collateral: i128,                  // the vault's current total_collateral
    pub prev_cover_commitment: [u8; 32],   // the vault's stored running-cover commitment
    pub new_cover_commitment: [u8; 32],    // the post-purchase running-cover commitment
    pub position_commitment: [u8; 32],     // the buyer's position commitment (vault folds it)
    // --- private witness ---
    pub old_total: i128,
    pub old_salt: [u8; 32],
    pub new_salt: [u8; 32],
    pub cover: i128,
    pub buyer: Vec<u8>,
    pub salt: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SolvencyJournal {
    pub prev_cover_commitment: [u8; 32],
    pub new_cover_commitment: [u8; 32],
    pub position_commitment: [u8; 32],
    pub collateral: i128,
}

impl SolvencyJournal {
    /// 112-byte layout: prev(32) ‖ new(32) ‖ position(32) ‖ collateral(16 BE). The vault verifies
    /// the proof against this journal's digest, checks `prev` == its stored commitment, then sets
    /// the stored commitment to `new` and folds `position` into position_root.
    pub fn to_bytes(&self) -> [u8; 112] {
        let mut b = [0u8; 112];
        b[0..32].copy_from_slice(&self.prev_cover_commitment);
        b[32..64].copy_from_slice(&self.new_cover_commitment);
        b[64..96].copy_from_slice(&self.position_commitment);
        b[96..112].copy_from_slice(&self.collateral.to_be_bytes());
        b
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SolvencyError {
    /// new_total would exceed collateral — the purchase is rejected (no proof can exist).
    Insolvent,
    /// the stored commitment does not open to the supplied old_total/old_salt.
    PrevMismatch,
    /// the new commitment does not open to new_total/new_salt.
    NewMismatch,
    /// the position commitment is not H(buyer ‖ cover ‖ salt) for the same hidden cover.
    PositionMismatch,
    BadInput,
    Overflow,
}

fn fp_from_bytes(b: &[u8]) -> Fp {
    Fp::from_be_bytes_mod_order(b)
}
fn fp_to_bytes32(f: &Fp) -> [u8; 32] {
    let be = f.into_bigint().to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - be.len()..].copy_from_slice(&be);
    out
}

/// Poseidon commitment to the running aggregate cover: `H(total ‖ salt)` over BN254 — the same
/// parity-settled Poseidon2 the position commitment uses, so both live in the same field.
pub fn commit_total(total: i128, salt: &[u8; 32]) -> [u8; 32] {
    let perm = Poseidon2::new(&POSEIDON2_BN256_PARAMS);
    let state = [fp_from_bytes(&total.to_be_bytes()), fp_from_bytes(salt), fp_from_bytes(&[0u8; 32])];
    fp_to_bytes32(&perm.permutation(&state)[0])
}

/// Verify a purchase preserves solvency, hiding the cover and the totals. Any `Err` is a guest
/// panic (no proof). On success the journal binds the prev/new commitments the vault advances.
pub fn check(inp: &SolvencyInputs) -> Result<SolvencyJournal, SolvencyError> {
    if inp.cover <= 0 || inp.old_total < 0 || inp.collateral < 0 {
        return Err(SolvencyError::BadInput);
    }
    // the stored commitment opens to old_total
    if commit_total(inp.old_total, &inp.old_salt) != inp.prev_cover_commitment {
        return Err(SolvencyError::PrevMismatch);
    }
    let new_total = inp.old_total.checked_add(inp.cover).ok_or(SolvencyError::Overflow)?;
    // SOLVENCY: the running total never exceeds the reserve — the whole point of the proof.
    if new_total > inp.collateral {
        return Err(SolvencyError::Insolvent);
    }
    // the advanced commitment opens to new_total
    if commit_total(new_total, &inp.new_salt) != inp.new_cover_commitment {
        return Err(SolvencyError::NewMismatch);
    }
    // the SAME hidden cover is the one bound in the buyer's position commitment (no cover swap)
    if settle_credit_v1::commitment(&inp.buyer, inp.cover, &inp.salt) != inp.position_commitment {
        return Err(SolvencyError::PositionMismatch);
    }
    Ok(SolvencyJournal {
        prev_cover_commitment: inp.prev_cover_commitment,
        new_cover_commitment: inp.new_cover_commitment,
        position_commitment: inp.position_commitment,
        collateral: inp.collateral,
    })
}

// ─────────────────────────── withdrawal solvency (the symmetric check) ───────────────────────────
//
// A purchase grows the hidden aggregate (proven ≤ collateral). A withdrawal shrinks the reserve, so
// it must prove the SAME hidden aggregate still fits under the post-withdrawal collateral — without
// revealing it. The vault's withdraw entrypoint verifies this, checks the commitment matches what it
// stored, and that collateral_after == total_collateral − amount.

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WithdrawInputs {
    // --- public ---
    pub collateral_after: i128,     // total_collateral − the withdrawal amount
    pub cover_commitment: [u8; 32], // the vault's stored running-cover commitment
    // --- private witness ---
    pub total: i128,    // the hidden running aggregate the commitment opens to
    pub salt: [u8; 32], // its opening salt
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WithdrawJournal {
    pub cover_commitment: [u8; 32],
    pub collateral_after: i128,
}

impl WithdrawJournal {
    /// 48-byte layout: cover_commitment(32) ‖ collateral_after(16 BE). Length-distinct from the
    /// 112-byte purchase journal, so the vault's withdraw entrypoint can never accept a buy proof.
    pub fn to_bytes(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        b[0..32].copy_from_slice(&self.cover_commitment);
        b[32..48].copy_from_slice(&self.collateral_after.to_be_bytes());
        b
    }
}

/// Verify a withdrawal preserves solvency: the stored aggregate (hidden) still fits under the
/// post-withdrawal collateral. Reveals neither the aggregate nor the salt. Any `Err` is a guest
/// panic (no proof can exist), so the vault cannot release collateral that breaks the invariant.
pub fn check_withdraw(inp: &WithdrawInputs) -> Result<WithdrawJournal, SolvencyError> {
    if inp.total < 0 || inp.collateral_after < 0 {
        return Err(SolvencyError::BadInput);
    }
    // the stored commitment opens to the claimed aggregate
    if commit_total(inp.total, &inp.salt) != inp.cover_commitment {
        return Err(SolvencyError::PrevMismatch);
    }
    // SOLVENCY after the withdrawal: the reserve still covers the whole outstanding book
    if inp.total > inp.collateral_after {
        return Err(SolvencyError::Insolvent);
    }
    Ok(WithdrawJournal { cover_commitment: inp.cover_commitment, collateral_after: inp.collateral_after })
}

/// The guest entry: a confidential purchase or a confidential withdrawal. The zkVM wrapper reads
/// this, runs the matching check, and commits the journal (112 bytes for buy, 48 for withdraw — the
/// vault decodes by length, so the two proofs cannot be confused).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum SolvencyRequest {
    Buy(SolvencyInputs),
    Withdraw(WithdrawInputs),
}

#[cfg(test)]
mod test;
