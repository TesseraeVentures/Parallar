//! Parallar prover host — the soroban-aware encoding bridge between the on-chain contracts
//! and the RISC Zero guest.
//!
//! The guest cannot run Soroban XDR, so the host produces the canonical `Address::to_xdr`
//! bytes the guest folds into `config_hash` / `allocation_root`. The host cannot supply false
//! bytes: wrong config-Address XDR changes `instrument_id`; a wrong payee changes the
//! committed `position_root`. The parity tests below assert the guest's encodings equal the
//! real factory/settlement encodings byte-for-byte.
//!
//! This crate is also the prover host library: `prove_settlement` runs the RISC Zero guest
//! under the real Groth16 prover and packages a submittable [`ProofArtifact`]. The
//! `parallar-prover` binary (src/main.rs) wraps it with `prove` / `submit`. `history-builder`
//! (the qualifying-payment scan, TECH_SPEC §10) is the next addition.

use serde::{Deserialize, Serialize};
use settle_credit_v1::{settle, Allocation, Inputs, Journal};
use sha2::{Digest, Sha256};
use soroban_sdk::{xdr::ToXdr, Address, Env, Symbol};

/// Assembles the guest witness (snapshot + qualifying payments) from observed chain data,
/// per the normative rules in TECH_SPEC §10. See the module for the trust caveats.
pub mod history_builder;

/// Canonical Address XDR — exactly what the contracts fold via `addr.to_xdr(env)`.
pub fn address_xdr(env: &Env, addr: &Address) -> Vec<u8> {
    addr.clone().to_xdr(env).iter().collect()
}

/// Canonical Symbol XDR — what the factory folds for `type_id` in `derive_instrument_id`.
pub fn symbol_xdr(env: &Env, s: &Symbol) -> Vec<u8> {
    s.clone().to_xdr(env).iter().collect()
}

/// The RISC Zero Groth16 verifier selector for risc0 3.0.x / `parameters.json` v3.0.0. The
/// Nethermind `groth16-verifier` build derives `SELECTOR = 73c457ba` (from CONTROL_ROOT
/// a54dc85a… + BN254_CONTROL_ID 04446e66… + the verification key). The router dispatches on
/// `seal[0..4]`, so a submittable seal is `SELECTOR ‖ raw_256B_seal` = 260 bytes. This is
/// pinned to the verifier params — a new param set is a new selector (and a new type entry).
pub const GROTH16_SELECTOR: [u8; 4] = [0x73, 0xc4, 0x57, 0xba];

/// One payout line, ready for `settlement.settle`: the payee as a Stellar strkey (what
/// `stellar contract invoke` consumes) and the amount. Order is load-bearing — it must match
/// the order the guest folded into `allocation_root`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllocationOut {
    pub payee: String, // G… / C… strkey
    pub amount: i128,
}

/// The submittable result of a proof: everything `settlement.settle(proof, journal,
/// allocations)` consumes, plus the values the verifier router reconstructs the claim from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofArtifact {
    /// 260-byte selector-wrapped Groth16 seal (`SELECTOR ‖ A‖B‖C`), hex — the `proof` arg.
    pub seal: String,
    /// 32-byte guest image id (the pinned `SETTLE_CREDIT_V1` id), hex.
    pub image_id: String,
    /// The 116-byte generic journal (§3.4), hex — the `journal` arg.
    pub journal: String,
    /// `sha256(journal)`, hex — the settlement contract recomputes this itself; emitted for
    /// cross-checking against the verifier's expected claim input.
    pub journal_digest: String,
    pub epoch: u32,
    pub total_payout: u64,
    /// The payout set, in guest-fold order — the `allocations` arg to `settle()`.
    pub allocations: Vec<AllocationOut>,
}

/// Prepend the verifier selector to a raw 256-byte Groth16 seal → the 260-byte seal the
/// RISC Zero router expects (it dispatches on `seal[0..4]`). Inverse is dropping the prefix.
pub fn wrap_seal(raw_seal: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(GROTH16_SELECTOR.len() + raw_seal.len());
    s.extend_from_slice(&GROTH16_SELECTOR);
    s.extend_from_slice(raw_seal);
    s
}

/// Decode a canonical Address XDR (the bytes the guest/contract fold) back into a Stellar
/// strkey, so the proof artifact carries submit-ready payees. Inverse of [`address_xdr`].
pub fn address_xdr_to_strkey(xdr: &[u8]) -> anyhow::Result<String> {
    use soroban_sdk::xdr::{Limits, PublicKey, ReadXdr, ScAddress, ScVal, Uint256};
    let scval = ScVal::from_xdr(xdr, Limits::none())
        .map_err(|e| anyhow::anyhow!("payee xdr is not a valid ScVal: {e:?}"))?;
    let addr = match scval {
        ScVal::Address(a) => a,
        other => anyhow::bail!("payee xdr is not an ScVal::Address: {other:?}"),
    };
    let strkey = match addr {
        ScAddress::Account(account_id) => match account_id.0 {
            PublicKey::PublicKeyTypeEd25519(Uint256(k)) => {
                stellar_strkey::Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(k))
                    .to_string()
            }
        },
        ScAddress::Contract(contract_id) => {
            let hash: soroban_sdk::xdr::Hash = contract_id.0;
            stellar_strkey::Strkey::Contract(stellar_strkey::Contract(hash.0)).to_string()
        }
        other => anyhow::bail!("unsupported address type for a payee: {other:?}"),
    };
    Ok(strkey)
}

/// Run the settlement guest under the real Groth16 prover (STARK→SNARK; **needs Docker/x86** —
/// Rosetta x86 on Apple Silicon, ~34 min, TECH_SPEC §2), verify the receipt against the pinned
/// image id, confirm the committed journal equals native `settle()`, and package the
/// submittable [`ProofArtifact`] (seal router-wrapped, payees as strkeys in guest-fold order).
pub fn prove_settlement(inputs: &Inputs) -> anyhow::Result<ProofArtifact> {
    use parallar_methods::{SETTLE_CREDIT_V1_GUEST_ELF, SETTLE_CREDIT_V1_GUEST_ID};
    use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};

    // native reference: the allocations + journal the guest must reproduce
    let (allocs, native_journal): (Vec<Allocation>, Journal) =
        settle(inputs).map_err(|e| anyhow::anyhow!("native settle failed: {e:?}"))?;
    let native_journal_bytes = native_journal.to_bytes();

    let env = ExecutorEnv::builder().write(inputs)?.build()?;
    let receipt = default_prover()
        .prove_with_opts(env, SETTLE_CREDIT_V1_GUEST_ELF, &ProverOpts::groth16())?
        .receipt;

    receipt.verify(SETTLE_CREDIT_V1_GUEST_ID)?;
    anyhow::ensure!(
        receipt.journal.bytes.as_slice() == &native_journal_bytes[..],
        "guest-committed journal != native settle journal"
    );

    let raw_seal = receipt
        .inner
        .groth16()
        .map_err(|e| anyhow::anyhow!("not a groth16 receipt: {e:?}"))?
        .seal
        .clone();
    let image_id = risc0_zkvm::sha::Digest::from(SETTLE_CREDIT_V1_GUEST_ID);
    let journal_digest: [u8; 32] = Sha256::digest(&receipt.journal.bytes).into();

    let allocations = allocs
        .iter()
        .map(|a| {
            Ok(AllocationOut { payee: address_xdr_to_strkey(&a.buyer)?, amount: a.amount })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(ProofArtifact {
        seal: hex::encode(wrap_seal(&raw_seal)),
        image_id: hex::encode(image_id.as_bytes()),
        journal: hex::encode(&receipt.journal.bytes),
        journal_digest: hex::encode(journal_digest),
        epoch: native_journal.epoch,
        total_payout: native_journal.total_payout,
        allocations,
    })
}

/// Instance #2: run the `settle_weather_v1` guest under the real Groth16 prover and package
/// the submittable artifact. The pipeline is identical to [`prove_settlement`] — same host,
/// same `ProofArtifact`, same seal wrapping and payee decoding — pinned to the weather guest's
/// OWN image id. That a second instrument type proves through this unchanged path, into the
/// same generic settlement contract, is the layer thesis made operational.
pub fn prove_weather_settlement(
    inputs: &settle_weather_v1::Inputs,
) -> anyhow::Result<ProofArtifact> {
    use parallar_methods::{SETTLE_WEATHER_V1_GUEST_ELF, SETTLE_WEATHER_V1_GUEST_ID};
    use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};

    let (allocs, native_journal) = settle_weather_v1::settle(inputs)
        .map_err(|e| anyhow::anyhow!("native weather settle failed: {e:?}"))?;
    let native_journal_bytes = native_journal.to_bytes();

    let env = ExecutorEnv::builder().write(inputs)?.build()?;
    let receipt = default_prover()
        .prove_with_opts(env, SETTLE_WEATHER_V1_GUEST_ELF, &ProverOpts::groth16())?
        .receipt;

    receipt.verify(SETTLE_WEATHER_V1_GUEST_ID)?;
    anyhow::ensure!(
        receipt.journal.bytes.as_slice() == &native_journal_bytes[..],
        "guest-committed journal != native weather settle journal"
    );

    let raw_seal = receipt
        .inner
        .groth16()
        .map_err(|e| anyhow::anyhow!("not a groth16 receipt: {e:?}"))?
        .seal
        .clone();
    let image_id = risc0_zkvm::sha::Digest::from(SETTLE_WEATHER_V1_GUEST_ID);
    let journal_digest: [u8; 32] = Sha256::digest(&receipt.journal.bytes).into();

    let allocations = allocs
        .iter()
        .map(|a| Ok(AllocationOut { payee: address_xdr_to_strkey(&a.buyer)?, amount: a.amount }))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(ProofArtifact {
        seal: hex::encode(wrap_seal(&raw_seal)),
        image_id: hex::encode(image_id.as_bytes()),
        journal: hex::encode(&receipt.journal.bytes),
        journal_digest: hex::encode(journal_digest),
        epoch: native_journal.epoch,
        total_payout: native_journal.total_payout,
        allocations,
    })
}

/// Instance #1, ATTESTED (G1): run the `settle_credit_v2` guest under the real Groth16 prover.
/// Identical pipeline to [`prove_settlement`], pinned to credit_v2's own image id. The proof now
/// also certifies, in-circuit, that the payment snapshot was signed by the committed issuer key.
pub fn prove_credit_v2_settlement(
    inputs: &settle_credit_v2::Inputs,
) -> anyhow::Result<ProofArtifact> {
    use parallar_methods::{SETTLE_CREDIT_V2_GUEST_ELF, SETTLE_CREDIT_V2_GUEST_ID};
    use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};

    let (allocs, native_journal) = settle_credit_v2::settle(inputs)
        .map_err(|e| anyhow::anyhow!("native credit_v2 settle failed: {e:?}"))?;
    let native_journal_bytes = native_journal.to_bytes();

    let env = ExecutorEnv::builder().write(inputs)?.build()?;
    let receipt = default_prover()
        .prove_with_opts(env, SETTLE_CREDIT_V2_GUEST_ELF, &ProverOpts::groth16())?
        .receipt;

    receipt.verify(SETTLE_CREDIT_V2_GUEST_ID)?;
    anyhow::ensure!(
        receipt.journal.bytes.as_slice() == &native_journal_bytes[..],
        "guest-committed journal != native credit_v2 settle journal"
    );

    let raw_seal = receipt
        .inner
        .groth16()
        .map_err(|e| anyhow::anyhow!("not a groth16 receipt: {e:?}"))?
        .seal
        .clone();
    let image_id = risc0_zkvm::sha::Digest::from(SETTLE_CREDIT_V2_GUEST_ID);
    let journal_digest: [u8; 32] = Sha256::digest(&receipt.journal.bytes).into();

    let allocations = allocs
        .iter()
        .map(|a| Ok(AllocationOut { payee: address_xdr_to_strkey(&a.buyer)?, amount: a.amount }))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(ProofArtifact {
        seal: hex::encode(wrap_seal(&raw_seal)),
        image_id: hex::encode(image_id.as_bytes()),
        journal: hex::encode(&receipt.journal.bytes),
        journal_digest: hex::encode(journal_digest),
        epoch: native_journal.epoch,
        total_payout: native_journal.total_payout,
        allocations,
    })
}

/// Escape hatch (G2): run the `claim_credit_v1` guest under the real Groth16 prover — a SINGLE
/// buyer's allocation proof, verified against the CLAIM image_id. Same pipeline as the settlement
/// provers, one payout. The claimable settlement variant's `claim_direct` consumes this artifact.
pub fn prove_claim_credit_v1(
    inputs: &claim_credit_v1::ClaimInputs,
) -> anyhow::Result<ProofArtifact> {
    use parallar_methods::{CLAIM_CREDIT_V1_GUEST_ELF, CLAIM_CREDIT_V1_GUEST_ID};
    use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};

    let (alloc, native_journal) = claim_credit_v1::claim(inputs)
        .map_err(|e| anyhow::anyhow!("native claim failed: {e:?}"))?;
    let native_journal_bytes = native_journal.to_bytes();

    let env = ExecutorEnv::builder().write(inputs)?.build()?;
    let receipt = default_prover()
        .prove_with_opts(env, CLAIM_CREDIT_V1_GUEST_ELF, &ProverOpts::groth16())?
        .receipt;

    receipt.verify(CLAIM_CREDIT_V1_GUEST_ID)?;
    anyhow::ensure!(
        receipt.journal.bytes.as_slice() == &native_journal_bytes[..],
        "guest-committed journal != native claim journal"
    );

    let raw_seal = receipt
        .inner
        .groth16()
        .map_err(|e| anyhow::anyhow!("not a groth16 receipt: {e:?}"))?
        .seal
        .clone();
    let image_id = risc0_zkvm::sha::Digest::from(CLAIM_CREDIT_V1_GUEST_ID);
    let journal_digest: [u8; 32] = Sha256::digest(&receipt.journal.bytes).into();
    let allocations = vec![AllocationOut { payee: address_xdr_to_strkey(&alloc.buyer)?, amount: alloc.amount }];

    Ok(ProofArtifact {
        seal: hex::encode(wrap_seal(&raw_seal)),
        image_id: hex::encode(image_id.as_bytes()),
        journal: hex::encode(&receipt.journal.bytes),
        journal_digest: hex::encode(journal_digest),
        epoch: native_journal.epoch,
        total_payout: native_journal.total_payout,
        allocations,
    })
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

    /// A full-miss settlement witness with a REAL buyer Address (so the payee decodes to a
    /// strkey and the journal's allocation_root matches what the settlement contract folds
    /// over real Addresses — the fixture the on-chain verifier integration test needs).
    fn sample_inputs(env: &Env) -> (settle_credit_v1::Inputs, Address) {
        use settle_credit_v1::{position_root, snapshot_root, terms_hash, Holder, Inputs, Position, Terms};
        let buyer = Address::generate(env);
        let holders = vec![Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false }];
        let positions = vec![Position { buyer: address_xdr(env, &buyer), cover: 800, salt: [7; 32] }];
        let terms = Terms { coupon_rate_bps: 1000 };
        let config = ConfigFields {
            reference_asset_xdr: address_xdr(env, &Address::generate(env)),
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&holders),
            collateral_token_xdr: address_xdr(env, &Address::generate(env)),
            premium_bps: 200,
            epoch_deadlines: vec![(1u32, 500u64)],
        };
        let type_id_xdr = symbol_xdr(env, &Symbol::new(env, "credit_v1"));
        let proot = position_root(&positions);
        let instrument_id =
            settle_credit_v1::derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
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
        (inputs, buyer)
    }

    /// `wrap_seal` yields the 260-byte router seal: selector ‖ raw 256-byte proof.
    #[test]
    fn wrap_seal_prepends_selector_to_260_bytes() {
        let raw = [0xABu8; 256];
        let wrapped = wrap_seal(&raw);
        assert_eq!(wrapped.len(), 260);
        assert_eq!(&wrapped[0..4], &GROTH16_SELECTOR);
        assert_eq!(&wrapped[4..], &raw[..]);
    }

    /// A payee's canonical Address XDR decodes back to the strkey `settle()` is invoked with.
    #[test]
    fn address_xdr_to_strkey_round_trips() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let strkey = address_xdr_to_strkey(&address_xdr(&env, &addr)).unwrap();
        let back = Address::from_string(&soroban_sdk::String::from_str(&env, &strkey));
        assert_eq!(addr, back, "xdr -> strkey -> Address round-trips");
    }

    /// The proof artifact serializes and parses back unchanged (the submit handoff contract).
    #[test]
    fn proof_artifact_json_round_trips() {
        let a = ProofArtifact {
            seal: format!("73c457ba{}", "ab".repeat(256)),
            image_id: "00".repeat(32),
            journal: "11".repeat(116),
            journal_digest: "22".repeat(32),
            epoch: 1,
            total_payout: 800,
            allocations: vec![AllocationOut { payee: "GABC".into(), amount: 800 }],
        };
        let back: ProofArtifact = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, back);
    }

    /// The witness JSON (`Inputs`) the `prove` CLI consumes round-trips, and its payee decodes.
    #[test]
    fn witness_json_round_trips_and_payee_decodes() {
        let env = Env::default();
        let (inputs, buyer) = sample_inputs(&env);
        let json = serde_json::to_string_pretty(&inputs).unwrap();
        let back: settle_credit_v1::Inputs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.collateral, 1000);
        assert_eq!(back.positions.len(), 1);
        // the position buyer (== the eventual payee) decodes to the buyer's strkey
        let strkey = address_xdr_to_strkey(&back.positions[0].buyer).unwrap();
        let recovered = Address::from_string(&soroban_sdk::String::from_str(&env, &strkey));
        assert_eq!(buyer, recovered);
    }

    /// SPIKE (slow, needs Docker): generate a REAL Groth16 proof via the STARK→SNARK wrap
    /// (x86 image under Rosetta), package the submittable artifact, assert the seal is the
    /// 260-byte selector-wrapped form, and persist the artifact + witness fixtures (consumed
    /// by the on-chain verifier integration test + the demo).
    /// Run explicitly: `cargo test -p parallar-prover-host -- --ignored --nocapture groth16`.
    #[test]
    #[ignore = "slow: real Groth16 proof via Rosetta x86 Docker"]
    fn groth16_proof_generates_and_verifies() {
        let env = Env::default();
        let (inputs, buyer) = sample_inputs(&env);

        let artifact = prove_settlement(&inputs).expect("prove");

        let seal = hex::decode(&artifact.seal).unwrap();
        assert_eq!(seal.len(), 260, "selector-wrapped seal");
        assert_eq!(&seal[0..4], &GROTH16_SELECTOR);
        assert_eq!(artifact.allocations.len(), 1, "one payout");
        assert_eq!(artifact.allocations[0].amount, 800, "full cover paid");
        let buyer_strkey =
            address_xdr_to_strkey(&address_xdr(&env, &buyer)).unwrap();
        assert_eq!(artifact.allocations[0].payee, buyer_strkey, "payee == buyer");

        // persist fixtures (real Address XDR) for the on-chain verifier test + the demo
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            format!("{dir}/real_proof.json"),
            serde_json::to_string_pretty(&artifact).unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/witness.json"),
            serde_json::to_string_pretty(&inputs).unwrap(),
        )
        .unwrap();
        println!(
            "GROTH16 OK | seal=260B image_id={} payee={} amount={}",
            artifact.image_id, artifact.allocations[0].payee, artifact.allocations[0].amount
        );
    }
}

/// Instance #2 (weather_v1) host parity: the SAME factory derives its instrument_id, the SAME
/// settlement verifies its allocation_root, and the cross-compiled weather circuit commits the
/// same journal as native settle — proving the generic surfaces serve a second instrument type.
#[cfg(test)]
mod weather_test {
    use super::*;
    use parallar_factory::{hash_config, InstrumentConfig};
    use parallar_settlement::hash_allocations;
    use settle_weather_v1::{
        allocation_root, config_hash, position_root, snapshot_root, terms_hash, Allocation,
        ConfigFields, Inputs, Observation, Position, Terms, WeatherParams,
    };
    use soroban_sdk::{testutils::Address as _, vec as svec, BytesN, Symbol};

    /// weather_v1's flat config_hash equals the factory's over the same generic config — so the
    /// SAME factory/registry derives a weather instrument_id (the registry surface is unchanged).
    #[test]
    fn weather_config_hash_matches_factory_byte_for_byte() {
        let env = Env::default();
        let reference = Address::generate(&env);
        let collateral = Address::generate(&env);
        let config = InstrumentConfig {
            reference_asset: reference.clone(),
            terms_hash: BytesN::from_array(&env, &[0x11; 32]),
            schedule_root: BytesN::from_array(&env, &[0x22; 32]),
            snapshot_root: BytesN::from_array(&env, &[0x33; 32]),
            collateral_token: collateral.clone(),
            premium_bps: 150,
            epoch_deadlines: svec![&env, (1u32, 700u64)],
        };
        let contract_ch = hash_config(&env, &config);
        let guest_cf = ConfigFields {
            reference_asset_xdr: address_xdr(&env, &reference),
            terms_hash: [0x11; 32],
            schedule_root: [0x22; 32],
            snapshot_root: [0x33; 32],
            collateral_token_xdr: address_xdr(&env, &collateral),
            premium_bps: 150,
            epoch_deadlines: vec![(1, 700)],
        };
        assert_eq!(contract_ch.to_array(), config_hash(&guest_cf));
    }

    /// weather_v1's allocation_root equals the settlement contract's hash_allocations over real
    /// Stellar Addresses — so the SAME settlement WASM verifies a weather payout set.
    #[test]
    fn weather_allocation_root_matches_settlement() {
        let env = Env::default();
        let b1 = Address::generate(&env);
        let allocs = svec![&env, (b1.clone(), 400i128)];
        let contract_root = hash_allocations(&env, &allocs);
        let guest_allocs = vec![Allocation { buyer: address_xdr(&env, &b1), amount: 400 }];
        assert_eq!(contract_root.to_array(), allocation_root(&guest_allocs));
    }

    fn weather_inputs(env: &Env) -> (Inputs, Address) {
        let buyer = Address::generate(env);
        let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 200 };
        let terms = Terms { trigger_mm: 500, exhaust_mm: 100 };
        let positions = vec![Position { buyer: address_xdr(env, &buyer), cover: 800, salt: [7; 32] }];
        let config = ConfigFields {
            reference_asset_xdr: address_xdr(env, &Address::generate(env)),
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&params),
            collateral_token_xdr: address_xdr(env, &Address::generate(env)),
            premium_bps: 150,
            epoch_deadlines: vec![(1u32, 200u64)],
        };
        let type_id_xdr = symbol_xdr(env, &Symbol::new(env, "weather_v1"));
        let proot = position_root(&positions);
        let instrument_id =
            settle_weather_v1::derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
        let inputs = Inputs {
            type_id_xdr,
            rules_version: 1,
            config,
            instrument_id,
            epoch: 1,
            deadline: 200,
            terms,
            params,
            collateral: 1000,
            observations: vec![Observation { station: [9; 32], mm: 300, observed_at: 150 }],
            positions,
            position_root: proot,
        };
        (inputs, buyer)
    }

    /// The cross-compiled weather zkVM guest, run in the executor (no proof), commits the SAME
    /// 116-byte journal as native settle() — confirming the circuit matches the reference rule.
    #[test]
    fn weather_zkvm_guest_journal_matches_native_settle() {
        use parallar_methods::SETTLE_WEATHER_V1_GUEST_ELF;
        use risc0_zkvm::{default_executor, ExecutorEnv};
        let env = Env::default();
        let (inputs, _buyer) = weather_inputs(&env);
        let native_journal = settle_weather_v1::settle(&inputs).unwrap().1.to_bytes();
        let exec_env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let session = default_executor().execute(exec_env, SETTLE_WEATHER_V1_GUEST_ELF).unwrap();
        assert_eq!(session.journal.bytes.as_slice(), &native_journal[..]);
    }

    /// The weather witness JSON round-trips and its payee decodes (the prove CLI handoff).
    #[test]
    fn weather_witness_json_round_trips_and_payee_decodes() {
        let env = Env::default();
        let (inputs, buyer) = weather_inputs(&env);
        let json = serde_json::to_string_pretty(&inputs).unwrap();
        let back: settle_weather_v1::Inputs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.collateral, 1000);
        let strkey = address_xdr_to_strkey(&back.positions[0].buyer).unwrap();
        let recovered = Address::from_string(&soroban_sdk::String::from_str(&env, &strkey));
        assert_eq!(buyer, recovered);
    }
}

/// Instance #1 ATTESTED (credit_v2, G1): the cross-compiled guest verifies the issuer Ed25519
/// signature IN-CIRCUIT and still commits the native journal — proving in-guest attestation works.
#[cfg(test)]
mod credit_v2_test {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use settle_credit_v2::{
        config_hash, derive_instrument_id, payments_digest, position_root, snapshot_root,
        terms_hash, ConfigFields, Holder, Inputs, Payment, Position, Terms,
    };
    use soroban_sdk::{testutils::Address as _, Symbol};

    fn attested_inputs(env: &Env) -> Inputs {
        let buyer = Address::generate(env);
        let sk = SigningKey::from_bytes(&[42u8; 32]);
        let snapshot = vec![
            Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false },
            Holder { id: [2; 32], balance: 10_000, has_trustline: true, frozen: false },
        ];
        let payments = vec![Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }];
        let positions = vec![Position { buyer: address_xdr(env, &buyer), cover: 800, salt: [7; 32] }];
        let terms = Terms { coupon_rate_bps: 1000, issuer_pubkey: sk.verifying_key().to_bytes() };
        let config = ConfigFields {
            reference_asset_xdr: address_xdr(env, &Address::generate(env)),
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&snapshot),
            collateral_token_xdr: address_xdr(env, &Address::generate(env)),
            premium_bps: 200,
            epoch_deadlines: vec![(1u32, 500u64)],
        };
        let type_id_xdr = symbol_xdr(env, &Symbol::new(env, "credit_v2"));
        let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
        let attestation = sk.sign(&payments_digest(&payments)).to_bytes().to_vec();
        Inputs {
            type_id_xdr,
            rules_version: 1,
            config,
            instrument_id,
            epoch: 1,
            deadline: 500,
            terms,
            collateral: 2000,
            snapshot,
            payments,
            attestation,
            positions: positions.clone(),
            position_root: position_root(&positions),
        }
    }

    #[test]
    fn credit_v2_zkvm_guest_journal_matches_native_settle() {
        use parallar_methods::SETTLE_CREDIT_V2_GUEST_ELF;
        use risc0_zkvm::{default_executor, ExecutorEnv};
        let env = Env::default();
        let inputs = attested_inputs(&env);
        let native_journal = settle_credit_v2::settle(&inputs).unwrap().1.to_bytes();
        let exec_env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let session = default_executor().execute(exec_env, SETTLE_CREDIT_V2_GUEST_ELF).unwrap();
        assert_eq!(session.journal.bytes.as_slice(), &native_journal[..]);
    }

    #[test]
    fn credit_v2_witness_json_round_trips() {
        let env = Env::default();
        let inputs = attested_inputs(&env);
        let json = serde_json::to_string_pretty(&inputs).unwrap();
        let back: Inputs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.attestation.len(), 64);
        assert_eq!(back.collateral, 2000);
    }
}

/// Escape hatch (G2, claim_credit_v1): the cross-compiled claim guest commits the SAME
/// single-allocation journal as the native rule — proving the in-circuit single-buyer claim.
#[cfg(test)]
mod claim_credit_v1_test {
    use super::*;
    use claim_credit_v1::{
        commitment, config_hash, derive_instrument_id, snapshot_root, terms_hash, ClaimInputs,
        ConfigFields, Holder, Payment, Position, Terms,
    };

    fn h(b: u8) -> Holder {
        Holder { id: [b; 32], balance: 10_000, has_trustline: true, frozen: false }
    }

    fn claim_inputs() -> ClaimInputs {
        let snapshot = vec![h(1), h(2)];
        let payments = vec![Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }];
        let terms = Terms { coupon_rate_bps: 1000 };
        let pos0 = Position { buyer: vec![0x10u8; 40], cover: 600, salt: [1; 32] };
        let pos1 = Position { buyer: vec![0x20u8; 40], cover: 400, salt: [2; 32] };
        let positions = vec![pos0.clone(), pos1.clone()];
        let commitments = vec![
            commitment(&pos0.buyer, pos0.cover, &pos0.salt),
            commitment(&pos1.buyer, pos1.cover, &pos1.salt),
        ];
        let position_root = settle_credit_v1::position_root(&positions);
        let config = ConfigFields {
            reference_asset_xdr: vec![0xAA, 1, 2, 3],
            terms_hash: terms_hash(&terms),
            schedule_root: [0x55; 32],
            snapshot_root: snapshot_root(&snapshot),
            collateral_token_xdr: vec![0xBB, 4, 5, 6],
            premium_bps: 200,
            epoch_deadlines: vec![(1u32, 500u64)],
        };
        let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
        let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
        ClaimInputs {
            type_id_xdr,
            rules_version: 1,
            config,
            instrument_id,
            epoch: 1,
            deadline: 500,
            terms,
            collateral: 2000,
            snapshot,
            payments,
            commitments,
            claimant_index: 0,
            claimant: pos0,
            position_root,
        }
    }

    #[test]
    fn claim_zkvm_guest_journal_matches_native() {
        use parallar_methods::CLAIM_CREDIT_V1_GUEST_ELF;
        use risc0_zkvm::{default_executor, ExecutorEnv};
        let inputs = claim_inputs();
        let native_journal = claim_credit_v1::claim(&inputs).unwrap().1.to_bytes();
        let exec_env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let session = default_executor().execute(exec_env, CLAIM_CREDIT_V1_GUEST_ELF).unwrap();
        assert_eq!(session.journal.bytes.as_slice(), &native_journal[..]);
    }
}
