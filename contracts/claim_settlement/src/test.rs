#![cfg(test)]
extern crate std;

use super::*;
use parallar_vault::{VaultContract, VaultContractClient};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token, Address, Bytes, BytesN, Env, Vec,
};

const DEADLINE: u64 = 500;
const GRACE: u64 = 100; // direct claims open at t > 600
const EPOCH: u32 = 1;

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
    settlement_id: Address,
    vault_id: Address,
    coll: Address,
    instrument_id: BytesN<32>,
}

fn setup(env: &Env) -> Ctx {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1000);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let collateral_admin = token::StellarAssetClient::new(env, &sac.address());

    let vault_id = env.register(VaultContract, ());
    let settlement_id = env.register(ClaimableSettlement, ());
    let vault = VaultContractClient::new(env, &vault_id);
    let settlement = ClaimableSettlementClient::new(env, &settlement_id);
    vault.init(&settlement_id, &sac.address());

    let router_id = env.register(MockRouter, ());
    let instrument_id = BytesN::from_array(env, &[5u8; 32]);
    let mut deadlines = Vec::new(env);
    deadlines.push_back((EPOCH, DEADLINE));
    settlement.init(
        &BytesN::from_array(env, &[9u8; 32]), // settle image_id
        &BytesN::from_array(env, &[8u8; 32]), // claim image_id
        &instrument_id,
        &vault_id,
        &deadlines,
        &router_id,
        &GRACE,
    );

    let seller = Address::generate(env);
    collateral_admin.mint(&seller, &1_000);
    vault.deposit(&seller, &1_000);
    let buyer = Address::generate(env);
    vault.buy_protection(&buyer, &BytesN::from_array(env, &[7u8; 32]), &800);

    Ctx { settlement_id, vault_id, coll: sac.address(), instrument_id }
}

#[allow(clippy::too_many_arguments)]
fn make_journal(env: &Env, instrument_id: &BytesN<32>, epoch: u32, deadline: u64, position_root: &BytesN<32>, allocation_root: &BytesN<32>, total_payout: u64) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &instrument_id.to_array()));
    b.append(&Bytes::from_array(env, &epoch.to_be_bytes()));
    b.append(&Bytes::from_array(env, &deadline.to_be_bytes()));
    b.append(&Bytes::from_array(env, &position_root.to_array()));
    b.append(&Bytes::from_array(env, &allocation_root.to_array()));
    b.append(&Bytes::from_array(env, &total_payout.to_be_bytes()));
    b
}

fn stub(env: &Env) -> Bytes {
    Bytes::from_array(env, &[0u8; 4])
}
fn alloc_vec(env: &Env, to: &Address, amt: i128) -> Vec<(Address, i128)> {
    let mut a = Vec::new(env);
    a.push_back((to.clone(), amt));
    a
}

/// Build the single-allocation journal a claim proof would commit for (claimant, amount).
fn claim_journal(env: &Env, c: &Ctx, claimant: &Address, amount: i128) -> Bytes {
    let vault = VaultContractClient::new(env, &c.vault_id);
    let root = hash_allocations(env, &alloc_vec(env, claimant, amount));
    make_journal(env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &root, amount as u64)
}

#[test]
fn claim_direct_pays_single_buyer_after_grace() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let collateral = token::Client::new(&env, &c.coll);
    env.ledger().with_mut(|l| l.timestamp = 700); // > deadline + grace (600)

    let claimant = Address::generate(&env);
    let journal = claim_journal(&env, &c, &claimant, 300);
    settlement.claim_direct(&stub(&env), &journal, &claimant, &300);

    assert_eq!(collateral.balance(&claimant), 300, "the claimant was paid directly");
    assert!(settlement.has_claimed(&EPOCH, &claimant));
    assert!(!settlement.is_settled(&EPOCH), "a direct claim does not mark the epoch fully settled");
    assert_eq!(vault.total_collateral(), 700);
}

#[test]
#[should_panic(expected = "keeper-grace window not passed")]
fn claim_before_grace_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    env.ledger().with_mut(|l| l.timestamp = 550); // past deadline (500) but inside grace (<= 600)

    let claimant = Address::generate(&env);
    let journal = claim_journal(&env, &c, &claimant, 300);
    settlement.claim_direct(&stub(&env), &journal, &claimant, &300);
}

#[test]
#[should_panic(expected = "already claimed")]
fn double_claim_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let claimant = Address::generate(&env);
    let journal = claim_journal(&env, &c, &claimant, 300);
    settlement.claim_direct(&stub(&env), &journal, &claimant, &300);
    settlement.claim_direct(&stub(&env), &journal, &claimant, &300); // second claim -> revert
}

#[test]
#[should_panic(expected = "already fully settled")]
fn claim_after_full_settle_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let payee = Address::generate(&env);
    let alloc = alloc_vec(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let full_journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);
    settlement.settle(&stub(&env), &full_journal, &alloc); // keeper settles the whole epoch

    let claimant = Address::generate(&env);
    let cj = claim_journal(&env, &c, &claimant, 100);
    settlement.claim_direct(&stub(&env), &cj, &claimant, &100); // -> revert: already settled
}

#[test]
#[should_panic(expected = "has direct claims")]
fn full_settle_after_claim_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let claimant = Address::generate(&env);
    let cj = claim_journal(&env, &c, &claimant, 300);
    settlement.claim_direct(&stub(&env), &cj, &claimant, &300);

    let payee = Address::generate(&env);
    let alloc = alloc_vec(&env, &payee, 200);
    let alloc_root = hash_allocations(&env, &alloc);
    let full_journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 200);
    settlement.settle(&stub(&env), &full_journal, &alloc); // -> revert: epoch has direct claims
}

#[test]
fn keeper_full_settle_still_works() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let collateral = token::Client::new(&env, &c.coll);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let payee = Address::generate(&env);
    let alloc = alloc_vec(&env, &payee, 300);
    let alloc_root = hash_allocations(&env, &alloc);
    let journal = make_journal(&env, &c.instrument_id, EPOCH, DEADLINE, &vault.position_root(), &alloc_root, 300);
    settlement.settle(&stub(&env), &journal, &alloc);

    assert_eq!(collateral.balance(&payee), 300);
    assert!(settlement.is_settled(&EPOCH));
}

#[test]
#[should_panic]
fn forged_claim_proof_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let claimant = Address::generate(&env);
    let journal = claim_journal(&env, &c, &claimant, 300);
    let forged = Bytes::from_array(&env, &[0xFFu8; 4]); // sentinel the mock router rejects
    settlement.claim_direct(&forged, &journal, &claimant, &300);
}

#[test]
fn two_buyers_each_claim_their_share() {
    let env = Env::default();
    let c = setup(&env);
    let settlement = ClaimableSettlementClient::new(&env, &c.settlement_id);
    let vault = VaultContractClient::new(&env, &c.vault_id);
    let collateral = token::Client::new(&env, &c.coll);
    env.ledger().with_mut(|l| l.timestamp = 700);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    settlement.claim_direct(&stub(&env), &claim_journal(&env, &c, &a, 300), &a, &300);
    settlement.claim_direct(&stub(&env), &claim_journal(&env, &c, &b, 200), &b, &200);

    assert_eq!(collateral.balance(&a), 300);
    assert_eq!(collateral.balance(&b), 200);
    assert_eq!(vault.total_collateral(), 500, "cumulative claims bounded by the vault");
}
