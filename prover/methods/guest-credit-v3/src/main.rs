//! The RISC Zero settle_credit_v3 guest (ATTESTED + RECORD-DATE, G4): read the witness, VERIFY the
//! issuer's Ed25519 signature over (epoch ‖ snapshot ‖ payments) — so the per-epoch record-date
//! holder set is attested, not fixed at issuance — run the published rule, and commit the 116-byte
//! generic journal. Any rejection (unattested holder set/payments, cross-epoch replay, or a
//! fully-paid epoch) panics, so no proof can exist.

use risc0_zkvm::guest::env;
use settle_credit_v3::{settle, Inputs};

fn main() {
    let inputs: Inputs = env::read();
    let (_allocations, journal) = settle(&inputs).expect("settlement rules rejected the inputs");
    env::commit_slice(&journal.to_bytes());
}
