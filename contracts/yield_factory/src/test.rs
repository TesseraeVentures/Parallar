#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, BytesN, Env, String, Symbol, Vec};

// The protected family, embedded from the wasm built by `stellar contract build`.
mod vault_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_yield_vault.wasm");
}
mod settlement_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_settlement.wasm");
}
mod router_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_yield_router.wasm");
}

struct Ctx {
    factory: YieldFactoryClient<'static>,
    coupon: Address,
    verifier: Address,
}

fn setup(env: &Env) -> Ctx {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let verifier = Address::generate(env);
    let vhash = env.deployer().upload_contract_wasm(vault_wasm::WASM);
    let shash = env.deployer().upload_contract_wasm(settlement_wasm::WASM);
    let rhash = env.deployer().upload_contract_wasm(router_wasm::WASM);
    let factory = YieldFactoryClient::new(
        env,
        &env.register(YieldFactory, (admin, verifier.clone(), vhash, shash, rhash)),
    );
    let coupon = env.register_stellar_asset_contract_v2(Address::generate(env)).address();

    // two standardised risk tiers: investment-grade (tight, low premium) and high-yield (wider, high)
    factory.register_tier(&Symbol::new(env, "ig"), &100, &300, &0, &String::from_str(env, "investment grade"));
    factory.register_tier(&Symbol::new(env, "hy"), &500, &1500, &1000, &String::from_str(env, "high yield"));

    Ctx { factory, coupon, verifier }
}

fn mk_cfg(env: &Env, tag: u8, bond: &Address, coupon: &Address, premium_bps: u32) -> ProtectedConfig {
    let mut deadlines = Vec::new(env);
    deadlines.push_back((1u32, 500u64));
    ProtectedConfig {
        instrument_id: BytesN::from_array(env, &[tag; 32]),
        image_id: BytesN::from_array(env, &[9u8; 32]),
        bond_token: bond.clone(),
        coupon_token: coupon.clone(),
        premium_bps,
        protocol_fee_bps: 1200,
        dist_fee_bps: 1000,
        epoch_deadlines: deadlines,
    }
}

#[test]
fn deploy_protected_creates_cross_bound_family() {
    let env = Env::default();
    let c = setup(&env);
    let bond = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let (vault, settlement, router) =
        c.factory.deploy_protected(&Symbol::new(&env, "ig"), &mk_cfg(&env, 1, &bond, &c.coupon, 200));

    let v = vault_wasm::Client::new(&env, &vault);
    let s = settlement_wasm::Client::new(&env, &settlement);
    assert_eq!(v.settlement(), settlement, "vault bound to settlement");
    assert_eq!(v.collateral_token(), c.coupon);
    assert_eq!(s.vault(), vault, "settlement bound to vault");
    assert_eq!(s.image_id(), BytesN::from_array(&env, &[9u8; 32]));
    assert_eq!(s.verifier(), c.verifier);

    // the registry records the instrument under its tier
    let inst = c.factory.get_protected(&BytesN::from_array(&env, &[1u8; 32]));
    assert_eq!(inst.tier, Symbol::new(&env, "ig"));
    assert_eq!(inst.premium_bps, 200);
    assert_eq!(c.factory.tier_count(&Symbol::new(&env, "ig")), 1);
    let _ = router; // bound below in the economics test
}

#[test]
#[should_panic(expected = "outside the tier's risk band")]
fn premium_outside_tier_band_rejected() {
    let env = Env::default();
    let c = setup(&env);
    let bond = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    // 400 bps is outside investment-grade's 100–300 band
    c.factory.deploy_protected(&Symbol::new(&env, "ig"), &mk_cfg(&env, 1, &bond, &c.coupon, 400));
}

#[test]
fn risk_priced_tiers_yield_different_net_coupons() {
    // The core requirement: the SAME factory wraps two different bonds at two risk tiers, and the
    // risk-priced premium produces DIFFERENT net coupons for the protected holders.
    let env = Env::default();
    let c = setup(&env);
    let coupon_admin = token::StellarAssetClient::new(&env, &c.coupon);

    // investment-grade bond: premium 200 bps
    let bond_a = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let (va, _sa, ra) = c.factory.deploy_protected(&Symbol::new(&env, "ig"), &mk_cfg(&env, 1, &bond_a, &c.coupon, 200));
    // high-yield bond: premium 1000 bps (riskier → costlier protection)
    let bond_b = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let (vb, _sb, rb) = c.factory.deploy_protected(&Symbol::new(&env, "hy"), &mk_cfg(&env, 2, &bond_b, &c.coupon, 1000));

    // reserve A + wrap 5_000 of bond A
    let router_a = router_wasm::Client::new(&env, &ra);
    let seller_a = Address::generate(&env);
    coupon_admin.mint(&seller_a, &10_000);
    vault_wasm::Client::new(&env, &va).deposit(&seller_a, &10_000);
    let holder_a = Address::generate(&env);
    token::StellarAssetClient::new(&env, &bond_a).mint(&holder_a, &5_000);
    router_a.wrap(&holder_a, &5_000);

    // reserve B + wrap 5_000 of bond B
    let router_b = router_wasm::Client::new(&env, &rb);
    let seller_b = Address::generate(&env);
    coupon_admin.mint(&seller_b, &10_000);
    vault_wasm::Client::new(&env, &vb).deposit(&seller_b, &10_000);
    let holder_b = Address::generate(&env);
    token::StellarAssetClient::new(&env, &bond_b).mint(&holder_b, &5_000);
    router_b.wrap(&holder_b, &5_000);

    // same 700 gross coupon to each → different premium → different net coupon
    let issuer_a = Address::generate(&env);
    coupon_admin.mint(&issuer_a, &700);
    router_a.route_coupon(&issuer_a, &1u32, &700); // premium 5000*2% = 100 → net 600
    let issuer_b = Address::generate(&env);
    coupon_admin.mint(&issuer_b, &700);
    router_b.route_coupon(&issuer_b, &1u32, &700); // premium 5000*10% = 500 → net 200

    assert_eq!(router_a.pending_coupon(&holder_a), 600, "investment grade: 2% premium → 600 net");
    assert_eq!(router_b.pending_coupon(&holder_b), 200, "high yield: 10% premium → 200 net");
    assert!(
        router_a.pending_coupon(&holder_a) > router_b.pending_coupon(&holder_b),
        "risk is priced into the net coupon: the same bond cash-flow yields more, protected, in the safer tier"
    );
}
