#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Symbol, Vec};

// The claimable family, embedded from the wasm built by `stellar contract build` / cargo.
mod vault_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_vault.wasm");
}
mod claim_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_claim_settlement.wasm");
}

struct Ctx {
    factory: ClaimFactoryClient<'static>,
    coll: Address,
    verifier: Address,
}

fn setup(env: &Env) -> Ctx {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let verifier = Address::generate(env);
    let vhash = env.deployer().upload_contract_wasm(vault_wasm::WASM);
    let chash = env.deployer().upload_contract_wasm(claim_wasm::WASM);
    let factory = ClaimFactoryClient::new(env, &env.register(ClaimFactory, (admin, verifier.clone(), vhash, chash)));
    let coll = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
    Ctx { factory, coll, verifier }
}

fn cfg(env: &Env, coll: &Address) -> InstrumentConfig {
    let mut deadlines = Vec::new(env);
    deadlines.push_back((1u32, 500u64));
    InstrumentConfig {
        reference_asset: Address::generate(env),
        terms_hash: BytesN::from_array(env, &[0x11; 32]),
        schedule_root: BytesN::from_array(env, &[0x22; 32]),
        snapshot_root: BytesN::from_array(env, &[0x33; 32]),
        collateral_token: coll.clone(),
        premium_bps: 200,
        epoch_deadlines: deadlines,
    }
}

fn ctype(env: &Env) -> ClaimableType {
    ClaimableType {
        settle_image_id: BytesN::from_array(env, &[9u8; 32]),
        claim_image_id: BytesN::from_array(env, &[8u8; 32]),
        rules_version: 1,
        grace: 100,
    }
}

#[test]
fn deploy_claimable_creates_cross_bound_family() {
    let env = Env::default();
    let c = setup(&env);
    c.factory.set_collateral_eligible(&c.coll, &true);
    let ty = Symbol::new(&env, "credit_v1_claim");
    c.factory.register_claimable_type(&ty, &ctype(&env));

    let (instrument_id, vault_addr, claim_addr) = c.factory.deploy_claimable(&ty, &cfg(&env, &c.coll));

    let v = vault_wasm::Client::new(&env, &vault_addr);
    let cs = claim_wasm::Client::new(&env, &claim_addr);
    assert_eq!(v.settlement(), claim_addr, "vault bound to the claimable settlement");
    assert_eq!(v.collateral_token(), c.coll);
    assert_eq!(cs.settle_image_id(), BytesN::from_array(&env, &[9u8; 32]), "settle guest bound");
    assert_eq!(cs.claim_image_id(), BytesN::from_array(&env, &[8u8; 32]), "claim guest bound");
    assert_eq!(cs.grace(), 100, "keeper-grace window bound");

    let inst = c.factory.get_instrument(&instrument_id);
    assert_eq!(inst.vault, vault_addr);
    assert_eq!(inst.settlement, claim_addr);
    let _ = c.verifier;
}

#[test]
#[should_panic(expected = "not eligible")]
fn ineligible_collateral_rejected() {
    let env = Env::default();
    let c = setup(&env);
    let ty = Symbol::new(&env, "credit_v1_claim");
    c.factory.register_claimable_type(&ty, &ctype(&env));
    // collateral never marked eligible -> the clawback/freeze gate rejects the deploy
    c.factory.deploy_claimable(&ty, &cfg(&env, &c.coll));
}

#[test]
#[should_panic(expected = "type already registered")]
fn claimable_type_is_immutable() {
    let env = Env::default();
    let c = setup(&env);
    let ty = Symbol::new(&env, "credit_v1_claim");
    c.factory.register_claimable_type(&ty, &ctype(&env));
    c.factory.register_claimable_type(&ty, &ctype(&env)); // re-register -> panic
}
