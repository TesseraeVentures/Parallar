//! Parallar prover host — the soroban-aware encoding bridge between the on-chain contracts
//! and the RISC Zero guest.
//!
//! The guest cannot run Soroban XDR, so the host produces the canonical `Address::to_xdr`
//! bytes the guest folds into `config_hash` / `allocation_root`. The host cannot supply false
//! bytes: wrong config-Address XDR changes `instrument_id`; a wrong payee changes the
//! committed `position_root`. The parity tests below assert the guest's encodings equal the
//! real factory/settlement encodings byte-for-byte.
//!
//! (The `prove` / `submit` / `history_builder` CLI is added with the zkVM wrapper.)

use soroban_sdk::{xdr::ToXdr, Address, Env, Symbol};

/// Canonical Address XDR — exactly what the contracts fold via `addr.to_xdr(env)`.
pub fn address_xdr(env: &Env, addr: &Address) -> Vec<u8> {
    addr.clone().to_xdr(env).iter().collect()
}

/// Canonical Symbol XDR — what the factory folds for `type_id` in `derive_instrument_id`.
pub fn symbol_xdr(env: &Env, s: &Symbol) -> Vec<u8> {
    s.clone().to_xdr(env).iter().collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use parallar_factory::{derive_instrument_id, hash_config, InstrumentConfig};
    use parallar_settlement::hash_allocations;
    use settle_credit_v1::{allocation_root, config_hash, Allocation, ConfigFields};
    use soroban_sdk::{testutils::Address as _, vec as svec, BytesN, Symbol};

    /// The guest's flat `config_hash` must equal the factory's, over the same config.
    #[test]
    fn guest_config_hash_matches_factory_byte_for_byte() {
        let env = Env::default();
        let reference = Address::generate(&env);
        let collateral = Address::generate(&env);
        let config = InstrumentConfig {
            reference_asset: reference.clone(),
            terms_hash: BytesN::from_array(&env, &[0x11; 32]),
            schedule_root: BytesN::from_array(&env, &[0x22; 32]),
            snapshot_root: BytesN::from_array(&env, &[0x33; 32]),
            collateral_token: collateral.clone(),
            premium_bps: 200,
            epoch_deadlines: svec![&env, (1u32, 500u64), (2u32, 1000u64)],
        };
        let contract_ch = hash_config(&env, &config);

        let guest_cf = ConfigFields {
            reference_asset_xdr: address_xdr(&env, &reference),
            terms_hash: [0x11; 32],
            schedule_root: [0x22; 32],
            snapshot_root: [0x33; 32],
            collateral_token_xdr: address_xdr(&env, &collateral),
            premium_bps: 200,
            epoch_deadlines: vec![(1, 500), (2, 1000)],
        };
        assert_eq!(contract_ch.to_array(), config_hash(&guest_cf));
    }

    /// The guest's `instrument_id` derivation must equal the factory's.
    #[test]
    fn guest_instrument_id_matches_factory() {
        let env = Env::default();
        let type_id = Symbol::new(&env, "credit_v1");
        let ch = BytesN::from_array(&env, &[0x44; 32]);
        let contract_id = derive_instrument_id(&env, &type_id, 1, &ch);
        let guest_id = derive_instrument_id_guest(&symbol_xdr(&env, &type_id), 1, &[0x44; 32]);
        assert_eq!(contract_id.to_array(), guest_id);
    }

    // local alias to disambiguate the two `derive_instrument_id` in scope
    fn derive_instrument_id_guest(type_id_xdr: &[u8], rv: u32, ch: &[u8; 32]) -> [u8; 32] {
        settle_credit_v1::derive_instrument_id(type_id_xdr, rv, ch)
    }

    /// The guest's `allocation_root` must equal the settlement contract's `hash_allocations`,
    /// using real Stellar Address XDR — this is the M3 bridge, verified.
    #[test]
    fn guest_allocation_root_matches_settlement_byte_for_byte() {
        let env = Env::default();
        let b1 = Address::generate(&env);
        let b2 = Address::generate(&env);
        let allocs = svec![&env, (b1.clone(), 300i128), (b2.clone(), 200i128)];
        let contract_root = hash_allocations(&env, &allocs);

        let guest_allocs = vec![
            Allocation { buyer: address_xdr(&env, &b1), amount: 300 },
            Allocation { buyer: address_xdr(&env, &b2), amount: 200 },
        ];
        assert_eq!(contract_root.to_array(), allocation_root(&guest_allocs));
    }

    /// The zkVM guest, run in the executor (no proof), must commit the SAME 116-byte journal
    /// as native `settle()` — confirming the cross-compiled circuit matches the reference logic.
    #[test]
    fn zkvm_guest_journal_matches_native_settle() {
        use parallar_methods::SETTLE_CREDIT_V1_GUEST_ELF;
        use risc0_zkvm::{default_executor, ExecutorEnv};
        use settle_credit_v1::{
            position_root, settle, snapshot_root, terms_hash, Holder, Inputs, Position, Terms,
        };

        // valid full-miss settlement: 1 holder owed 1000, unpaid; buyer cover 800
        let holders = vec![Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false }];
        let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
        let terms = Terms { coupon_rate_bps: 1000 };
        let config = ConfigFields {
            reference_asset_xdr: vec![0xAA, 1, 2, 3],
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&holders),
            collateral_token_xdr: vec![0xBB, 4, 5, 6],
            premium_bps: 200,
            epoch_deadlines: vec![(1u32, 500u64)],
        };
        let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
        let proot = position_root(&positions);
        let instrument_id = settle_credit_v1::derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
        let inputs = Inputs {
            type_id_xdr,
            rules_version: 1,
            config,
            instrument_id,
            epoch: 1,
            deadline: 500,
            terms,
            collateral: 1000,
            snapshot: holders,
            payments: vec![],
            positions,
            position_root: proot,
        };

        let native_journal = settle(&inputs).unwrap().1.to_bytes();

        let exec_env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let session = default_executor().execute(exec_env, SETTLE_CREDIT_V1_GUEST_ELF).unwrap();

        assert_eq!(session.journal.bytes.as_slice(), &native_journal[..]);
    }

    /// SPIKE (slow, needs Docker): generate a REAL Groth16 proof via the STARK→SNARK wrap
    /// (x86 image under Rosetta), verify it against the image_id, confirm the journal, and
    /// print the (seal, image_id, journal_digest) the on-chain verifier consumes.
    /// Run explicitly: `cargo test -p parallar-prover-host -- --ignored --nocapture groth16`.
    #[test]
    #[ignore = "slow: real Groth16 proof via Rosetta x86 Docker"]
    fn groth16_proof_generates_and_verifies() {
        use parallar_methods::{SETTLE_CREDIT_V1_GUEST_ELF, SETTLE_CREDIT_V1_GUEST_ID};
        use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};
        use settle_credit_v1::{
            position_root, settle, snapshot_root, terms_hash, Holder, Inputs, Position, Terms,
        };
        use sha2::{Digest, Sha256};

        let holders = vec![Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false }];
        let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
        let terms = Terms { coupon_rate_bps: 1000 };
        let config = ConfigFields {
            reference_asset_xdr: vec![0xAA, 1, 2, 3],
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&holders),
            collateral_token_xdr: vec![0xBB, 4, 5, 6],
            premium_bps: 200,
            epoch_deadlines: vec![(1u32, 500u64)],
        };
        let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
        let proot = position_root(&positions);
        let instrument_id = settle_credit_v1::derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
        let inputs = Inputs {
            type_id_xdr,
            rules_version: 1,
            config,
            instrument_id,
            epoch: 1,
            deadline: 500,
            terms,
            collateral: 1000,
            snapshot: holders,
            payments: vec![],
            positions,
            position_root: proot,
        };
        let native_journal = settle(&inputs).unwrap().1.to_bytes();

        let env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let receipt = default_prover()
            .prove_with_opts(env, SETTLE_CREDIT_V1_GUEST_ELF, &ProverOpts::groth16())
            .unwrap()
            .receipt;

        receipt.verify(SETTLE_CREDIT_V1_GUEST_ID).unwrap();
        assert_eq!(receipt.journal.bytes.as_slice(), &native_journal[..]);

        // the values the on-chain verifier consumes. The raw Groth16 seal comes straight off
        // the receipt; wrapping it with the router selector for Stellar is the verifier-wiring step.
        let seal = receipt.inner.groth16().expect("groth16 receipt").seal.clone();
        let image_id = risc0_zkvm::sha::Digest::from(SETTLE_CREDIT_V1_GUEST_ID);
        let journal_digest: [u8; 32] = Sha256::digest(&receipt.journal.bytes).into();
        println!(
            "GROTH16 OK | seal={}B image_id=0x{} journal_digest=0x{}",
            seal.len(),
            hex::encode(image_id.as_bytes()),
            hex::encode(journal_digest)
        );
    }
}
