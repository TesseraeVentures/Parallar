#![cfg(test)]
use super::*;

// A valid rainfall-shortfall settlement: station 0x09, window [100,200], deadline 200.
// Band trigger=500 / exhaust=100 (span 400). `observed_mm` is set by the caller via the
// readings; one buyer holds 800 of cover against 1000 collateral.
fn inputs_with_observed(observed_total: u32) -> Inputs {
    let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 200 };
    let terms = Terms { trigger_mm: 500, exhaust_mm: 100 };
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&params),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 200u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    // a single in-window reading carrying the whole observed total (kept ≤ u32)
    let observations = vec![Observation { station: [9; 32], mm: observed_total, observed_at: 150 }];
    Inputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch: 1,
        deadline: 200,
        terms,
        params,
        collateral: 1000,
        observations,
        positions,
        position_root: position_root(&[Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }]),
    }
}

#[test]
fn breach_pays_pro_rata() {
    // observed 300, trigger 500, span 400 -> severity 200/400 = 0.5 -> payout 800*0.5 = 400
    let inputs = inputs_with_observed(300);
    let (allocs, journal) = settle(&inputs).expect("a breach must settle");
    assert_eq!(allocs.len(), 1);
    assert_eq!(allocs[0].amount, 400, "pro-rata payout = cover * shortfall/span");
    assert_eq!(journal.total_payout, 400);
    assert_eq!(journal.instrument_id, inputs.instrument_id);
    assert_eq!(journal.allocation_root, allocation_root(&allocs));
}

#[test]
fn full_payout_at_exhaustion() {
    // observed == exhaust (100): shortfall 400 == span 400 -> severity 1 -> full cover 800
    let (allocs, j) = settle(&inputs_with_observed(100)).expect("settles");
    assert_eq!(allocs[0].amount, 800, "exhaustion pays the full cover");
    assert_eq!(j.total_payout, 800);
}

#[test]
fn below_exhaustion_is_capped_at_cover() {
    // observed 40 < exhaust 100: raw severity > span -> capped -> still full cover, never more
    let (allocs, _) = settle(&inputs_with_observed(40)).expect("settles");
    assert_eq!(allocs[0].amount, 800, "payout is capped at cover");
}

#[test]
fn no_breach_is_unprovable() {
    // observed == trigger (500): no shortfall -> NoBreach (the guest would panic: no proof)
    assert_eq!(settle(&inputs_with_observed(500)), Err(SettleError::NoBreach));
    // and comfortably above trigger
    assert_eq!(settle(&inputs_with_observed(900)), Err(SettleError::NoBreach));
}

#[test]
fn out_of_window_and_late_readings_are_excluded() {
    let mut inputs = inputs_with_observed(300); // in-window 300 -> would breach
    // add readings that must NOT count: wrong station, before window, after window, after deadline
    inputs.observations.push(Observation { station: [8; 32], mm: 1000, observed_at: 150 }); // wrong station
    inputs.observations.push(Observation { station: [9; 32], mm: 1000, observed_at: 50 }); // before window
    inputs.observations.push(Observation { station: [9; 32], mm: 1000, observed_at: 250 }); // after window/deadline
    let (allocs, _) = settle(&inputs).expect("settles on the in-window 300 only");
    assert_eq!(allocs[0].amount, 400, "only the in-window reading counts");
}

#[test]
fn instrument_mismatch_rejected() {
    let mut inputs = inputs_with_observed(300);
    inputs.instrument_id = [0xFF; 32];
    assert_eq!(settle(&inputs), Err(SettleError::InstrumentMismatch));
}

#[test]
fn tampered_terms_rejected() {
    let mut inputs = inputs_with_observed(300);
    inputs.terms.trigger_mm = 9999; // no longer opens the committed terms_hash
    assert_eq!(settle(&inputs), Err(SettleError::TermsMismatch));
}

#[test]
fn tampered_params_rejected() {
    let mut inputs = inputs_with_observed(300);
    inputs.params.station_id = [0xEE; 32]; // no longer opens the committed snapshot_root
    assert_eq!(settle(&inputs), Err(SettleError::ParamsMismatch));
}

#[test]
fn wrong_deadline_rejected() {
    let mut inputs = inputs_with_observed(300);
    inputs.deadline = 201; // != the committed epoch-1 deadline (200)
    assert_eq!(settle(&inputs), Err(SettleError::DeadlineMismatch));
}

#[test]
fn tampered_position_root_rejected() {
    let mut inputs = inputs_with_observed(300);
    inputs.position_root = [0x01; 32];
    assert_eq!(settle(&inputs), Err(SettleError::PositionMismatch));
}

#[test]
fn inverted_band_rejected() {
    let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 200 };
    let terms = Terms { trigger_mm: 100, exhaust_mm: 500 }; // trigger ≤ exhaust: invalid band
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&params),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 200u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    let inputs = Inputs {
        type_id_xdr, rules_version: 1, config, instrument_id, epoch: 1, deadline: 200,
        terms, params, collateral: 1000,
        observations: vec![Observation { station: [9; 32], mm: 50, observed_at: 150 }],
        positions: positions.clone(),
        position_root: position_root(&positions),
    };
    assert_eq!(settle(&inputs), Err(SettleError::BadInput));
}

#[test]
fn journal_is_116_bytes_generic_layout() {
    let (_, j) = settle(&inputs_with_observed(300)).unwrap();
    let b = j.to_bytes();
    assert_eq!(b.len(), 116);
    assert_eq!(&b[0..32], &j.instrument_id);
    assert_eq!(&b[32..36], &j.epoch.to_be_bytes());
    assert_eq!(&b[108..116], &j.total_payout.to_be_bytes());
}

// The cross-guest parity tests (weather_v1's generic primitives are byte-identical to credit_v1)
// live in `prover/proptests/tests/parity.rs` — kept out of this crate so the guest carries ZERO
// dev-dependencies and its ELF / image_id stays reproducible (R35).
