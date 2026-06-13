#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, Bytes, BytesN, Env, Vec};

/// Registers a collateral SAC + a (dummy) bound settlement, deploys & inits the vault.
/// Returns (vault_id, collateral_sac_addr, settlement_addr).
fn setup(env: &Env) -> (Address, Address, Address) {
    env.mock_all_auths();
    let sac_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin);
    let settlement = Address::generate(env);
    let vault_id = env.register(VaultContract, ());
    let vault = VaultContractClient::new(env, &vault_id);
    vault.init(&settlement, &sac.address());
    (vault_id, sac.address(), settlement)
}

fn comm(env: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(env, &[b; 32])
}

#[test]
fn deposit_tracks_collateral_and_seller_balance() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral = token::Client::new(&env, &coll);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);

    assert_eq!(vault.total_collateral(), 1_000);
    assert_eq!(vault.seller_balance(&seller), 1_000);
    assert_eq!(collateral.balance(&vault_id), 1_000);
}

#[test]
fn buy_protection_folds_root_and_tracks_cover() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);
    assert_eq!(vault.position_root(), comm(&env, 0)); // zeros before any buy

    let buyer = Address::generate(&env);
    let c1 = comm(&env, 7);
    vault.buy_protection(&buyer, &c1, &600);

    assert_eq!(vault.total_cover(), 600);

    // position_root == sha256(zeros(32) || c1) — exactly the fold the guest reproduces
    let mut buf = Bytes::new(&env);
    buf.append(&Bytes::from_array(&env, &[0u8; 32]));
    buf.append(&Bytes::from_array(&env, &c1.to_array()));
    let expected: BytesN<32> = env.crypto().sha256(&buf).to_bytes();
    assert_eq!(vault.position_root(), expected);
}

#[test]
#[should_panic(expected = "insolvent")]
fn buy_protection_rejects_cover_exceeding_collateral() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &500);
    vault.deposit(&seller, &500);

    let buyer = Address::generate(&env);
    vault.buy_protection(&buyer, &comm(&env, 1), &600); // 600 > 500 collateral
}

#[test]
#[should_panic(expected = "below outstanding cover")]
fn withdraw_cannot_drop_collateral_below_cover() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);
    let buyer = Address::generate(&env);
    vault.buy_protection(&buyer, &comm(&env, 1), &800);

    vault.withdraw(&seller, &300); // 1000-300=700 < 800 cover -> panic
}

#[test]
#[should_panic(expected = "frozen")]
fn withdraw_frozen_during_settlement_window() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);

    vault.set_window(&true); // settlement opens the freeze window
    vault.withdraw(&seller, &100); // -> panic
}

#[test]
fn pay_allocations_pays_recipients_and_reduces_collateral() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral = token::Client::new(&env, &coll);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let mut alloc = Vec::new(&env);
    alloc.push_back((b1.clone(), 300i128));
    alloc.push_back((b2.clone(), 200i128));
    vault.pay_allocations(&1u32, &alloc); // settlement authorizes (mock_all_auths)

    assert_eq!(collateral.balance(&b1), 300);
    assert_eq!(collateral.balance(&b2), 200);
    assert_eq!(vault.total_collateral(), 500);
}

#[test]
#[should_panic(expected = "exceed collateral")]
fn pay_allocations_rejects_sum_above_collateral() {
    let env = Env::default();
    let (vault_id, coll, _) = setup(&env);
    let vault = VaultContractClient::new(&env, &vault_id);
    let collateral_admin = token::StellarAssetClient::new(&env, &coll);

    let seller = Address::generate(&env);
    collateral_admin.mint(&seller, &400);
    vault.deposit(&seller, &400);

    let b1 = Address::generate(&env);
    let mut alloc = Vec::new(&env);
    alloc.push_back((b1, 500i128)); // 500 > 400 collateral
    vault.pay_allocations(&1u32, &alloc);
}
