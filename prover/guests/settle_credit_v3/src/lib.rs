//! `settle_credit_v3` — instance #1, ATTESTED + RECORD-DATE (PRODUCTION_GAP G4). Pure Rust:
//! testable natively AND runs unchanged inside the RISC Zero guest.
//!
//! credit_v2 (G1) closed the *payment*-canonicity gap: it verifies an issuer Ed25519 signature over
//! the payment snapshot. But it still binds the HOLDER snapshot to a value fixed at issuance
//! (`snapshot_root == config.snapshot_root`) — which is wrong for a TRADED bond, where whoever holds
//! on each epoch's record date is a DIFFERENT set. credit_v3 closes that: the issuer attests the
//! PER-EPOCH holder snapshot together with the payments, signing
//!   `sha256(epoch_be ‖ snapshot_digest(snapshot) ‖ payments_digest(payments))`.
//! So the settlement pays over the record-date holder set the issuer signed for THIS epoch, and the
//! keeper can no more fabricate the holder set than it can fabricate payments. The per-epoch
//! snapshot is NOT checked against `config.snapshot_root` (it is not fixed); `config.snapshot_root`
//! still binds whatever the deployer committed (e.g. a record-date schedule), keeping config_hash
//! meaningful and the generic surfaces unchanged.
//!
//! A SEPARATE type with its OWN image_id (the versioning law). credit_v1/v2 source is untouched.
//! REUSES credit_v1's exact generic primitives (Poseidon commitment, sha256 roots, flat
//! config_hash, the 116-byte journal), so the SAME vault/settlement/factory accept it unchanged.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

pub use settle_credit_v1::{
    allocation_root, commitment, config_hash, derive_instrument_id, position_root, snapshot_root,
    Allocation, ConfigFields, Holder, Journal, Payment, Position,
};

pub const BPS_DENOM: i128 = 10_000;

/// credit_v3 terms: the coupon rate AND the issuer's Ed25519 public key whose signature attests the
/// per-epoch (snapshot + payments). Committed via `terms_hash`, so the trusted key is bound at deploy.
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
    /// The PER-EPOCH (record-date) holder snapshot — attested, not fixed at issuance.
    pub snapshot: Vec<Holder>,
    pub payments: Vec<Payment>,
    /// Issuer's Ed25519 signature (64 bytes) over `record_date_msg(epoch, snapshot, payments)`.
    pub attestation: Vec<u8>,
    pub positions: Vec<Position>,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SettleError {
    NoDefault,
    InstrumentMismatch,
    TermsMismatch,
    DeadlineMismatch,
    PositionMismatch,
    Insolvent,
    Overflow,
    BadInput,
    /// The issuer signature over (epoch ‖ snapshot ‖ payments) did not verify — the record-date
    /// holder set and/or the payments are not attested (G4).
    AttestationInvalid,
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// `terms_hash` = sha256(coupon_rate_bps_be ‖ issuer_pubkey) — identical layout to credit_v2.
pub fn terms_hash(t: &Terms) -> [u8; 32] {
    let mut buf = [0u8; 36];
    buf[0..4].copy_from_slice(&t.coupon_rate_bps.to_be_bytes());
    buf[4..36].copy_from_slice(&t.issuer_pubkey);
    sha256(&buf)
}

/// `snapshot_digest` = sha256 chain over (holder_id ‖ balance_be ‖ has_trustline ‖ frozen) — the
/// canonical commitment to the record-date holder set the issuer signs.
pub fn snapshot_digest(snapshot: &[Holder]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for h in snapshot {
        let mut s = Sha256::new();
        s.update(acc);
        s.update(h.id);
        s.update(h.balance.to_be_bytes());
        s.update([h.has_trustline as u8]);
        s.update([h.frozen as u8]);
        acc = s.finalize().into();
    }
    acc
}

/// `payments_digest` = sha256 chain over (holder ‖ amount_be ‖ paid_at_be ‖ clawed_back) — identical
/// layout to credit_v2.
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

/// The record-date attestation message: `sha256(epoch_be ‖ snapshot_digest ‖ payments_digest)`.
/// One signature binds the holder set AND the payments to THIS epoch — no replay across epochs, no
/// holder-set or payment fabrication.
pub fn record_date_msg(epoch: u32, snapshot: &[Holder], payments: &[Payment]) -> [u8; 32] {
    let mut buf = [0u8; 68];
    buf[0..4].copy_from_slice(&epoch.to_be_bytes());
    buf[4..36].copy_from_slice(&snapshot_digest(snapshot));
    buf[36..68].copy_from_slice(&payments_digest(payments));
    sha256(&buf)
}

fn verify_attestation(pubkey: &[u8; 32], msg: &[u8; 32], sig: &[u8]) -> bool {
    let sig: [u8; 64] = match sig.try_into() {
        Ok(s) => s,
        Err(_) => return false,
    };
    match VerifyingKey::from_bytes(pubkey) {
        Ok(vk) => vk.verify(msg, &Signature::from_bytes(&sig)).is_ok(),
        Err(_) => false,
    }
}

/// Run determination + payout. Identical to credit_v2 EXCEPT the attestation covers the per-epoch
/// (snapshot + payments) rather than only payments, and the snapshot is NOT pinned to
/// `config.snapshot_root` (it is the attested record-date set).
pub fn settle(inputs: &Inputs) -> Result<(Vec<Allocation>, Journal), SettleError> {
    // 0. bind every payout-determining input to the committed instrument config (M1/M2).
    let ch = config_hash(&inputs.config);
    if derive_instrument_id(&inputs.type_id_xdr, inputs.rules_version, &ch) != inputs.instrument_id {
        return Err(SettleError::InstrumentMismatch);
    }
    if terms_hash(&inputs.terms) != inputs.config.terms_hash {
        return Err(SettleError::TermsMismatch);
    }

    // 0b. G4: the PER-EPOCH holder snapshot AND the payments must be attested by the committed
    // issuer key for THIS epoch. (No fixed snapshot_root check — the record-date set is dynamic.)
    let msg = record_date_msg(inputs.epoch, &inputs.snapshot, &inputs.payments);
    if !verify_attestation(&inputs.terms.issuer_pubkey, &msg, &inputs.attestation) {
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

    // 1. owed / paid / shortfall per holder (identical to credit_v1/v2)
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

    // 3. pro-rata payout per buyer, capped at cover
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
