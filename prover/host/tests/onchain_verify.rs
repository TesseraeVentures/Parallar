//! Keystone validation — a REAL Parallar Groth16 proof verifies through the ACTUAL on-chain
//! RISC Zero groth16-verifier (BN254 + Poseidon Soroban host fns, parameters.json v3.0.0).
//!
//! This closes R12's #1 risk: that the host-produced 260-byte **selector-wrapped** seal +
//! image_id + **sha256(journal)** are exactly what an on-chain verifier accepts — the same
//! inputs `settlement.settle()` forwards to the verifier router. We deploy the real verifier
//! (the commit-pinned Nethermind contract, via `contractimport!`) and verify the committed
//! fixture proof produced by the `groth16_proof_generates_and_verifies` proving spike.

use parallar_prover_host::{ProofArtifact, GROTH16_SELECTOR};
use parallar_settlement::{SettlementContract, SettlementContractClient};
use parallar_vault::{VaultContract, VaultContractClient};
use settle_credit_v1::{commitment, Inputs};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Bytes, BytesN, Env, Vec as SVec,
};

// The real Nethermind groth16-verifier wasm, vendored as a fixture (16 KB) so this test is
// fresh-clone runnable (external/ is gitignored). Built from the commit-pinned verifier
// (STATUS: e8ff6ea, parameters.json v3.0.0) via:
//   (cd external/stellar-risc0-verifier && stellar contract build --manifest-path contracts/groth16-verifier/Cargo.toml)
//   cp external/.../release/groth16_verifier.wasm prover/host/tests/fixtures/
mod verifier_wasm {
    soroban_sdk::contractimport!(file = "tests/fixtures/groth16_verifier.wasm");
}

fn fixture() -> ProofArtifact {
    serde_json::from_str(include_str!("fixtures/real_proof.json"))
        .expect("parse tests/fixtures/real_proof.json (run the groth16 proving spike to produce it)")
}

fn b32(h: &str) -> [u8; 32] {
    <[u8; 32]>::try_from(hex::decode(h).unwrap().as_slice()).unwrap()
}

/// The fixture seal verifies on-chain, through the values settlement forwards to the verifier.
#[test]
fn real_parallar_proof_verifies_on_chain() {
    let a = fixture();
    let seal = hex::decode(&a.seal).unwrap();
    assert_eq!(seal.len(), 260, "selector-wrapped seal is 260 bytes");
    assert_eq!(&seal[0..4], &GROTH16_SELECTOR, "router selector prefix");

    let env = Env::default();
    let v = verifier_wasm::Client::new(&env, &env.register(verifier_wasm::WASM, ()));

    let journal = Bytes::from_slice(&env, &hex::decode(&a.journal).unwrap());
    let journal_digest: BytesN<32> = env.crypto().sha256(&journal).into();
    // the contract recomputes sha256(journal) itself; confirm it equals the artifact's digest
    assert_eq!(
        hex::encode(journal_digest.to_array()),
        a.journal_digest,
        "sha256(journal) == artifact journal_digest"
    );

    // verify() returns () on success and traps on a bad proof — this is settlement's gate
    let ok = v.verify(
        &Bytes::from_slice(&env, &seal),
        &BytesN::from_array(&env, &b32(&a.image_id)),
        &journal_digest,
    );
    assert_eq!(ok, (), "real Parallar proof verifies through the on-chain groth16-verifier");
}

/// A one-byte corruption of the seal is rejected on-chain (no false settlement can pass).
#[test]
#[should_panic]
fn tampered_seal_rejected_on_chain() {
    let a = fixture();
    let mut seal = hex::decode(&a.seal).unwrap();
    let n = seal.len();
    seal[n - 1] ^= 0xFF; // flip the last byte of the proof's C point

    let env = Env::default();
    let v = verifier_wasm::Client::new(&env, &env.register(verifier_wasm::WASM, ()));
    let journal = Bytes::from_slice(&env, &hex::decode(&a.journal).unwrap());
    let journal_digest: BytesN<32> = env.crypto().sha256(&journal).into();
    v.verify(
        &Bytes::from_slice(&env, &seal),
        &BytesN::from_array(&env, &b32(&a.image_id)),
        &journal_digest,
    ); // invalid proof -> trap
}

fn witness() -> Inputs {
    serde_json::from_str(include_str!("fixtures/witness.json")).expect("parse witness.json fixture")
}

/// The COMPLETE on-chain path with the real proof: deploy the verifier + vault + settlement,
/// reconstruct the exact committed position so the vault's `position_root` matches the journal,
/// then `settle()` — the proof verifies through the verifier endpoint, every binding passes,
/// and the buyer is paid from the vault. The whole pipeline, end to end, deterministic, no
/// testnet. (Settlement points at the groth16-verifier directly; the production router is a
/// thin selector-dispatcher with the identical `verify(seal, image_id, journal)` interface.)
#[test]
fn full_settlement_pays_out_with_real_proof() {
    let a = fixture(); // real_proof.json — the proven artifact
    let w = witness(); // witness.json — the exact Inputs the proof was generated from
    let env = Env::default();
    env.mock_all_auths();

    // the real on-chain verifier (the endpoint settlement calls to check the proof)
    let verifier = env.register(verifier_wasm::WASM, ());

    // collateral SAC
    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let coll = sac.address();
    let coll_admin = token::StellarAssetClient::new(&env, &coll);
    let coll_tok = token::Client::new(&env, &coll);

    // vault + settlement, cross-bound; settlement pinned to the proof's image_id + verifier
    let vault_id = env.register(VaultContract, ());
    let settlement_id = env.register(SettlementContract, ());
    let vault = VaultContractClient::new(&env, &vault_id);
    let settlement = SettlementContractClient::new(&env, &settlement_id);

    vault.init(&settlement_id, &coll);
    let image_id = BytesN::from_array(&env, &b32(&a.image_id));
    let instrument_id = BytesN::from_array(&env, &w.instrument_id);
    let mut deadlines = SVec::new(&env);
    deadlines.push_back((w.epoch, w.deadline));
    settlement.init(&image_id, &instrument_id, &vault_id, &deadlines, &verifier);

    // reconstruct the vault state so position_root matches the journal: fund collateral, then
    // buy_protection with the SAME Poseidon commitment the guest folded (buyer, cover, salt)
    let seller = Address::generate(&env);
    coll_admin.mint(&seller, &w.collateral);
    vault.deposit(&seller, &w.collateral);

    let pos = &w.positions[0];
    let buyer = Address::from_string(&soroban_sdk::String::from_str(&env, &a.allocations[0].payee));
    let commit = BytesN::from_array(&env, &commitment(&pos.buyer, pos.cover, &pos.salt));
    vault.buy_protection(&buyer, &commit, &pos.cover);

    let journal = hex::decode(&a.journal).unwrap();
    assert_eq!(
        &vault.position_root().to_array()[..],
        &journal[44..76],
        "reconstructed vault position_root == the journal's committed position_root"
    );

    // settle past the deadline with the REAL proof
    env.ledger().with_mut(|l| l.timestamp = w.deadline + 1);
    let mut allocs = SVec::new(&env);
    allocs.push_back((buyer.clone(), a.allocations[0].amount));

    assert_eq!(coll_tok.balance(&buyer), 0, "buyer unpaid before settlement");
    settlement.settle(
        &Bytes::from_slice(&env, &hex::decode(&a.seal).unwrap()),
        &Bytes::from_slice(&env, &journal),
        &allocs,
    );

    // the proof verified on-chain, every binding passed, the buyer was paid from the vault
    assert_eq!(coll_tok.balance(&buyer), a.allocations[0].amount, "buyer paid the proven payout");
    assert!(settlement.is_settled(&w.epoch), "epoch marked settled");
    assert_eq!(vault.total_collateral(), w.collateral - a.allocations[0].amount, "collateral reduced by payout");
}
