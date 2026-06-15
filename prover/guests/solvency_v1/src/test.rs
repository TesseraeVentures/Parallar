#![cfg(test)]
use super::*;

// A purchase: running total goes old_total -> old_total + cover, must stay <= collateral.
// buyer/salt bind the same `cover` into the position commitment.
fn inputs(old_total: i128, cover: i128, collateral: i128) -> SolvencyInputs {
    let old_salt = [1u8; 32];
    let new_salt = [2u8; 32];
    let salt = [7u8; 32];
    let buyer = vec![0x12u8; 40];
    SolvencyInputs {
        collateral,
        prev_cover_commitment: commit_total(old_total, &old_salt),
        new_cover_commitment: commit_total(old_total + cover, &new_salt),
        position_commitment: settle_credit_v1::commitment(&buyer, cover, &salt),
        old_total,
        old_salt,
        new_salt,
        cover,
        buyer,
        salt,
    }
}

#[test]
fn solvent_purchase_proves() {
    // first purchase: 0 -> 600, under collateral 1000.
    let j = check(&inputs(0, 600, 1000)).expect("a solvent purchase proves");
    // the journal binds the commitments the vault will check + advance; the cover is NOT in it.
    assert_eq!(j.collateral, 1000);
    assert_eq!(j.prev_cover_commitment, commit_total(0, &[1u8; 32]));
    assert_eq!(j.new_cover_commitment, commit_total(600, &[2u8; 32]));
    let bytes = j.to_bytes();
    assert_eq!(bytes.len(), 112);
}

#[test]
fn second_solvent_purchase_proves() {
    // running total already 600; add 300 -> 900 <= 1000.
    check(&inputs(600, 300, 1000)).expect("still solvent");
}

#[test]
fn insolvent_purchase_is_unprovable() {
    // 800 + 300 = 1100 > 1000 collateral -> no proof can exist.
    assert_eq!(check(&inputs(800, 300, 1000)), Err(SolvencyError::Insolvent));
    // exactly at the limit is fine
    check(&inputs(800, 200, 1000)).expect("new_total == collateral is solvent");
}

#[test]
fn wrong_prev_commitment_rejected() {
    let mut i = inputs(0, 600, 1000);
    i.prev_cover_commitment = [0xFF; 32]; // not the stored aggregate
    assert_eq!(check(&i), Err(SolvencyError::PrevMismatch));
}

#[test]
fn wrong_new_commitment_rejected() {
    let mut i = inputs(0, 600, 1000);
    i.new_cover_commitment = commit_total(599, &[2u8; 32]); // doesn't open to new_total 600
    assert_eq!(check(&i), Err(SolvencyError::NewMismatch));
}

#[test]
fn cover_must_match_the_position_commitment() {
    // the position commits a DIFFERENT cover than the one added to the aggregate -> rejected.
    // (prevents adding 600 to the aggregate while committing a 50 position, or vice versa.)
    let mut i = inputs(0, 600, 1000);
    i.position_commitment = settle_credit_v1::commitment(&i.buyer, 50, &i.salt);
    assert_eq!(check(&i), Err(SolvencyError::PositionMismatch));
}

#[test]
fn the_cover_never_appears_in_the_journal() {
    // structural: the journal carries only commitments + the public collateral, never the cover.
    let j = check(&inputs(0, 777, 1000)).unwrap();
    let bytes = j.to_bytes();
    let cover_be = 777i128.to_be_bytes();
    // the cover's byte pattern must not appear anywhere in the public journal
    assert!(!bytes.windows(cover_be.len()).any(|w| w == cover_be), "cover must not leak into the journal");
}
