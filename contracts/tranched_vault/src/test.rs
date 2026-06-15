#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, Address, BytesN, Env, Vec};

struct Ctx {
    vault: Address,
    coll: Address,
    #[allow(dead_code)]
    settlement: Address,
    #[allow(dead_code)]
    admin: Address,
}

fn setup(env: &Env, weights: &[u32], premium_bps: u32, fee_bps: u32) -> Ctx {
    env.mock_all_auths();
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let settlement = Address::generate(env);
    let admin = Address::generate(env);
    let id = env.register(TranchedVault, ());
    let v = TranchedVaultClient::new(env, &id);
    let mut w = Vec::new(env);
    for x in weights {
        w.push_back(*x);
    }
    v.init(&settlement, &sac.address(), &admin, &premium_bps, &fee_bps, &w);
    Ctx { vault: id, coll: sac.address(), settlement, admin }
}
fn mint(env: &Env, coll: &Address, to: &Address, amt: i128) {
    token::StellarAssetClient::new(env, coll).mint(to, &amt);
}
fn comm(env: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(env, &[b; 32])
}

#[test]
fn junior_takes_first_loss_then_senior() {
    // Junior (rank 0) capital is consumed before senior; senior only absorbs the overflow.
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000); // junior tranche
    v.deposit(&senior, &1, &1_000); // senior tranche; total reserve 2_000

    let buyer = Address::generate(&env);
    v.buy_protection(&buyer, &comm(&env, 1), &1_500); // cover 1500 ≤ 2000

    // a default pays out 1_200: junior's 1_000 is wiped, senior absorbs the remaining 200.
    let payee = Address::generate(&env);
    v.pay_allocations(&1u32, &vec![&env, (payee.clone(), 1_200i128)]);

    assert_eq!(v.tranche_collateral(&0), 0, "junior wiped first");
    assert_eq!(v.tranche_collateral(&1), 800, "senior absorbed only the 200 overflow");
    assert_eq!(v.seller_value(&junior, &0), 0, "junior underwriter took the first loss");
    assert_eq!(v.seller_value(&senior, &1), 800, "senior underwriter was protected");
    assert_eq!(tok.balance(&payee), 1_200, "payee received the full default payout");
    assert_eq!(v.total_collateral(), 800);
}

#[test]
fn loss_within_junior_spares_senior_entirely() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000);

    let buyer = Address::generate(&env);
    v.buy_protection(&buyer, &comm(&env, 1), &1_500);

    let payee = Address::generate(&env);
    v.pay_allocations(&1u32, &vec![&env, (payee, 600i128)]); // 600 < junior 1000

    assert_eq!(v.tranche_collateral(&0), 400, "junior absorbed the whole loss");
    assert_eq!(v.tranche_collateral(&1), 1_000, "senior untouched");
}

#[test]
fn junior_earns_more_premium_by_weight() {
    // Equal collateral, weights 3:1 → junior earns 3× the senior's premium (paid for first-loss risk).
    let env = Env::default();
    let c = setup(&env, &[3, 1], 200, 0); // 2% premium, no protocol fee
    let v = TranchedVaultClient::new(&env, &c.vault);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000);

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 100);
    v.buy_protection(&buyer, &comm(&env, 1), &2_000); // premium = 2000*2% = 40

    assert_eq!(v.pending_premium(&junior, &0), 30, "junior: 3/4 of 40");
    assert_eq!(v.pending_premium(&senior, &1), 10, "senior: 1/4 of 40");
}

#[test]
fn loss_is_shared_pro_rata_within_a_tranche() {
    // Two underwriters in junior with different stakes both lose value pro-rata when junior absorbs.
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);

    let big = Address::generate(&env);
    let small = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &big, 750);
    mint(&env, &c.coll, &small, 250);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&big, &0, &750);
    v.deposit(&small, &0, &250); // junior total 1_000 (big 75%, small 25%)
    v.deposit(&senior, &1, &1_000);

    let buyer = Address::generate(&env);
    v.buy_protection(&buyer, &comm(&env, 1), &1_500);

    let payee = Address::generate(&env);
    v.pay_allocations(&1u32, &vec![&env, (payee, 800i128)]); // junior absorbs 800, 200 left

    assert_eq!(v.seller_value(&big, &0), 150, "75% of the surviving 200");
    assert_eq!(v.seller_value(&small, &0), 50, "25% of the surviving 200");
    assert_eq!(v.seller_value(&senior, &1), 1_000, "senior untouched");
}

#[test]
#[should_panic]
fn withdraw_below_cover_floor_rejected() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);
    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000);
    let buyer = Address::generate(&env);
    v.buy_protection(&buyer, &comm(&env, 1), &1_800); // cover 1800
    v.withdraw(&senior, &1, &500); // 2000-500 = 1500 < 1800 cover -> insolvent
}

#[test]
#[should_panic]
fn deposit_to_unknown_tranche_rejected() {
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);
    let s = Address::generate(&env);
    mint(&env, &c.coll, &s, 1_000);
    v.deposit(&s, &2, &1_000); // only ranks 0,1 exist
}

#[test]
fn withdraw_returns_loss_adjusted_collateral() {
    // After junior absorbs a loss, a junior underwriter withdraws only their reduced share value.
    let env = Env::default();
    let c = setup(&env, &[1, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let junior = Address::generate(&env);
    let senior = Address::generate(&env);
    mint(&env, &c.coll, &junior, 1_000);
    mint(&env, &c.coll, &senior, 1_000);
    v.deposit(&junior, &0, &1_000);
    v.deposit(&senior, &1, &1_000);
    let buyer = Address::generate(&env);
    v.buy_protection(&buyer, &comm(&env, 1), &500);

    let payee = Address::generate(&env);
    v.pay_allocations(&1u32, &vec![&env, (payee, 400i128)]); // junior 1000 -> 600

    // junior holds 1000 shares against 600 collateral; burning all shares returns 600
    let got = v.withdraw(&junior, &0, &1_000);
    assert_eq!(got, 600, "withdrew the loss-adjusted value");
    assert_eq!(tok.balance(&junior), 600);
    assert_eq!(v.tranche_collateral(&0), 0);
}

#[test]
fn pay_allocations_requires_the_bound_settlement_auth() {
    // Law #1: the reserve moves ONLY via the bound settlement. The other tests use
    // mock_all_auths(); this proves the first-loss payout gate REJECTS an unauthorized caller.
    let env = Env::default();
    let c = setup(&env, &[3, 1], 0, 0);
    let v = TranchedVaultClient::new(&env, &c.vault);
    let s = Address::generate(&env);
    mint(&env, &c.coll, &s, 1_000);
    v.deposit(&s, &0, &1_000);

    let payee = Address::generate(&env);
    env.set_auths(&[]); // the bound settlement has NOT signed this call
    let res = v.try_pay_allocations(&1u32, &vec![&env, (payee, 500i128)]);
    assert!(res.is_err(), "pay_allocations must trap without the bound settlement's auth");
    assert_eq!(v.total_collateral(), 1_000, "reserve must be untouched");
}
