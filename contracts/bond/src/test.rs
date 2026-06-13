#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, BytesN, Env, Vec};

fn zero32(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

/// Issuer is the SAC admin so it can mint itself coupon funds; returns (token client, admin client).
fn make_coupon<'a>(env: &Env, issuer: &Address) -> (token::Client<'a>, token::StellarAssetClient<'a>) {
    let sac = env.register_stellar_asset_contract_v2(issuer.clone());
    (
        token::Client::new(env, &sac.address()),
        token::StellarAssetClient::new(env, &sac.address()),
    )
}

fn deploy_bond<'a>(env: &Env, issuer: &Address, coupon_token: &Address) -> BondContractClient<'a> {
    // (helper) coupon_token is the SAC address; bond is initialized with zero commitments.
    let client = BondContractClient::new(env, &env.register(BondContract, ()));
    let z = zero32(env);
    client.init(issuer, coupon_token, &z, &z, &z);
    client
}

#[test]
fn full_coupon_pays_all_ten_holders() {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let (coupon, coupon_admin) = make_coupon(&env, &issuer);
    coupon_admin.mint(&issuer, &10_000);

    let mut holders = Vec::new(&env);
    for _ in 0..10 {
        holders.push_back(Address::generate(&env));
    }

    let bond = deploy_bond(&env, &issuer, &coupon.address.clone());

    let mut payments = Vec::new(&env);
    for h in holders.iter() {
        payments.push_back((h.clone(), 100i128));
    }
    bond.pay_coupon(&0u32, &payments);

    for h in holders.iter() {
        assert_eq!(coupon.balance(&h), 100);
    }
    assert_eq!(coupon.balance(&issuer), 10_000 - 1_000);
}

#[test]
fn partial_coupon_pays_only_listed_holders() {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let (coupon, coupon_admin) = make_coupon(&env, &issuer);
    coupon_admin.mint(&issuer, &10_000);

    let mut holders = Vec::new(&env);
    for _ in 0..10 {
        holders.push_back(Address::generate(&env));
    }

    let bond = deploy_bond(&env, &issuer, &coupon.address.clone());

    // Issuer short-pays: only the first 7 of 10 holders (R1 acceptance scenario).
    let mut payments = Vec::new(&env);
    for (i, h) in holders.iter().enumerate() {
        if i < 7 {
            payments.push_back((h.clone(), 100i128));
        }
    }
    bond.pay_coupon(&1u32, &payments);

    for (i, h) in holders.iter().enumerate() {
        let bal = coupon.balance(&h);
        if i < 7 {
            assert_eq!(bal, 100, "holder {i} should be paid");
        } else {
            assert_eq!(bal, 0, "holder {i} should be unpaid (short-pay)");
        }
    }
}

#[test]
fn commitments_are_stored_and_readable() {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let (coupon, _) = make_coupon(&env, &issuer);

    let bond = BondContractClient::new(&env, &env.register(BondContract, ()));
    let terms = BytesN::from_array(&env, &[1u8; 32]);
    let schedule = BytesN::from_array(&env, &[2u8; 32]);
    let snapshot = BytesN::from_array(&env, &[3u8; 32]);
    bond.init(&issuer, &coupon.address.clone(), &terms, &schedule, &snapshot);

    assert_eq!(bond.issuer(), issuer);
    assert_eq!(bond.coupon_token(), coupon.address.clone());
    assert_eq!(bond.terms_commitment(), terms);
    assert_eq!(bond.schedule_root(), schedule);
    assert_eq!(bond.snapshot_root(), snapshot);
}

#[test]
#[should_panic(expected = "already initialized")]
fn double_init_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let (coupon, _) = make_coupon(&env, &issuer);
    let bond = deploy_bond(&env, &issuer, &coupon.address.clone());
    let z = zero32(&env);
    bond.init(&issuer, &coupon.address.clone(), &z, &z, &z);
}
