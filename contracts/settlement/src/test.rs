#![cfg(test)]
extern crate std;

use super::*;
use parallar_vault::{VaultContract, VaultContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Bytes, BytesN, Env, Vec,
};

const DEADLINE: u64 = 500;
const EPOCH: u32 = 1;

struct Ctx {
    settlement_id: Address,
    vault_id: Address,
    coll: Address,
    instrument_id: BytesN<32>,
}

/// Deploys a cross-bound vault+settlement, funds the vault (1000), and buys 800 cover so
/// `position_root` is set. Ledger time is 1000 (past the epoch-1 deadline of 500).
fn setup(env: &Env) -> Ctx {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1000);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let collateral_admin = token::StellarAssetClient::new(env, &sac.address());

    let vault_id = env.register(VaultContract, ());
    let settlement_id = env.register(SettlementContract, ());
    let vault = VaultContractClient::new(env, &vault_id);
    let settlement = SettlementContractClient::new(env, &settlement_id);

    vault.init(&settlement_id, &sac.address());

    let instrument_id = BytesN::from_array(env, &[5u8; 32]);
    let mut deadlines = Vec::new(env);
    deadlines.push_back((EPOCH, DEADLINE));
    settlement.init(&BytesN::from_array(env, &[9u8; 32]), &instrument_id, &vault_id, &deadlines);

    let seller = Address::generate(env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);
    let buyer = Address::generate(env);
    vault.buy_protection(&buyer, &BytesN::from_array(env, &[7u8; 32]), &800);

    Ctx { settlement_id, vault_id, coll: sac.address(), instrument_id }
}

#[allow(clippy::too_many_arguments)]
fn make_journal(
    env: &Env,
    instrument_id: &BytesN<32>,
    epoch: u32,
    deadline: u64,
    position_root: &BytesN<32>,
    allocation_root: &BytesN<32>,
    total_payout: u64,
) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &instrument_id.to_array()));
    b.append(&Bytes::from_array(env, &epoch.to_be_bytes()));
    b.append(&Bytes::from_array(env, &deadline.to_be_bytes()));
    b.append(&Bytes::from_array(env, &position_root.to_array()));
    b.append(&Bytes::from_array(env, &allocation_root.to_array()));
    b.append(&Bytes::from_array(env, &total_payout.to_be_bytes()));
    b
}

fn stub_proof(env: &Env) -> Bytes {
    Bytes::from_array(env, &[0u8; 4])
}

fn one_alloc(env: &Env, to: &Address, amt: i128) -> Vec<(Address, i128)> {
    let mut a = Vec::new(env);
    a.push_back((to.clone(), amt));
    a
}

#[test]
fn settle_verifies_bindings_and_pays_through_vault() {
    let env = Env::default();
    let c = setup(&env);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);
    let collateral = token::Client::new(&env, &c.coll);

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);

    assert_eq!(collateral.balance(&payee), 300, "buyer paid via vault");
    assert!(settlement.is_settled(&EPOCH));
    assert_eq!(vault.total_collateral(), 700, "collateral reduced by payout");
}

#[test]
#[should_panic(expected = "epoch already settled")]
fn replay_same_epoch_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);
    settlement.settle(&stub_proof(&env), &journal, &alloc); // replay -> revert
}

#[test]
#[should_panic(expected = "stale position_root")]
fn stale_position_root_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let wrong_root = BytesN::from_array(&env, &[0xAAu8; 32]);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &wrong_root, &alloc_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);
}

#[test]
#[should_panic(expected = "instrument_id mismatch")]
fn wrong_instrument_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let wrong_instrument = BytesN::from_array(&env, &[0xBBu8; 32]);
    let journal = make_journal(&env, &wrong_instrument, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);
}

#[test]
#[should_panic(expected = "allocation_root mismatch")]
fn tampered_allocation_root_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let bad_root = BytesN::from_array(&env, &[0xCCu8; 32]);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &bad_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);
}

#[test]
#[should_panic(expected = "deadline not passed")]
fn settling_before_deadline_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let settlement = SettlementContractClient::new(&env, &c.settlement_id);
    env.ledger().with_mut(|l| l.timestamp = 100); // before DEADLINE=500

    let payee = Address::generate(&env);
    let alloc = one_alloc(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);

    settlement.settle(&stub_proof(&env), &journal, &alloc);
}
