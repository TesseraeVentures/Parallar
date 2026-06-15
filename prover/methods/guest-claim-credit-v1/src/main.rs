//! The RISC Zero claim_credit_v1 guest (escape hatch, G2): read the claim witness, prove the
//! claimant's own allocation against the committed position_root, and commit the single-allocation
//! 116-byte journal. Any rejection (unattested holder, tampered root, non-default) panics, so no
//! claim proof can exist for an invalid claim.

use risc0_zkvm::guest::env;
use claim_credit_v1::{claim, ClaimInputs};

fn main() {
    let inputs: ClaimInputs = env::read();
    let (_alloc, journal) = claim(&inputs).expect("claim rules rejected the inputs");
    env::commit_slice(&journal.to_bytes());
}
