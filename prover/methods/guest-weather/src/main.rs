//! The RISC Zero settle_weather_v1 guest: read the witness, run the published parametric
//! rule, and commit the 116-byte generic journal. Any rule rejection (incl. a non-breaching
//! index -> NoBreach) panics, so no proof can exist for an invalid or non-triggering
//! settlement — the same structural guarantee credit_v1 gives, on a different instrument.

use risc0_zkvm::guest::env;
use settle_weather_v1::{settle, Inputs};

fn main() {
    let inputs: Inputs = env::read();
    let (_allocations, journal) = settle(&inputs).expect("settlement rules rejected the inputs");
    env::commit_slice(&journal.to_bytes());
}
