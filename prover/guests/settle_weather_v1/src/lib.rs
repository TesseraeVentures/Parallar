//! `settle_weather_v1` — the determination + payout LOGIC for instrument #2: parametric
//! weather/index protection (TECH_SPEC §4, PRODUCTION_GAP G8). Pure Rust so it is
//! unit-testable natively AND runs unchanged inside the RISC Zero guest.
//!
//! THE THESIS, IN CODE. This is a *separate* guest type with its own `image_id` — never an
//! in-place edit of `settle_credit_v1` (the versioning law). Yet it reproduces the generic
//! primitives — the Poseidon2 position commitment, the sha256 `position_root` /
//! `allocation_root` accumulators, the flat `config_hash`, the 116-byte journal — BYTE-FOR-BYTE
//! identical to credit_v1. So the SAME generic vault WASM, the SAME settlement WASM, and the
//! SAME factory/registry deploy and settle a weather instrument with zero contract changes.
//! The only new code is the published rule below. That is what "a family over one provable
//! core" means, made concrete rather than asserted.
//!
//! SOUNDNESS BINDING (same discipline as credit_v1, review M1/M2): every payout-determining
//! input is bound to the instrument's committed config. `settle()` re-derives `config_hash`
//! → `instrument_id` and asserts it equals the public one; the supplied parameters hash to the
//! committed `snapshot_root`; the threshold terms open the committed `terms_hash`; the epoch's
//! deadline matches the committed schedule; the positions hash to the committed `position_root`.
//! A prover cannot fabricate the threshold, the station/window, or a payee.
//!
//! Published rule (weather_v1): a rainfall-shortfall (parametric drought) cover. The instrument
//! commits a `trigger_mm` (the rainfall at/above which there is no payout) and an `exhaust_mm`
//! (the rainfall at/below which the payout is the full cover), with `trigger_mm > exhaust_mm`.
//! Over the committed station + window, `observed_mm` is the sum of attested in-window readings
//! received by the deadline. Severity is the linear shortfall fraction:
//!   observed ≥ trigger          → NO breach → unprovable (the guest panics, like a fully-paid
//!                                 coupon: parametric cover that did not trigger cannot be claimed);
//!   exhaust < observed < trigger → payout = cover × (trigger − observed) / (trigger − exhaust);
//!   observed ≤ exhaust          → payout = cover (capped).
//! Σ payouts ≤ collateral.

use ark_ff::{BigInteger, PrimeField};
use sha2::{Digest, Sha256};
use zkhash::fields::bn256::FpBN256 as Fp;
use zkhash::poseidon2::poseidon2::Poseidon2;
use zkhash::poseidon2::poseidon2_instance_bn256::POSEIDON2_BN256_PARAMS;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Position {
    /// Canonical Address XDR of the buyer (host-provided; bound via the position commitment).
    pub buyer: Vec<u8>,
    pub cover: i128,
    pub salt: [u8; 32],
}

/// One attested observation (e.g. a daily rainfall reading). The G1 input-canonicity gap
/// applies exactly as it does to credit_v1's payments: production accepts only attested
/// readings; the MVP keeper supplies them. Readings are the witness, NOT committed in config.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Observation {
    pub station: [u8; 32],
    pub mm: u32,
    pub observed_at: u64,
}

/// The published threshold terms — the preimage of the committed `terms_hash`. Opening it binds
/// the trigger/exhaust band to what the instrument committed at deploy.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Terms {
    pub trigger_mm: u32,
    pub exhaust_mm: u32,
}

/// The station + window this instrument settles over — the preimage of the committed
/// `snapshot_root` (the weather analog of credit's holder snapshot: the fixed reference set the
/// determination runs against, committed at issuance).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WeatherParams {
    pub station_id: [u8; 32],
    pub window_start: u64,
    pub window_end: u64,
}

/// The flat, guest-reproducible image of the on-chain `InstrumentConfig` (factory
/// `hash_config`) — IDENTICAL layout to credit_v1, because the registry/factory surface is
/// frozen (Law #2). The two Address fields are canonical XDR bytes (host-provided).
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
    pub params: WeatherParams,
    pub collateral: i128,
    // --- private witness ---
    pub observations: Vec<Observation>,
    pub positions: Vec<Position>,
    pub position_root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Allocation {
    /// Canonical Address XDR of the payee (matches what the contract folds + pays).
    pub buyer: Vec<u8>,
    pub amount: i128,
}

/// The GENERIC journal — byte-identical struct + layout to credit_v1's, so the same generic
/// settlement contract reads it (TECH_SPEC §3.4).
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
    /// The generic journal's fixed 116-byte big-endian layout (TECH_SPEC §3.4):
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
    /// observed_mm ≥ trigger_mm: the index did not breach — no proof can exist (guest panics).
    /// The weather analog of credit's NoDefault: parametric cover that did not trigger.
    NoBreach,
    /// Re-derived instrument_id != the public one — fabricated/mismatched config (M1/M2).
    InstrumentMismatch,
    /// Supplied threshold terms don't open the committed terms_hash (band not bound).
    TermsMismatch,
    /// Supplied station/window params don't hash to the committed snapshot_root.
    ParamsMismatch,
    /// Epoch absent from the committed schedule, or deadline != the committed one.
    DeadlineMismatch,
    /// Supplied positions don't hash to the committed position_root.
    PositionMismatch,
    Insolvent,
    Overflow,
    /// A committed input is out of its valid domain (e.g. trigger ≤ exhaust, negative cover).
    BadInput,
}

// ---------- hashing — IDENTICAL to settle_credit_v1 (same on-chain compatibility) ----------

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

/// Poseidon2 commitment `H(buyer ‖ cover ‖ salt)` over BN254 — IDENTICAL to credit_v1, so the
/// same generic vault folds weather positions into the same `position_root`.
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

/// `allocation_root` = sha256 chain over (buyer_xdr ‖ amount_be) — byte-identical to the
/// settlement contract's `hash_allocations`.
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

/// `instrument_id = H(type_id_xdr ‖ rules_version_be ‖ config_hash)` — matches factory.
pub fn derive_instrument_id(type_id_xdr: &[u8], rules_version: u32, config_hash: &[u8; 32]) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(type_id_xdr);
    buf.extend_from_slice(&rules_version.to_be_bytes());
    buf.extend_from_slice(config_hash);
    sha256(&buf)
}

// ---------- weather-specific commitments ----------

/// `terms_hash` — the threshold-band commitment the guest opens to bind the trigger/exhaust.
pub fn terms_hash(t: &Terms) -> [u8; 32] {
    let mut buf = [0u8; 8];
    buf[0..4].copy_from_slice(&t.trigger_mm.to_be_bytes());
    buf[4..8].copy_from_slice(&t.exhaust_mm.to_be_bytes());
    sha256(&buf)
}

/// `snapshot_root` (weather) = sha256 over (station_id ‖ window_start_be ‖ window_end_be). The
/// committed reference set the determination runs against — the weather analog of credit's
/// holder snapshot. Bound via `config.snapshot_root`.
pub fn snapshot_root(p: &WeatherParams) -> [u8; 32] {
    let mut leaf = [0u8; 48];
    leaf[0..32].copy_from_slice(&p.station_id);
    leaf[32..40].copy_from_slice(&p.window_start.to_be_bytes());
    leaf[40..48].copy_from_slice(&p.window_end.to_be_bytes());
    sha256(&leaf)
}

// ---------- the published settlement rules ----------

/// Run determination + payout. The guest treats any `Err` as a panic; `NoBreach` is the
/// load-bearing one — an index that did not breach produces no proof.
pub fn settle(inputs: &Inputs) -> Result<(Vec<Allocation>, Journal), SettleError> {
    // 0. BIND every payout-determining input to the committed instrument config (M1/M2).
    let ch = config_hash(&inputs.config);
    let derived_id = derive_instrument_id(&inputs.type_id_xdr, inputs.rules_version, &ch);
    if derived_id != inputs.instrument_id {
        return Err(SettleError::InstrumentMismatch);
    }
    if terms_hash(&inputs.terms) != inputs.config.terms_hash {
        return Err(SettleError::TermsMismatch);
    }
    if snapshot_root(&inputs.params) != inputs.config.snapshot_root {
        return Err(SettleError::ParamsMismatch);
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

    // published band: trigger strictly above exhaust (a positive severity span).
    let trigger = inputs.terms.trigger_mm as i128;
    let exhaust = inputs.terms.exhaust_mm as i128;
    if trigger <= exhaust {
        return Err(SettleError::BadInput);
    }
    let span = trigger - exhaust;

    // 1. observed_mm = Σ attested in-window readings for the committed station, by the deadline.
    let mut observed: i128 = 0;
    for o in &inputs.observations {
        if o.station == inputs.params.station_id
            && o.observed_at >= inputs.params.window_start
            && o.observed_at <= inputs.params.window_end
            && o.observed_at <= inputs.deadline
        {
            observed = observed.checked_add(o.mm as i128).ok_or(SettleError::Overflow)?;
        }
    }

    // 2. trigger: breach iff observed < trigger (rainfall fell short), else unprovable.
    if observed >= trigger {
        return Err(SettleError::NoBreach);
    }

    // 3. severity = min(trigger − observed, span) / span; payout = cover × severity, capped.
    let severity_num = {
        let raw = trigger - observed; // > 0 past the NoBreach check
        if raw > span {
            span
        } else {
            raw
        }
    };

    let mut allocs = Vec::new();
    let mut total: i128 = 0;
    for p in &inputs.positions {
        if p.cover <= 0 {
            return Err(SettleError::BadInput);
        }
        let payout = p.cover.checked_mul(severity_num).ok_or(SettleError::Overflow)? / span;
        // payout ≤ cover by construction (severity_num ≤ span); floor favours sellers.
        if payout > 0 {
            allocs.push(Allocation { buyer: p.buyer.clone(), amount: payout });
            total = total.checked_add(payout).ok_or(SettleError::Overflow)?;
        }
    }

    // 4. Σ payouts ≤ collateral. Defensive only — the BINDING solvency check is on-chain.
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
