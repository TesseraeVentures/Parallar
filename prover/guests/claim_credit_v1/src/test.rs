#![cfg(test)]
use super::*;

fn h(b: u8) -> Holder {
    Holder { id: [b; 32], balance: 10_000, has_trustline: true, frozen: false }
}

// 2 holders owed 1000 each; holder 2 paid in full, holder 1 unpaid -> severity 0.5.
// 2 buyers: cover 600 (claimant, index 0) and 400. Claimant's pro-rata payout = 300.
fn scenario() -> ClaimInputs {
    let snapshot = vec![h(1), h(2)];
    let payments = vec![Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }];
    let terms = Terms { coupon_rate_bps: 1000 };
    let pos0 = Position { buyer: vec![0x10u8; 40], cover: 600, salt: [1; 32] };
    let pos1 = Position { buyer: vec![0x20u8; 40], cover: 400, salt: [2; 32] };
    let positions = vec![pos0.clone(), pos1.clone()];
    let commitments = vec![
        commitment(&pos0.buyer, pos0.cover, &pos0.salt),
        commitment(&pos1.buyer, pos1.cover, &pos1.salt),
    ];
    let position_root = settle_credit_v1::position_root(&positions);
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&snapshot),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    ClaimInputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 2000,
        snapshot,
        payments,
        commitments,
        claimant_index: 0,
        claimant: pos0,
        position_root,
    }
}

#[test]
fn claim_pays_the_claimant_pro_rata() {
    let (alloc, j) = claim(&scenario()).expect("claim settles");
    assert_eq!(alloc.amount, 300, "cover 600 × 0.5 severity");
    assert_eq!(j.total_payout, 300);
    assert_eq!(alloc.buyer, vec![0x10u8; 40]);
    assert_eq!(j.allocation_root, super::single_allocation_root(&alloc));
}

#[test]
fn claim_matches_what_a_full_settlement_would_pay() {
    use settle_credit_v1 as v1;
    let s = scenario();
    let (claim_alloc, _) = claim(&s).unwrap();

    // the same book under a FULL settlement; the claimant's allocation must be identical
    let positions = vec![
        v1::Position { buyer: vec![0x10u8; 40], cover: 600, salt: [1; 32] },
        v1::Position { buyer: vec![0x20u8; 40], cover: 400, salt: [2; 32] },
    ];
    let full = v1::Inputs {
        type_id_xdr: s.type_id_xdr.clone(),
        rules_version: 1,
        instrument_id: s.instrument_id,
        config: s.config.clone(),
        epoch: 1,
        deadline: 500,
        terms: Terms { coupon_rate_bps: 1000 },
        collateral: 2000,
        snapshot: vec![h(1), h(2)],
        payments: vec![Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }],
        positions: positions.clone(),
        position_root: v1::position_root(&positions),
    };
    let (allocs, _) = v1::settle(&full).unwrap();
    let full_for_claimant = allocs.iter().find(|a| a.buyer == vec![0x10u8; 40]).unwrap().amount;
    assert_eq!(claim_alloc.amount, full_for_claimant, "claim == the buyer's full-settlement share");
}

#[test]
fn wrong_opening_rejected() {
    let mut s = scenario();
    s.claimant.cover = 999; // opening no longer matches commitments[0]
    assert_eq!(claim(&s), Err(ClaimError::CommitmentMismatch));
}

#[test]
fn tampered_position_root_rejected() {
    let mut s = scenario();
    s.position_root = [0xFF; 32];
    assert_eq!(claim(&s), Err(ClaimError::PositionMismatch));
}

#[test]
fn out_of_range_index_rejected() {
    let mut s = scenario();
    s.claimant_index = 7;
    assert_eq!(claim(&s), Err(ClaimError::BadIndex));
}

#[test]
fn no_default_is_unprovable() {
    let mut s = scenario();
    // pay holder 1 in full too -> no shortfall
    s.payments.push(Payment { holder: [1; 32], amount: 1000, paid_at: 400, clawed_back: false });
    assert_eq!(claim(&s), Err(ClaimError::NoDefault));
}
