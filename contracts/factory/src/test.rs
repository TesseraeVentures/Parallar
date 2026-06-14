#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, BytesN, Env, String, Symbol, Vec};

// The generic pair, embedded from the wasm built by `stellar contract build`.
mod vault_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_vault.wasm");
}
mod settlement_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/parallar_settlement.wasm");
}

fn type_id(env: &Env) -> Symbol {
    Symbol::new(env, "credit_v1")
}

/// Factory + uploaded vault/settlement hashes + an eligible collateral SAC.
fn setup(env: &Env) -> (ParallarFactoryClient<'static>, BytesN<32>, BytesN<32>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let verifier = Address::generate(env); // the network's RISC Zero verifier router
    let factory =
        ParallarFactoryClient::new(env, &env.register(ParallarFactory, (admin, verifier.clone())));
    let vault_hash = env.deployer().upload_contract_wasm(vault_wasm::WASM);
    let settlement_hash = env.deployer().upload_contract_wasm(settlement_wasm::WASM);
    let coll = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
    factory.set_collateral_eligible(&coll, &true);
    (factory, vault_hash, settlement_hash, coll, verifier)
}

fn register_credit(env: &Env, f: &ParallarFactoryClient, vhash: &BytesN<32>, shash: &BytesN<32>, image_id: &BytesN<32>) {
    let t = InstrumentType {
        image_id: image_id.clone(),
        rules_version: 1,
        rules_uri: String::from_str(env, "ipfs://credit_v1"),
        vault_wasm: vhash.clone(),
        settlement_wasm: shash.clone(),
    };
    f.register_type(&type_id(env), &t);
}

fn mk_config(env: &Env, collateral: &Address, tag: u8) -> InstrumentConfig {
    let z = BytesN::from_array(env, &[tag; 32]);
    let mut deadlines = Vec::new(env);
    deadlines.push_back((1u32, 500u64));
    InstrumentConfig {
        reference_asset: Address::generate(env),
        terms_hash: z.clone(),
        schedule_root: z.clone(),
        snapshot_root: z,
        collateral_token: collateral.clone(),
        premium_bps: 200,
        epoch_deadlines: deadlines,
    }
}

#[test]
fn deploy_instrument_creates_cross_bound_live_pair() {
    let env = Env::default();
    let (factory, vhash, shash, coll, verifier) = setup(&env);
    let image_id = BytesN::from_array(&env, &[9u8; 32]);
    register_credit(&env, &factory, &vhash, &shash, &image_id);

    let (iid, vault_addr, settlement_addr) =
        factory.deploy_instrument(&type_id(&env), &mk_config(&env, &coll, 1));

    // registry record
    let inst = factory.get_instrument(&iid);
    assert_eq!(inst.vault, vault_addr);
    assert_eq!(inst.settlement, settlement_addr);
    assert_eq!(inst.type_id, type_id(&env));

    // cross-bindings on the actually-deployed contracts
    let v = vault_wasm::Client::new(&env, &vault_addr);
    let s = settlement_wasm::Client::new(&env, &settlement_addr);
    assert_eq!(v.settlement(), settlement_addr, "vault bound to settlement");
    assert_eq!(v.collateral_token(), coll);
    assert_eq!(s.vault(), vault_addr, "settlement bound to vault");
    assert_eq!(s.instrument_id(), iid);
    assert_eq!(s.image_id(), image_id, "instance pinned to type image_id");
    assert_eq!(s.verifier(), verifier, "settlement bound to the factory's verifier router");
    assert_eq!(s.deadline(&1u32), 500u64);

    // the deployed vault is live
    let coll_admin = token::StellarAssetClient::new(&env, &coll);
    let seller = Address::generate(&env);
    coll_admin.mint(&seller, &1_000);
    v.deposit(&seller, &1_000);
    assert_eq!(v.total_collateral(), 1_000);
}

#[test]
fn second_deploy_is_an_independent_instance() {
    // The replication beat: a second deploy_instrument yields a distinct, separate pair.
    let env = Env::default();
    let (factory, vhash, shash, coll, _verifier) = setup(&env);
    register_credit(&env, &factory, &vhash, &shash, &BytesN::from_array(&env, &[9u8; 32]));

    let (iid1, v1, s1) = factory.deploy_instrument(&type_id(&env), &mk_config(&env, &coll, 1));
    let (iid2, v2, s2) = factory.deploy_instrument(&type_id(&env), &mk_config(&env, &coll, 2));

    assert!(iid1 != iid2, "different config -> different instrument_id");
    assert!(v1 != v2 && s1 != s2, "independent vault/settlement addresses");
    assert_eq!(factory.get_instrument(&iid1).vault, v1);
    assert_eq!(factory.get_instrument(&iid2).settlement, s2);
}

#[test]
#[should_panic(expected = "not eligible")]
fn ineligible_collateral_rejected() {
    let env = Env::default();
    let (factory, vhash, shash, _coll, _verifier) = setup(&env);
    register_credit(&env, &factory, &vhash, &shash, &BytesN::from_array(&env, &[9u8; 32]));

    // a fresh SAC that was never marked eligible (could be clawback-enabled)
    let bad = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    factory.deploy_instrument(&type_id(&env), &mk_config(&env, &bad, 1));
}

#[test]
#[should_panic(expected = "type not registered")]
fn unregistered_type_rejected() {
    let env = Env::default();
    let (factory, _vhash, _shash, coll, _verifier) = setup(&env);
    // no register_type
    factory.deploy_instrument(&Symbol::new(&env, "weather_v1"), &mk_config(&env, &coll, 1));
}

#[test]
#[should_panic(expected = "already registered")]
fn register_type_is_immutable() {
    let env = Env::default();
    let (factory, vhash, shash, _coll, _verifier) = setup(&env);
    let image_id = BytesN::from_array(&env, &[9u8; 32]);
    register_credit(&env, &factory, &vhash, &shash, &image_id);
    register_credit(&env, &factory, &vhash, &shash, &image_id); // re-register -> panic
}
