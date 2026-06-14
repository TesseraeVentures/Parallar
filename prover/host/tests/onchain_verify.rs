//! Keystone validation — a REAL Parallar Groth16 proof verifies through the ACTUAL on-chain
//! RISC Zero groth16-verifier (BN254 + Poseidon Soroban host fns, parameters.json v3.0.0).
//!
//! This closes R12's #1 risk: that the host-produced 260-byte **selector-wrapped** seal +
//! image_id + **sha256(journal)** are exactly what an on-chain verifier accepts — the same
//! inputs `settlement.settle()` forwards to the verifier router. We deploy the real verifier
//! (the commit-pinned Nethermind contract, via `contractimport!`) and verify the committed
//! fixture proof produced by the `groth16_proof_generates_and_verifies` proving spike.

use parallar_prover_host::{ProofArtifact, GROTH16_SELECTOR};
use soroban_sdk::{Bytes, BytesN, Env};

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
