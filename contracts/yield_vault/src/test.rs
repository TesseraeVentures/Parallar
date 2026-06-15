#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, BytesN, Env, Vec};

struct Ctx {
    vault: Address,
    coll: Address,
    settlement: Address,
    admin: Address,
}

fn setup_with(env: &Env, premium_bps: u32, fee_bps: u32, haircut_bps: u32) -> Ctx {
    env.mock_all_auths();
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let settlement = Address::generate(env);
    let admin = Address::generate(env);
    let vault_id = env.register(YieldVault, ());
    let v = YieldVaultClient::new(env, &vault_id);
    v.init(&settlement, &sac.address(), &admin, &premium_bps, &fee_bps, &haircut_bps);
    Ctx { vault: vault_id, coll: sac.address(), settlement, admin }
}
fn setup(env: &Env) -> Ctx {
    setup_with(env, 200, 1200, 0) // 2% premium, 12% protocol fee, no haircut
}
fn mint(env: &Env, coll: &Address, to: &Address, amt: i128) {
    token::StellarAssetClient::new(env, coll).mint(to, &amt);
}
fn comm(env: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(env, &[b; 32])
}

#[test]
fn buyer_premium_distributes_to_seller_and_protocol() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 7), &5_000); // premium = 5000*2% = 100

    // 100 premium: 12% (12) to protocol, 88 to the single seller.
    assert_eq!(v.protocol_fee_accrued(), 12);
    assert_eq!(v.pending_premium(&seller), 88);
    assert_eq!(tok.balance(&buyer), 900, "buyer paid 100 premium");

    v.claim_premium(&seller);
    assert_eq!(tok.balance(&seller), 88, "seller earned the premium");
    assert_eq!(v.pending_premium(&seller), 0);
    v.claim_protocol_fee();
    assert_eq!(tok.balance(&c.admin), 12, "protocol earned its base fee");
    // reserve untouched by premium flow
    assert_eq!(v.total_collateral(), 10_000);
}

#[test]
fn two_sellers_split_premium_pro_rata() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    mint(&env, &c.coll, &a, 6_000);
    mint(&env, &c.coll, &b, 4_000);
    v.deposit(&a, &6_000);
    v.deposit(&b, &4_000); // total 10_000; A 60%, B 40%

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &5_000); // premium 100, sellers' cut 88

    assert_eq!(v.pending_premium(&a), 52, "60% of 88 (floor)");
    assert_eq!(v.pending_premium(&b), 35, "40% of 88 (floor)");
}

#[test]
fn late_depositor_earns_nothing_from_past_premium() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);

    let early = Address::generate(&env);
    mint(&env, &c.coll, &early, 10_000);
    v.deposit(&early, &10_000);

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &5_000); // 88 to `early`

    // a seller who joins AFTER the premium accrued earns none of it
    let late = Address::generate(&env);
    mint(&env, &c.coll, &late, 10_000);
    v.deposit(&late, &10_000);
    assert_eq!(v.pending_premium(&late), 0, "late stake earns nothing retroactively");
    assert_eq!(v.pending_premium(&early), 88);
}

#[test]
fn withdraw_settles_premium_then_returns_collateral() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &5_000); // 88 premium to seller

    v.withdraw(&seller, &4_000); // 10000-4000 = 6000 >= 5000 cover -> ok
    assert_eq!(tok.balance(&seller), 4_000, "collateral returned");
    assert_eq!(v.total_collateral(), 6_000);
    // premium earned before the withdrawal was settled and is still claimable
    assert_eq!(v.pending_premium(&seller), 88);
    v.claim_premium(&seller);
    assert_eq!(tok.balance(&seller), 4_088, "collateral + earned premium");
}

#[test]
#[should_panic(expected = "cover floor")]
fn withdraw_cannot_drop_reserve_below_cover() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &8_000); // cover 8000
    v.withdraw(&seller, &3_000); // 10000-3000=7000 < 8000 cover -> panic
}

#[test]
#[should_panic(expected = "insolvent")]
fn buy_beyond_collateral_rejected() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &1_000);
    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &1_001); // 1001 > 1000 collateral
}

#[test]
#[should_panic(expected = "insolvent")]
fn haircut_tightens_the_solvency_floor() {
    let env = Env::default();
    let c = setup_with(&env, 200, 1200, 1000); // 10% haircut: cover ≤ 90% collateral
    let v = YieldVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &1_000);
    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &901); // 901 > 900 (=90% of 1000) -> insolvent
}

#[test]
fn pay_allocations_pays_from_reserve_not_premium() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 1_000);
    v.buy_protection(&buyer, &comm(&env, 1), &5_000); // 88 premium to seller

    // settlement pays a default of 3000 to a payee
    let payee = Address::generate(&env);
    let mut alloc = Vec::new(&env);
    alloc.push_back((payee.clone(), 3_000i128));
    v.pay_allocations(&1u32, &alloc);
    assert_eq!(tok.balance(&payee), 3_000);
    assert_eq!(v.total_collateral(), 7_000, "reserve reduced by the payout");

    // the seller's premium is unaffected by the default payout and still claimable
    assert_eq!(v.pending_premium(&seller), 88);
    v.claim_premium(&seller);
    assert_eq!(tok.balance(&seller), 88);
}

#[test]
fn receive_premium_from_router_distributes() {
    let env = Env::default();
    let c = setup(&env);
    let v = YieldVaultClient::new(&env, &c.vault);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);

    // the router routes a coupon's premium cut (e.g. 500) into the vault
    let router = Address::generate(&env);
    mint(&env, &c.coll, &router, 500);
    v.receive_premium(&router, &500);

    // 500: 12% (60) protocol, 440 to the single seller
    assert_eq!(v.protocol_fee_accrued(), 60);
    assert_eq!(v.pending_premium(&seller), 440);
}
