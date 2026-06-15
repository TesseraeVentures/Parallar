//! The RISC Zero settle_credit_v2 guest: read the witness, VERIFY the issuer's Ed25519 signature
//! over the payment snapshot (G1), run the published rule, and commit the 116-byte generic
//! journal. Any rejection — an unattested/tampered snapshot (AttestationInvalid), or a fully-paid
//! epoch (NoDefault) — panics, so no proof can exist. The attestation check happens INSIDE the
//! circuit: the proof itself certifies the data was signed by the committed issuer key.

use risc0_zkvm::guest::env;
use settle_credit_v2::{settle, Inputs};

fn main() {
    let inputs: Inputs = env::read();
    let (_allocations, journal) = settle(&inputs).expect("settlement rules rejected the inputs");
    env::commit_slice(&journal.to_bytes());
}
