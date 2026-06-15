#![cfg(test)]
extern crate std;

use super::*;
use parallar_yield_vault::{YieldVault, YieldVaultClient};
use soroban_sdk::{testutils::Address as _, token, Address, Env};

struct Ctx {
    router: Address,
    vault: Address,
    bond: Address,
    coupon: Address,
    admin: Address,
}

// premium 2% (200 bps), vault base fee 12% (1200), router distribution fee 10% (1000), haircut 0.
fn setup(env: &Env) -> Ctx {
    env.mock_all_auths();
    let bond = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
    let coupon = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
    let admin = Address::generate(env);
    let settlement = Address::generate(env);

    let vault = env.register(YieldVault, ());
    YieldVaultClient::new(env, &vault).init(&settlement, &coupon, &admin, &200, &1200, &0);

    let router = env.register(YieldRouter, ());
    YieldRouterClient::new(env, &router).init(&bond, &coupon, &vault, &admin, &200, &1000);
    YieldVaultClient::new(env, &vault).set_router(&router);

    Ctx { router, vault, bond, coupon, admin }
}
fn mint(env: &Env, tok: &Address, to: &Address, amt: i128) {
    token::StellarAssetClient::new(env, tok).mint(to, &amt);
}

#[test]
fn wrap_route_coupon_full_waterfall() {
    let env = Env::default();
    let c = setup(&env);
    let router = YieldRouterClient::new(&env, &c.router);
    let vault = YieldVaultClient::new(&env, &c.vault);
    let coupon = token::Client::new(&env, &c.coupon);

    // underwriter funds the reserve
    let seller = Address::generate(&env);
    mint(&env, &c.coupon, &seller, 10_000);
    vault.deposit(&seller, &10_000);

    // a holder wraps 5_000 of the bond -> pBOND; cover auto-sizes + registers (5000 <= 10000)
    let holder = Address::generate(&env);
    mint(&env, &c.bond, &holder, 5_000);
    router.wrap(&holder, &5_000);
    assert_eq!(router.pbond_balance(&holder), 5_000);
    assert_eq!(vault.routed_cover(), 5_000);
    assert_eq!(vault.total_cover(), 5_000);

    // the issuer pays a 14%-on-5000 = 700 gross coupon for the wrapped bonds
    let issuer = Address::generate(&env);
    mint(&env, &c.coupon, &issuer, 700);
    router.route_coupon(&issuer, &1u32, &700);
    // premium = 5000*2% = 100; dist_fee = 100*10% = 10; to_vault = 90; net = 600.
    // vault base fee = 90*12% = 10; sellers' cut = 80.
    assert_eq!(router.dist_fee_accrued(), 10, "router distribution fee");
    assert_eq!(router.pending_coupon(&holder), 600, "12% net coupon to the protected holder");
    assert_eq!(vault.pending_premium(&seller), 80, "premium to the underwriter");
    assert_eq!(vault.protocol_fee_accrued(), 10, "vault base fee");

    // everyone claims; the 700 gross splits exactly 600 + 80 + 10 + 10
    router.claim_coupon(&holder);
    vault.claim_premium(&seller);
    vault.claim_protocol_fee();
    router.claim_dist_fee();
    assert_eq!(coupon.balance(&holder), 600);
    // seller deposited 10000 collateral; earns 80 premium on top
    assert_eq!(coupon.balance(&seller), 80);
    assert_eq!(coupon.balance(&c.admin), 20, "vault base fee 10 + router dist fee 10");
}

#[test]
#[should_panic(expected = "insolvent")]
fn wrap_beyond_reserve_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let router = YieldRouterClient::new(&env, &c.router);
    let vault = YieldVaultClient::new(&env, &c.vault);

    let seller = Address::generate(&env);
    mint(&env, &c.coupon, &seller, 1_000);
    vault.deposit(&seller, &1_000);

    let holder = Address::generate(&env);
    mint(&env, &c.bond, &holder, 5_000);
    router.wrap(&holder, &1_001); // routed cover 1001 > 1000 reserve -> vault rejects
}

#[test]
fn unwrap_returns_bond_and_lapses_cover() {
    let env = Env::default();
    let c = setup(&env);
    let router = YieldRouterClient::new(&env, &c.router);
    let vault = YieldVaultClient::new(&env, &c.vault);
    let bond = token::Client::new(&env, &c.bond);

    let seller = Address::generate(&env);
    mint(&env, &c.coupon, &seller, 10_000);
    vault.deposit(&seller, &10_000);

    let holder = Address::generate(&env);
    mint(&env, &c.bond, &holder, 5_000);
    router.wrap(&holder, &5_000);
    router.unwrap(&holder, &2_000);

    assert_eq!(router.pbond_balance(&holder), 3_000);
    assert_eq!(vault.routed_cover(), 3_000, "cover lapsed by the unwrapped amount");
    assert_eq!(bond.balance(&holder), 2_000, "bond returned");
}

#[test]
fn two_holders_split_net_coupon_pro_rata() {
    let env = Env::default();
    let c = setup(&env);
    let router = YieldRouterClient::new(&env, &c.router);
    let vault = YieldVaultClient::new(&env, &c.vault);

    let seller = Address::generate(&env);
    mint(&env, &c.coupon, &seller, 20_000);
    vault.deposit(&seller, &20_000);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    mint(&env, &c.bond, &a, 6_000);
    mint(&env, &c.bond, &b, 4_000);
    router.wrap(&a, &6_000);
    router.wrap(&b, &4_000); // total wrapped 10_000

    // gross 1_000; premium = 10000*2% = 200; net = 800, split 60/40
    let issuer = Address::generate(&env);
    mint(&env, &c.coupon, &issuer, 1_000);
    router.route_coupon(&issuer, &1u32, &1_000);
    assert_eq!(router.pending_coupon(&a), 480, "60% of the 800 net coupon");
    assert_eq!(router.pending_coupon(&b), 320, "40% of the 800 net coupon");
}

#[test]
fn claim_dist_fee_requires_admin() {
    // The router's distribution-fee pool is admin-only; an outside caller must not drain it.
    // Other tests use mock_all_auths(); this proves the admin gate REJECTS an unauthorized caller.
    let env = Env::default();
    let c = setup(&env);
    let router = YieldRouterClient::new(&env, &c.router);
    env.set_auths(&[]);
    let res = router.try_claim_dist_fee();
    assert!(res.is_err(), "claim_dist_fee must trap without the admin's auth");
}
