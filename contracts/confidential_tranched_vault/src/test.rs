#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, token, vec, Address, Bytes, BytesN, Env, Vec,
};

#[contract]
struct MockRouter;
#[contractimpl]
impl MockRouter {
    pub fn verify(env: Env, seal: Bytes, _image_id: BytesN<32>, _journal: BytesN<32>) {
        if seal == Bytes::from_array(&env, &[0xFFu8; 4]) {
            panic!("mock router: invalid proof");
        }
    }
}

struct Ctx {
    vault: Address,
    coll: Address,
    #[allow(dead_code)]
    settlement: Address,
}

fn setup(env: &Env, weights: &[u32], fee_bps: u32) -> Ctx {
    env.mock_all_auths();
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let settlement = Address::generate(env);
    let admin = Address::generate(env);
    let router = env.register(MockRouter, ());
    let vault = env.register(ConfidentialTranchedVault, ());
    let mut w = Vec::new(env);
    for x in weights {
        w.push_back(*x);
    }
    ConfidentialTranchedVaultClient::new(env, &vault).init(
        &settlement,
        &sac.address(),
        &admin,
        &BytesN::from_array(env, &[9u8; 32]),
        &router,
        &fee_bps,
        &w,
        &b32(env, 0),
    );
    Ctx { vault, coll: sac.address(), settlement }
}
fn mint(env: &Env, coll: &Address, to: &Address, amt: i128) {
    token::StellarAssetClient::new(env, coll).mint(to, &amt);
}
fn b32(env: &Env, v: u8) -> BytesN<32> {
    BytesN::from_array(env, &[v; 32])
}
fn good_seal(env: &Env) -> Bytes {
    Bytes::from_array(env, &[0x01u8; 4])
}
fn buy_journal(env: &Env, prev: &BytesN<32>, new: &BytesN<32>, pos: &BytesN<32>, collateral: i128) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &prev.to_array()));
    b.append(&Bytes::from_array(env, &new.to_array()));
    b.append(&Bytes::from_array(env, &pos.to_array()));
    b.append(&Bytes::from_array(env, &collateral.to_be_bytes()));
    b
}
fn withdraw_journal(env: &Env, cc: &BytesN<32>, collateral_after: i128) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &cc.to_array()));
    b.append(&Bytes::from_array(env, &collateral_after.to_be_bytes()));
    b
}

#[test]
fn confidential_buy_advances_commitment_and_splits_premium_by_tranche() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0); // junior weight 3, senior 1; no protocol fee
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000); // total reserve 2_000

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 40);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 1_500);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &40); // declared premium 40

    assert_eq!(v.cover_commitment(), b32(&env, 1), "hidden aggregate advanced");
    assert_ne!(v.position_root(), b32(&env, 0), "position folded");
    // premium 40 split 3:1 by tranche weight
    assert_eq!(v.pending_premium(&junior, &0), 30, "junior earns 3/4");
    assert_eq!(v.pending_premium(&senior, &1), 10, "senior earns 1/4");
}

#[test]
fn junior_takes_first_loss_under_confidential_cover() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000);

    // a proven purchase, then a default pays 1_200: junior wiped, senior absorbs 200
    let buyer = Address::generate(&env);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 1_500);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);

    let payee = Address::generate(&env);
    v.pay_allocations(&1u32, &vec![&env, (payee.clone(), 1_200i128)]);
    assert_eq!(v.tranche_collateral(&0), 0, "junior wiped first");
    assert_eq!(v.tranche_collateral(&1), 800, "senior absorbed the 200 overflow");
    assert_eq!(v.seller_value(&junior, &0), 0);
    assert_eq!(v.seller_value(&senior, &1), 800);
    assert_eq!(tok.balance(&payee), 1_200);
}

#[test]
fn proven_withdrawal_burns_tranche_shares() {
    let env = Env::default();
    let c = setup(&env, &[1, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let junior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    v.deposit(&junior, &0, &1_000);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&senior, &1, &1_000); // total 2_000

    // withdraw 400 from junior: collateral_after must be 2_000 - 400 = 1_600
    let wj = withdraw_journal(&env, &b32(&env, 0), 1_600);
    let got = v.withdraw_proven(&junior, &0, &400, &good_seal(&env), &wj);
    assert_eq!(got, 400);
    assert_eq!(tok.balance(&junior), 400);
    assert_eq!(v.tranche_collateral(&0), 600);
    assert_eq!(v.total_collateral(), 1_600);
}

#[test]
#[should_panic]
fn forged_solvency_proof_reverts() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &0, &1_000);
    let buyer = Address::generate(&env);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &Bytes::from_array(&env, &[0xFFu8; 4]), &j, &0);
}

#[test]
#[should_panic]
fn buy_from_wrong_prev_commitment_reverts() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &0, &1_000);
    let buyer = Address::generate(&env);
    let j = buy_journal(&env, &b32(&env, 5), &b32(&env, 1), &b32(&env, 7), 600); // prev != stored [0;32]
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);
}

#[test]
#[should_panic]
fn withdrawal_collateral_after_must_match_real_reserve() {
    let env = Env::default();
    let c = setup(&env, &[1, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let junior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    v.deposit(&junior, &0, &1_000); // total 1_000
    // collateral_after 700 != real total - amount (1_000 - 400 = 600)
    let wj = withdraw_journal(&env, &b32(&env, 0), 700);
    v.withdraw_proven(&junior, &0, &400, &good_seal(&env), &wj);
}

#[test]
fn pay_allocations_requires_the_bound_settlement_auth() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0);
    let v = ConfidentialTranchedVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &0, &1_000);

    let payee = Address::generate(&env);
    env.set_auths(&[]);
    let res = v.try_pay_allocations(&1u32, &vec![&env, (payee, 500i128)]);
    assert!(res.is_err(), "pay_allocations must trap without the bound settlement's auth");
    assert_eq!(v.total_collateral(), 1_000, "reserve untouched");
}
