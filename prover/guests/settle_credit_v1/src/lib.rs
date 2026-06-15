//! `settle_credit_v1` — the determination + payout LOGIC for instrument #1 (TECH_SPEC §4).
//!
//! Pure Rust so it is unit-testable natively AND runs unchanged inside the RISC Zero guest.
//!
//! SOUNDNESS BINDING (review M1/M2): every payout-determining input is bound to the
//! instrument's committed config. `settle()` re-derives `config_hash` (flat encoding,
//! byte-identical to the factory) → `instrument_id`, and asserts it equals the public
//! `instrument_id` the proof is bound to. It then checks the supplied snapshot hashes to
//! the *committed* `snapshot_root`, the coupon rate opens the *committed* `terms_hash`, and
//! the epoch's deadline matches the *committed* schedule. A prover therefore cannot fabricate
//! a snapshot or rate: doing so changes `instrument_id` and the proof binds to the wrong
//! instrument. (This is distinct from the documented input-canonicity gap, which is about
//! whether the *payment history* is complete — mitigated by permissionless keeping.)
//!
//! ENCODING BRIDGE (review M3): the two `Address` config fields and every payee are carried
//! as host-provided canonical Address-XDR bytes. `config_hash`/`allocation_root` fold those
//! bytes exactly as the contracts do (`addr.to_xdr`), so the guest's roots match on-chain
//! recomputation byte-for-byte. The host cannot supply false XDR — it would change
//! `instrument_id` (config) or the committed `position_root` (payees).
//!
//! Published rules: owed = balance × coupon_rate(bps); no-trustline EXCLUDED from Σ owed;
//! frozen = full shortfall; clawback nets out; boundary `paid_at ≤ deadline`; missed iff
//! Σ shortfall > 0 else unprovable; payout = cover × (Σ shortfall / Σ owed) capped at cover.

use ark_ff::{BigInteger, PrimeField};
use sha2::{Digest, Sha256};
use zkhash::fields::bn256::FpBN256 as Fp;
use zkhash::poseidon2::poseidon2::Poseidon2;
use zkhash::poseidon2::poseidon2_instance_bn256::POSEIDON2_BN256_PARAMS;

pub const BPS_DENOM: i128 = 10_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Holder {
    pub id: [u8; 32],
    pub balance: i128,
    pub has_trustline: bool,
    pub frozen: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Payment {
    pub holder: [u8; 32],
    pub amount: i128,
    pub paid_at: u64,
    pub clawed_back: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Position {
    /// Canonical Address XDR of the buyer (host-provided; bound via the position commitment).
    pub buyer: Vec<u8>,
    pub cover: i128,
    pub salt: [u8; 32],
}

/// The bond's coupon terms — the preimage of the committed `terms_hash`. Opening it binds
/// the coupon rate to what the instrument committed at deploy.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Terms {
    pub coupon_rate_bps: u32,
}

/// The flat, guest-reproducible image of the on-chain `InstrumentConfig` (see factory
/// `hash_config`). The two Address fields are canonical XDR bytes (host-provided).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConfigFields {
    pub reference_asset_xdr: Vec<u8>,
    pub terms_hash: [u8; 32],
    pub schedule_root: [u8; 32],
    pub snapshot_root: [u8; 32],
    pub collateral_token_xdr: Vec<u8>,
    pub premium_bps: u32,
    pub epoch_deadlines: Vec<(u32, u64)>,
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
    pub positions: Vec<Position>,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Allocation {
    /// Canonical Address XDR of the payee (matches what the contract folds + pays).
    pub buyer: Vec<u8>,
    pub amount: i128,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Journal {
    pub instrument_id: [u8; 32],
    pub epoch: u32,
    pub deadline: u64,
    pub position_root: [u8; 32],
    pub allocation_root: [u8; 32],
    pub total_payout: u64,
}

impl Journal {
    /// Serialize to the generic journal's fixed 116-byte big-endian layout (TECH_SPEC §3.4):
    /// instrument_id(32) ‖ epoch(4) ‖ deadline(8) ‖ position_root(32) ‖ allocation_root(32) ‖ total_payout(8).
    pub fn to_bytes(&self) -> [u8; 116] {
        let mut b = [0u8; 116];
        b[0..32].copy_from_slice(&self.instrument_id);
        b[32..36].copy_from_slice(&self.epoch.to_be_bytes());
        b[36..44].copy_from_slice(&self.deadline.to_be_bytes());
        b[44..76].copy_from_slice(&self.position_root);
        b[76..108].copy_from_slice(&self.allocation_root);
        b[108..116].copy_from_slice(&self.total_payout.to_be_bytes());
        b
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SettleError {
    /// Σ shortfall == 0: the trigger did not occur — no proof can exist (guest panics).
    NoDefault,
    /// Re-derived instrument_id != the public one — fabricated/mismatched config (M1/M2).
    InstrumentMismatch,
    /// Supplied snapshot doesn't hash to the committed snapshot_root.
    SnapshotMismatch,
    /// Supplied terms don't open the committed terms_hash (rate not bound).
    TermsMismatch,
    /// Epoch absent from the committed schedule, or deadline != the committed one.
    DeadlineMismatch,
    /// Supplied positions don't hash to the committed position_root.
    PositionMismatch,
    Insolvent,
    Overflow,
    /// A committed input is out of its valid domain (negative balance/amount/cover).
    BadInput,
}

// ---------- hashing (all sha256 except the Poseidon2 commitment) ----------

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn sha256_fold(acc: &[u8; 32], leaf: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(acc);
    h.update(leaf);
    h.finalize().into()
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

/// Poseidon2 commitment `H(buyer ‖ cover ‖ salt)` over BN254 (parity-settled params).
/// Buyer is the canonical Address XDR; computed by buyers off-chain, reproduced when opening.
pub fn commitment(buyer: &[u8], cover: i128, salt: &[u8; 32]) -> [u8; 32] {
    let perm = Poseidon2::new(&POSEIDON2_BN256_PARAMS);
    let state = [fp_from_bytes(buyer), fp_from_bytes(&cover.to_be_bytes()), fp_from_bytes(salt)];
    fp_to_bytes32(&perm.permutation(&state)[0])
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

/// `allocation_root` = sha256 chain over (buyer_xdr ‖ amount_be). Byte-identical to the
/// settlement contract's `hash_allocations` (which folds `addr.to_xdr ‖ amount_be`).
pub fn allocation_root(allocs: &[Allocation]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for a in allocs {
        let mut leaf: Vec<u8> = Vec::with_capacity(a.buyer.len() + 16);
        leaf.extend_from_slice(&a.buyer);
        leaf.extend_from_slice(&a.amount.to_be_bytes());
        acc = sha256_fold(&acc, &leaf);
    }
    acc
}

/// `config_hash` — flat canonical encoding, byte-identical to factory `hash_config`.
pub fn config_hash(c: &ConfigFields) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&c.reference_asset_xdr);
    buf.extend_from_slice(&c.terms_hash);
    buf.extend_from_slice(&c.schedule_root);
    buf.extend_from_slice(&c.snapshot_root);
    buf.extend_from_slice(&c.collateral_token_xdr);
    buf.extend_from_slice(&c.premium_bps.to_be_bytes());
    buf.extend_from_slice(&(c.epoch_deadlines.len() as u32).to_be_bytes());
    for (epoch, deadline) in &c.epoch_deadlines {
        buf.extend_from_slice(&epoch.to_be_bytes());
        buf.extend_from_slice(&deadline.to_be_bytes());
    }
    sha256(&buf)
}

/// `instrument_id = H(type_id_xdr ‖ rules_version_be ‖ config_hash)` — matches factory `derive_instrument_id`.
pub fn derive_instrument_id(type_id_xdr: &[u8], rules_version: u32, config_hash: &[u8; 32]) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(type_id_xdr);
    buf.extend_from_slice(&rules_version.to_be_bytes());
    buf.extend_from_slice(config_hash);
    sha256(&buf)
}

/// `terms_hash` — the coupon-terms commitment the guest opens to bind the rate.
pub fn terms_hash(t: &Terms) -> [u8; 32] {
    sha256(&t.coupon_rate_bps.to_be_bytes())
}

// ---------- the published settlement rules ----------

/// Run determination + payout. The guest treats any `Err` as a panic; `NoDefault` is the
/// load-bearing one — a fully-paid epoch produces no proof.
pub fn settle(inputs: &Inputs) -> Result<(Vec<Allocation>, Journal), SettleError> {
    // 0. BIND every payout-determining input to the committed instrument config (M1/M2).
    let ch = config_hash(&inputs.config);
    let derived_id = derive_instrument_id(&inputs.type_id_xdr, inputs.rules_version, &ch);
    if derived_id != inputs.instrument_id {
        return Err(SettleError::InstrumentMismatch);
    }
    if snapshot_root(&inputs.snapshot) != inputs.config.snapshot_root {
        return Err(SettleError::SnapshotMismatch);
    }
    if terms_hash(&inputs.terms) != inputs.config.terms_hash {
        return Err(SettleError::TermsMismatch);
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

    // 1. owed / paid / shortfall per holder
    let mut sum_owed: i128 = 0;
    let mut sum_short: i128 = 0;
    for h in &inputs.snapshot {
        if h.balance < 0 {
            return Err(SettleError::BadInput); // not real Stellar state
        }
        if !h.has_trustline {
            continue; // excluded from owed — holder-side failure is not issuer default
        }
        let owed = h.balance.checked_mul(rate).ok_or(SettleError::Overflow)? / BPS_DENOM;
        sum_owed = sum_owed.checked_add(owed).ok_or(SettleError::Overflow)?;

        // frozen holder cannot receive -> paid is 0 -> full owed is shortfall
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

    // 3. pro-rata payout per buyer, capped at cover (integer floor; remainder favours sellers)
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

    // 4. Σ payouts ≤ collateral. Defensive only — the BINDING solvency check is on-chain
    // (vault.pay_allocations vs the live balance); `inputs.collateral` is a free input here.
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
#[cfg(test)]
mod prop_test;
