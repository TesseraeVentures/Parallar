//! THE LAYER THESIS, ASSERTED ACROSS CRATES: weather_v1's generic primitives (Poseidon position
//! commitment, position/allocation roots, config_hash, instrument_id derivation) are
//! BYTE-IDENTICAL to credit_v1's — so the SAME generic vault, settlement, and factory WASM accept
//! a weather instrument with zero contract changes.
//!
//! This lives here, NOT inside the weather guest, so the guest crates carry ZERO dev-dependencies
//! and their ELFs / image_ids stay minimal and reproducible (R35 — a guest dev-dep once shifted
//! credit_v1's deployed image_id).

use settle_credit_v1 as credit;
use settle_weather_v1 as weather;

#[test]
fn commitment_matches_credit() {
    let buyer = vec![0x12u8; 40];
    let salt = [7u8; 32];
    assert_eq!(
        weather::commitment(&buyer, 800, &salt),
        credit::commitment(&buyer, 800, &salt),
        "Poseidon position commitment must be identical -> the same vault folds it"
    );
}

#[test]
fn position_root_matches_credit() {
    let w = vec![weather::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let c = vec![credit::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    assert_eq!(weather::position_root(&w), credit::position_root(&c));
}

#[test]
fn allocation_root_matches_credit() {
    let w = vec![weather::Allocation { buyer: vec![0xABu8; 40], amount: 400 }];
    let c = vec![credit::Allocation { buyer: vec![0xABu8; 40], amount: 400 }];
    assert_eq!(
        weather::allocation_root(&w),
        credit::allocation_root(&c),
        "allocation_root must match -> the same settlement contract verifies it"
    );
}

#[test]
fn config_hash_and_instrument_id_match_credit() {
    let wc = weather::ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: [0x11; 32],
        schedule_root: [0x22; 32],
        snapshot_root: [0x33; 32],
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1, 500), (2, 1000)],
    };
    let cc = credit::ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: [0x11; 32],
        schedule_root: [0x22; 32],
        snapshot_root: [0x33; 32],
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1, 500), (2, 1000)],
    };
    assert_eq!(weather::config_hash(&wc), credit::config_hash(&cc), "same factory derives the id");
    let tid = vec![0xCCu8, 9, 9];
    assert_eq!(
        weather::derive_instrument_id(&tid, 1, &weather::config_hash(&wc)),
        credit::derive_instrument_id(&tid, 1, &credit::config_hash(&cc)),
    );
}
