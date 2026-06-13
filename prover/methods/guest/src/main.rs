//! The RISC Zero settle_credit_v1 guest: read the witness, run the published rules, and
//! commit the 116-byte generic journal. Any rule rejection (incl. a fully-paid epoch ->
//! NoDefault) panics, so no proof can exist for an invalid or non-defaulting settlement.

use risc0_zkvm::guest::env;
use settle_credit_v1::{settle, Inputs};

fn main() {
    let inputs: Inputs = env::read();
    let (_allocations, journal) = settle(&inputs).expect("settlement rules rejected the inputs");
    env::commit_slice(&journal.to_bytes());
}
