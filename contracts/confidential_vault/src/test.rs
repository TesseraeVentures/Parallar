#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, token, Address, Bytes, BytesN, Env, Vec,
};

// Mock RISC Zero verifier: accepts any seal except the 4-byte 0xFF marker (a "forged" proof).
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
    settlement: Address,
    admin: Address,
}

fn setup_with(env: &Env, fee_bps: u32) -> Ctx {
    env.mock_all_auths();
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let settlement = Address::generate(env);
    let admin = Address::generate(env);
    let router = env.register(MockRouter, ());
    let vault = env.register(ConfidentialVault, ());
    ConfidentialVaultClient::new(env, &vault).init(
        &settlement,
        &sac.address(),
        &admin,
        &BytesN::from_array(env, &[9u8; 32]), // solvency image_id
        &router,
        &fee_bps,
        &b32(env, 0), // initial cover_commitment = commit_total(0, salt0) (computed off-chain)
    );
    Ctx { vault, coll: sac.address(), settlement, admin }
}
fn setup(env: &Env) -> Ctx {
    setup_with(env, 1200) // 12% protocol fee
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
fn bad_seal(env: &Env) -> Bytes {
    Bytes::from_array(env, &[0xFFu8; 4])
}
fn buy_journal(env: &Env, prev: &BytesN<32>, new: &BytesN<32>, pos: &BytesN<32>, collateral: i128) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &prev.to_array()));
    b.append(&Bytes::from_array(env, &new.to_array()));
    b.append(&Bytes::from_array(env, &pos.to_array()));
    b.append(&Bytes::from_array(env, &collateral.to_be_bytes()));
    b // 112 bytes
}
fn withdraw_journal(env: &Env, cc: &BytesN<32>, collateral_after: i128) -> Bytes {
    let mut b = Bytes::new(env);
    b.append(&Bytes::from_array(env, &cc.to_array()));
    b.append(&Bytes::from_array(env, &collateral_after.to_be_bytes()));
    b // 48 bytes
}

#[test]
fn confidential_buy_advances_the_hidden_commitment() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    assert_eq!(v.cover_commitment(), b32(&env, 0), "starts at the committed-zero aggregate");

    // a solvency proof advances the aggregate 0 -> [1;32] for some hidden cover, proven <= 600 <= reserve
    let buyer = Address::generate(&env);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);

    assert_eq!(v.cover_commitment(), b32(&env, 1), "advanced to the new hidden aggregate");
    assert_ne!(v.position_root(), b32(&env, 0), "position root folded the buyer's commitment");
    assert_eq!(v.total_collateral(), 10_000, "reserve unchanged by a purchase");
}

#[test]
#[should_panic]
fn forged_solvency_proof_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &bad_seal(&env), &j, &0); // forged proof -> verifier traps
}

#[test]
#[should_panic]
fn buy_from_wrong_prev_commitment_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    // prev = [5;32] but the vault stores [0;32] -> the proof does not advance from our aggregate
    let j = buy_journal(&env, &b32(&env, 5), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);
}

#[test]
#[should_panic]
fn buy_with_collateral_exceeding_reserve_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 1_000);
    v.deposit(&seller, &1_000);
    let buyer = Address::generate(&env);
    // the proof's collateral bound (1_001) exceeds the real reserve (1_000) -> solvency not assured
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 1_001);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);
}

#[test]
#[should_panic]
fn wrong_journal_length_reverts() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    let buyer = Address::generate(&env);
    // a 48-byte withdraw journal fed to the buy entrypoint -> bad length, cannot be confused
    let j = withdraw_journal(&env, &b32(&env, 0), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &0);
}

#[test]
fn premium_distributes_to_underwriters_minus_protocol_cut() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);

    let buyer = Address::generate(&env);
    mint(&env, &c.coll, &buyer, 100);
    let j = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &j, &100); // declared premium 100

    assert_eq!(v.protocol_fee_accrued(), 12, "12% protocol cut");
    assert_eq!(v.pending_premium(&seller), 88, "rest to the single underwriter");
    assert_eq!(tok.balance(&buyer), 0, "buyer paid the declared premium");
    v.claim_premium(&seller);
    assert_eq!(tok.balance(&seller), 88);
}

#[test]
fn proven_withdrawal_releases_collateral() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);

    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    // buy advances the aggregate to [1;32]
    let buyer = Address::generate(&env);
    let bj = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &bj, &0);

    // withdraw 3_000: proof attests the hidden aggregate <= collateral_after (7_000)
    let wj = withdraw_journal(&env, &b32(&env, 1), 7_000);
    v.withdraw_proven(&seller, &3_000, &good_seal(&env), &wj);
    assert_eq!(tok.balance(&seller), 3_000);
    assert_eq!(v.total_collateral(), 7_000);
    assert_eq!(v.seller_balance(&seller), 7_000);
}

#[test]
#[should_panic]
fn withdrawal_collateral_after_must_match_real_reserve() {
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);
    // collateral_after in the proof (9_000) != real total - amount (10_000 - 3_000 = 7_000)
    let wj = withdraw_journal(&env, &b32(&env, 0), 9_000);
    v.withdraw_proven(&seller, &3_000, &good_seal(&env), &wj);
}

#[test]
fn pay_allocations_requires_the_bound_settlement_auth() {
    // Law #1: the reserve moves ONLY via the bound settlement, never via a solvency proof or admin.
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);

    let payee = Address::generate(&env);
    let mut alloc = Vec::new(&env);
    alloc.push_back((payee, 3_000i128));
    env.set_auths(&[]);
    let res = v.try_pay_allocations(&1u32, &alloc);
    assert!(res.is_err(), "pay_allocations must trap without the bound settlement's auth");
    assert_eq!(v.total_collateral(), 10_000, "reserve untouched");
}

#[test]
fn settlement_pays_from_reserve() {
    // the bound settlement (mock_all_auths) can pay; the hidden cover commitment is untouched.
    let env = Env::default();
    let c = setup(&env);
    let v = ConfidentialVaultClient::new(&env, &c.vault);
    let tok = token::Client::new(&env, &c.coll);
    let seller = Address::generate(&env);
    mint(&env, &c.coll, &seller, 10_000);
    v.deposit(&seller, &10_000);

    let buyer = Address::generate(&env);
    let bj = buy_journal(&env, &b32(&env, 0), &b32(&env, 1), &b32(&env, 7), 600);
    v.buy_protection_proven(&buyer, &good_seal(&env), &bj, &0);

    let payee = Address::generate(&env);
    let mut alloc = Vec::new(&env);
    alloc.push_back((payee.clone(), 600i128));
    v.pay_allocations(&1u32, &alloc); // settlement authorized under mock_all_auths
    assert_eq!(tok.balance(&payee), 600);
    assert_eq!(v.total_collateral(), 9_400);
    let _ = c.admin;
    let _ = c.settlement;
}
