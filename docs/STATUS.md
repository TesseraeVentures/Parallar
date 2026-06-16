# STATUS LOG — append daily: done / next / blocked

## 2026-06-13 (Sprint 0 — Derisk)
- **Done:**
  - Doc-suite alignment review (9 dimensions, adversarially verified) + reconciliations applied across CLAUDE.md / TECH_SPEC / PRD / SPRINT_PLAN / PRODUCTION_GAP / README (trust-model law #4 → "faithful to §1"; versioning gloss; layered revenue model; holder count = 10; G3 cross-ref; register_type fields; Non-Goals; registry-naming collision; `trade_settlement_v1`; demo.sh/R10 slip carve-out; 2026 weekday labels).
  - Spec edit applied: TECH_SPEC §3.3 `settle()` now binds `journal.deadline == epoch_deadlines[journal.epoch]` (closes the prover-chosen-cutoff gap; R5 now enforceable).
  - Repo init: `git init`, `.gitignore` (`.env` gated), `.env.example` (testnet-only template).
  - Toolchain: rust 1.94 + `wasm32-unknown-unknown`; stellar-cli **26.1.0** (stellar-xdr 26.0.1); rzup **0.5.0**; cargo-risczero **3.0.5** (RISC Zero **3.0.x**, matches verifier `^3.0` + parameters.json v3.0.0). node v22, jq, Homebrew, Rosetta all present. ⚠️ risc0 `cpp` toolchain component failed to unpack — irrelevant for pure-Rust guests; re-install if a C/C++ guest is ever needed.
  - External examples cloned (gitignored `external/`), **pinned commits**:
    - `NethermindEth/stellar-risc0-verifier` @ `e8ff6ea202db195352c0141ecc533ff649393fe4`
    - `stellar/soroban-examples` @ `7b168174ae1268dab91a0190d80a94ab7ff41b59`
  - **Verifier derisk GREEN:** `cargo test` on `groth16-verifier` passes on arm64 (4/4) — a real embedded Groth16 proof verified via Soroban BN254 host fns, no x86 needed. verify() ≈ **~35M CPU insns** (Bn254Pairing 17.5M + G2-subgroup-check 11.8M + G1Mul 5.8M), ~3× headroom under Soroban's ~100M/tx budget. First verify-cost benchmark captured.
  - **Spike 2 scoped (Poseidon):** soroban-sdk 25.3 exposes `poseidon_permutation`/`poseidon2_permutation` (caller supplies field/t/d/rounds_f/rounds_p/MDS/round-constants; adapted from HorizenLabs/poseidon2). Confirms TECH_SPEC §2 "Protocol 25 host fn". No higher-level `poseidon_hash` yet → sponge built by us identically on both sides. Reference BN254 t=3 instance in env-host tests. Parity plan: same params + HorizenLabs `zkhash` algorithm guest-side → byte-match. RESOLVED: `poseidon_*` callable on-chain via the `soroban-sdk/hazmat-crypto` feature (`env.crypto_hazmat()`); always available in `#[test]`. Ground-truth captured: host `poseidon2_permutation([0,1,2])` BN254 t=3 = `[0x0bb61d24…, 0x303b6f7c…, 0x1ed25194…]`. ✅ **GREEN:** harness `spikes/poseidon_parity/` passes — `zkhash` 0.2 Poseidon2 reproduces the host vector byte-for-byte → guest↔host parity proven. **SETTLED Poseidon params (Sprint 0 decision):** Poseidon2 over BN254, t=3, d=5, rounds_f=8, rounds_p=56; guest = `zkhash` `POSEIDON2_BN256_PARAMS`, chain = `hazmat-crypto` `poseidon2_permutation` with the same params; sponge/compression to be defined identically on both sides in the commitment design.
  - **Keys:** testnet ADMIN `GBPA7CMP…KUIX` + DEPLOYER `GBMBWXCD…D7E` generated into gitignored `.stellar/` + `.env`. Friendbot funding handed to founder.
- **Key derisk finding (Spike 1):** host is **arm64 (Apple Silicon)**. RISC Zero Groth16 **proof GENERATION needs x86_64** (confirmed by the verifier's own docs); proof **VERIFICATION on Soroban runs on arm64**. The verifier ships a real embedded Groth16 proof (`groth16-verifier/src/test.rs`) — running its test suite locally derisks the whole verification path with no x86 needed. → **DECISION (June 13, Spike 1):** generate proofs via **local Rosetta x86 Docker**, time-boxed (4h box per TECH_SPEC §2); fall back to x86 VM / CI if it fights. Rosetta x86 Docker confirmed working (`docker run --platform linux/amd64` → `x86_64`); `risc0-groth16` prover component installing; full generation spike pending its completion. **Hosting (founder):** dev on this arm64 Mac; hackathon demo + production proving on an x86 VM/server (founder-provisioned) — local Rosetta is the dev-loop bridge only.
- **Sprint 0 exit: essentially MET** — external Groth16 proof verified in a Soroban contract on this setup ✅ (testnet-deploy invoke pending funding) AND Poseidon parity resolved ✅. Both blocking spikes cleared; `risc0-groth16` component installed (Rosetta generation spike ready to run).
- **Sprint 1 progress (commits R0–R4 on branch `dev`; testnet ADMIN+DEPLOYER funded 10k XLM each):**
  - **BondContract GREEN (4/4):** SAC coupons, partial 7-of-10, terms/schedule/snapshot commitments, **no payments getter**, `CouponPaid` event.
  - **VaultContract GREEN (7/7):** public collateral + opaque committed positions, sha256 `position_root` fold (guest-reproducible, verified against off-chain recompute), Option-B solvency, cover-floor on withdraw, settlement-only `pay_allocations` (Σ≤collateral), settlement-window freeze.
  - **SettlementContract GREEN (6/6):** decodes the 116-byte journal (§3.4); enforces instrument_id / one-per-epoch / `journal.deadline==stored && elapsed` / `position_root==vault` / `allocation_root==hash(allocs)`; cross-calls `vault.pay_allocations` — **payouts only via verified settlement, proven end-to-end**. `mock-verify` stub (default-on, Sprint 1); no admin/pause path. Negatives revert (replay, stale root, wrong instrument, tampered allocs, pre-deadline).
  - **Factory/Registry GREEN (5/5):** `register_type` (immutable type entry: image_id + version + rules_uri + vault/settlement wasm hashes) and `deploy_instrument` — deploys the generic vault+settlement pair via the **Soroban deployer pattern** (`env.deployer().deploy_v2`) and cross-inits atomically in one tx; `instrument_id = H(type_id ‖ rules_version ‖ config_hash)`. Asset-policy eligibility gate + versioning immutability + the **replication beat** (2nd independent instance) all tested against the actually-deployed pair. Deployer-pattern atomicity worked first try — **no fallback needed**. All four contracts build to wasm (`wasm32v1-none`). **Workspace 22/22, zero warnings.** `make test` enforces wasm-build-before-test.
- **Sprint 1 COMPLETE — all four production-surface contracts (factory / bond / vault / settlement) built, integrated, tested.**
- **Next:** **Sprint 2 — the guest.** `settle_credit_v1` RISC Zero guest (snapshot verify → owed → payment scan → missed/short determination → commitment opening + position_root recompute → pro-rata allocation + cap → commit the 116-byte journal), host CLI (`prove`/`submit`/`history_builder`), then the local Rosetta Groth16-generation spike to produce a real proof, and **replace the settlement `mock-verify` stub with on-chain Groth16 verification (June 22)**. Guest must reproduce: the sha256 `position_root` fold AND the `allocation_root` encoding (sha256 fold over `addr.to_xdr ‖ amount_be`); Poseidon2 commitments use the parity-settled params. In parallel: testnet deploy of the factory + a full `deploy_instrument` on-chain. Human-only: DoraHacks/#zk-chat/Telegram (founder).
- **Blocked:** none. (Deferred by request: COMPETITION "$2B" vs deck "$3B+" RWA figure; deck edits.)

## Sprint 2 — the guest (settle_credit_v1)
- **Done:** determination + payout LOGIC built (R5) + hardened (R6) — pure Rust, **18/18 native tests** across every §4 P0-blocking case (full miss / partial pro-rata / short / fully-paid→NoDefault / deadline boundary / clawback / frozen / no-trustline / cap / Σ≤collateral / commitment-mismatch / negative balance|payment|cover rejection / mixed-representation §10.2 / journal byte-layout). Poseidon2 commitments (parity-settled params) + sha256 `position_root` fold; `Journal::to_bytes()` = §3.4 116-byte layout.
- **Adversarial review (20-agent workflow) — 3 BLOCKERS + 1 major found before building proving on top:**
  - **M1/M2 (soundness):** the guest never verifies `config_hash`/`instrument_id`, so `snapshot_root` + `coupon_rate_bps` are free prover-chosen inputs → within the proof boundary a FALSE claim could prove. Must bind the guest's inputs to the committed config.
  - **M3 (bridge):** `allocation_root` mismatch — guest folds raw key, contract folds `addr.to_xdr` → under the real verifier (June 22) every settlement reverts.
  - **M4 + minors/nits: FIXED in R6.**
- **Root cause:** factory `config_hash = sha256(InstrumentConfig.to_xdr)` and settlement `allocation_root` via `addr.to_xdr` use full Soroban XDR — not reproducible in a RISC Zero guest.
- **DECISION (resolved):** flat canonical encoding + host parity (chosen; journal-binding alternative rejected per law #2). **R7–R9 DONE:**
  - **R7** — factory `config_hash` → flat guest-reproducible encoding.
  - **R8** — guest soundness binding (re-derive config_hash→instrument_id, bind snapshot/rate/deadline to committed config) + Address-XDR payees; **22/22 guest tests** incl. fabricated-config/snapshot/rate/deadline all rejected.
  - **R9** — prover host (`address_xdr`/`symbol_xdr` bridge) + **byte-exact parity tests vs the REAL contracts**: guest config_hash == factory, guest instrument_id == factory, guest allocation_root == settlement (**3/3**, real Stellar Address XDR).
  - ⇒ **All three adversarial-review blockers (M1/M2 soundness, M3 bridge) CLOSED & VERIFIED.** Workspace: contracts 22/22, guest 22/22, host 3/3.
- **Next:** the proving infrastructure — RISC Zero zkVM guest binary wrapping `settle_credit_v1` (methods crate + `env::read` inputs / `env::commit` the 116-byte journal), host CLI (`prove`/`submit`/`history_builder`), the local Rosetta x86 Docker Groth16-generation spike to produce a real proof, then **replace the settlement `mock-verify` stub with on-chain Groth16 verification (June 22)** + a full testnet `deploy_instrument` → settle. Human-only: DoraHacks/#zk-chat/Telegram (founder).

## 2026-06-14 — Proving spike (R10–R11): first REAL Groth16 proof
- **Done:**
  - **R10** — RISC Zero zkVM guest binary wrapping `settle_credit_v1` (`prover/methods` + `prover/guest`): `env::read` inputs → `env::commit` the 116-byte journal. **Executor validation GREEN:** `zkvm_guest_journal_matches_native_settle` runs the guest under the executor and asserts the committed journal == native `settle()` byte-for-byte.
  - **R11** — Groth16 proving harness (host), no alloy. `groth16_proof_generates_and_verifies` (`#[ignore]`, needs Docker): `default_prover().prove_with_opts(groth16)` → `verify(image_id)` → assert journal == native; pulls the raw Groth16 seal off the receipt + image_id + journal_digest (the values the on-chain verifier consumes). Dropped `risc0-ethereum-contracts` (pulled the whole alloy tree, filled the disk) — router-selector seal wrapping belongs with verifier wiring.
  - ✅ **FIRST REAL GROTH16 PROOF GENERATED + VERIFIED end-to-end** on this arm64 Mac via the §2 Rosetta-x86 dev-loop path: guest exec → **`r0vm` STARK prove** → **SNARK-wrap via `risczero/risc0-groth16-prover:v2025-04-03.1`** (linux/amd64 under Rosetta) → **receipt verified against `image_id`** `0x705ddac439284e2593aa8e510121e7e82dcc441549c7e8c8af0e77bd053d1891` → **journal asserted byte-equal to native settle** (`journal_digest=0xcf03d7daf7c2f3ab5aaa6b7506d92a0c1aa771ad0644668e5834dda130b0e2b5`, seal=256B). **Benchmark (N=1): 2027.77s (~33.8 min)** end-to-end (incremental compile + STARK + Rosetta-emulated SNARK wrap) on Apple Silicon. Validates the Spike-1 decision: Rosetta x86 Docker generation works for the dev loop (production proving still on founder's x86 VM).
  - [Ops, not code] Env was wedged: data volume at 98% (Docker VM hit `no space left on device` on boot) + Docker Desktop engine stuck (GUI/supervisor, `dockerd` not serving). Fixed by reclaiming ~3.5 GB of regenerable caches and starting the engine via the **`docker desktop` CLI** (`docker desktop start`) rather than the GUI — engine stable, ran the 34-min proof to completion. Proof scratch reclaimed after (`docker desktop stop` freed ~5.9 GB); disk back to ~7.5 GiB.
- **Next:** wire the real proof to chain — **replace the settlement `mock-verify` stub with on-chain Groth16 verification** (Nethermind verifier) fed this seal+image_id+journal, including the **router-selector wrapping of the 256B seal** for the Stellar verifier (the deferred verifier-wiring step); full testnet `deploy_instrument` → prove → submit → settle; capture **N=10 benchmark + extrapolation** for the README. Human-only: DoraHacks/#zk-chat/Telegram (founder).
- **Blocked:** none. (Note: Rosetta-emulated proving is ~34 min/proof — fine for the dev loop; the N=10 benchmark will be slow locally, so consider running it on the founder's x86 VM.)

## 2026-06-14 (cont.) — R12: real on-chain Groth16 verification wired (un-stubbed)
- **Done:**
  - **The `mock-verify` stub is GONE.** Deleted the feature from `contracts/settlement/Cargo.toml` and the cfg-gated `verify_proof` pair from settlement — the shipped settlement WASM now has **no payout path that bypasses verification** (architectural law #1: ZK structurally necessary). Ahead of the June-22 target.
  - `settle()` now performs **real verification**: computes `journal_digest = env.crypto().sha256(&journal)` and calls the RISC Zero verifier **router** cross-contract — `VerifierRouterClient::verify(seal, image_id, journal_digest)` — as step 1, before any binding is read. A bad proof / wrong selector / unknown verifier traps inside that call (the sole gate). The router client is **hand-written** in settlement (mirrors the existing `VaultClient` pattern), so **no `risc0-interface` dependency** is pulled — sidesteps the SDK/edition-2024 unification risk entirely.
  - **Topology = cross-contract router (not embedded):** keeps the generic settlement WASM small and the verifier external/upgradeable-by-selector — preserves Law #2's four surfaces. The verifier router address is **factory-level system config**: `factory.__constructor(admin, verifier_router)` stores it and `deploy_instrument` plumbs it into each `settlement.init(...)`. It is **NOT** a field on `InstrumentType` — the registry-interface surface stays frozen (Law #2). Settlement gained one stored field (`DataKey::Verifier`) + a `verifier()` getter; factory gained `DataKey::Verifier` + `verifier()` getter.
  - **New negative-path beat — `forged_proof_reverts`** (DoD invariant that had no test): a mock-router test-double accepts any seal except the `[0xFF;4]` sentinel, which it rejects → `settle` traps before any payout. The other 5 reverts (replay/stale-root/wrong-instrument/tampered-alloc/pre-deadline) now run through the real cross-contract verify path too.
  - **Workspace green:** contracts 23/23 (settlement 7, factory 5, vault 7, bond 4); prover guest 22 + host 4 (+ groth16 spike `#[ignore]`). All four contracts still build to `wasm32v1-none` (settlement 5254 B optimized).
- **Next:** prove the REAL on-chain path deterministically. The R11 spike proof used a **placeholder buyer XDR** (`vec![0x12;40]`) that no real Soroban `Address` produces, so `settle()`'s `hash_allocations` can't match its `allocation_root` — regenerate a proof with **real Address XDRs** (host `address_xdr` bridge), persist a fixture (260-B selector-wrapped seal + 116-B journal + image_id), then an integration test that deploys the **actual** groth16-verifier+router, registers the selector, and runs `settle()` with the real seal (positive path) → closes the "host-produced seal verified on-chain" gap. Then host CLI (prove/submit/history_builder), testnet deploy of the verifier stack + factory + instrument, `demo.sh`, README honesty fixes + N=10 benchmark table.
- **Blocked:** none. (Real-proof on-chain test needs a ~34-min Rosetta regen with real addresses; the Nethermind verifier router must be deployed + selector-registered on testnet before any live settle — `deployment.toml` ships empty for testnet, so the team deploys the whole stack.)

## 2026-06-14 (cont.) — R13: prover host CLI (`prove` + `submit`)
- **Done:**
  - `prover/host` is now a **CLI** (`parallar-prover` binary, clap) on top of the encoding-bridge lib. `prove_settlement(Inputs) -> ProofArtifact` extracted into the lib: runs the guest under the real Groth16 prover, verifies vs `image_id`, asserts the committed journal == native `settle()`, and packages a **submittable artifact**.
  - **Verifier selector captured** by building `groth16-verifier`: **SELECTOR = `73c457ba`** (CONTROL_ROOT a54dc85a…, BN254_CONTROL_ID 04446e66…, parameters.json **v3.0.0** / risc0 3.0). Embedded as `GROTH16_SELECTOR`; `wrap_seal` prepends it → the **260-byte** seal the router dispatches on (selector ‖ raw 256-B A‖B‖C).
  - `ProofArtifact` (serde JSON): 260-B selector-wrapped seal, image_id, the 116-B journal, journal_digest, epoch, total_payout, and the payout set as **(strkey, amount)** in guest-fold order. `address_xdr_to_strkey` decodes the canonical Address XDR (soroban_sdk::xdr + stellar-strkey) — the exact inverse of the host's `address_xdr` bridge.
  - `prove --inputs <witness.json> --out <proof.json>`; `submit --artifact <proof.json> --settlement <C…>` builds + runs `stellar contract invoke … settle(proof, journal, allocations)` (allocations as the `[["G…","amt"],…]` form stellar-cli expects); `--dry-run` prints the invocation (verified).
  - The `#[ignore]` groth16 spike now drives `prove_settlement` with a **real buyer Address** (payee decodes; journal `allocation_root` matches on-chain Addresses) and persists `tests/fixtures/{real_proof,witness}.json` — the fixtures the on-chain verifier integration test + demo will consume.
  - **4 new fast unit tests** (seal-wrap→260 B + selector prefix; Address-XDR→strkey→Address round-trip; ProofArtifact JSON round-trip; witness `Inputs` JSON round-trip + payee decode). Host fast tests **8/8** (+ groth16 `#[ignore]`); contracts 23/23, guest 22 — workspace green.
- **Next:** run `prove` for real (Docker/x86, ~34 min) to emit the fixtures → on-chain verifier integration test (deploy the real groth16-verifier+router, register selector `73c457ba`, verify the fixture seal) → `submit` against a deployed testnet instrument. Then `history-builder` (qualifying-payment scan, §10), `demo.sh`, testnet deploy of the verifier stack + factory + instrument.
- **Blocked:** none. (`submit` is built but unexercised against live testnet — needs a deployed instrument + funded keys; the `[["G…","amt"]]` allocations encoding is the expected stellar-cli form, to be confirmed against a live `settle`.)

## 2026-06-14 (cont.) — R14: REAL proof verified ON-CHAIN (keystone proven, not just wired)
- **Done:**
  - Regenerated a real Groth16 proof with **real Address XDRs** (`prove_settlement` via the spike, ~37 min Rosetta) and persisted fixtures `prover/host/tests/fixtures/{real_proof.json, witness.json}`. Artifact: seal `73c457ba…` (**260 B**, selector-prefixed), image_id `705ddac4…`, journal_digest `6eb03835…`, epoch 1, payout 800.
  - **New integration test `onchain_verify.rs`** deploys the **ACTUAL Nethermind groth16-verifier** (vendored 16 KB wasm fixture, built from the commit-pinned verifier e8ff6ea / parameters.json **v3.0.0**) in a Soroban `Env` and asserts: (1) the host-produced **260-B selector-wrapped seal + image_id + sha256(journal)** VERIFY through the real BN254+Poseidon verifier — `verify()` returns `()`; (2) a **1-byte-tampered seal traps** (rejected). Runs in **0.3 s** (verification is cheap; only generation is slow).
  - ⇒ **Closes R12's #1 risk:** a Parallar-generated proof is accepted on-chain. The selector-wrapping (`73c457ba`) and the `journal = sha256(journal_bytes)` digest convention are confirmed correct end-to-end — exactly the inputs `settlement.settle()` forwards to the verifier router. The router is a thin selector-dispatcher over this verifier (verifier-project-tested), so the in-Env direct-verify is the load-bearing validation; the full router path runs live in the testnet demo.
  - Workspace green: prover host lib **8** + `onchain_verify` **2**; contracts **23**; guest **22**.
- **Next:** the full on-chain `settle()` path — deploy vault+settlement+router on testnet, `submit` the real artifact, settle through the router (R13 `submit` + demo.sh). Then `history-builder` (witness from a qualifying-payment chain scan, §10), `demo.sh` (the DoD scenario), and the N=10 benchmark on x86.
- **Blocked:** none. (Live full-`settle()` through the deployed router is pending the verifier stack + factory + instrument on testnet; in-Env the proof verifies directly against the groth16-verifier.)

## 2026-06-14 (cont.) — R15: COMPLETE on-chain settle() proven with the real proof
- **Done:**
  - New `full_settlement_pays_out_with_real_proof` integration test — the **whole pipeline, end to end, deterministic, no testnet**. Deploys the real groth16-verifier + vault + settlement in-Env; reconstructs the exact committed position (`vault.buy_protection` with the guest's Poseidon `commitment(buyer_xdr, cover, salt)` from the witness) so **`vault.position_root()` == the journal's committed `position_root`** (asserted); funds collateral; then runs `settlement.settle(real 260-B seal, journal, allocations)` past the deadline.
  - Result: the real Groth16 proof **verifies through the on-chain verifier**, **every binding passes** (instrument_id · one-per-epoch · deadline==stored && elapsed · position_root==vault · allocation_root==hash(allocs)), and the **buyer is paid 800 from the vault** (Σ ≤ collateral), epoch marked settled, collateral reduced. Confirms both cross-contract legs (`settlement → verifier`, `settlement → vault.pay_allocations`) and `position_root`/`allocation_root` parity (guest ↔ contracts) under a REAL journal.
  - `onchain_verify` **3/3** (verify · tampered-reject · full-settle); host lib 8; contracts 23; guest 22 — **full workspace green**. Settlement points at the groth16-verifier directly; the production router is a thin selector-dispatcher with the identical `verify(seal, image_id, journal)` interface (exercised live in the testnet demo).
- **Next:** testnet deploy (factory + verifier stack + instrument) + `submit` against it (the live counterpart of this in-Env proof); `history-builder` (witness from a qualifying-payment chain scan, §10); `demo.sh` (the DoD scenario: register → deploy ×2 → deposits → cover → epoch-0 no-proof → prove → submit → settle → forged/replay/stale reverts → benchmarks); N=10 benchmark on x86.
- **Blocked:** none.

## 2026-06-14 (cont.) — R16: demo.sh (the DoD fresh-clone demo) — GREEN
- **Done:**
  - `demo.sh` + `make demo` — the Definition-of-Done one-command walkthrough, **fresh-clone runnable in seconds, no testnet keys and no 34-min live proof** (uses the committed real-proof fixture + the in-process Soroban host). Narrated, runs the FULL scenario against real code and fails loudly on any beat:
    1. toolchain → 2. build the four contracts to wasm → 3. factory: register `credit_v1` + **deploy TWO instruments** (replication beat) + asset-policy gate → 4. vault: deposits + **Poseidon-committed cover** (positions hidden) → 5. guest: a fully-paid epoch is **UNPROVABLE** (NoDefault) + partial-default determination (22 cases) → 6. **a REAL Groth16 proof verifies ON-CHAIN** (actual RISC Zero verifier) + **full `settle()` confidential payout** from the vault → 7. negative paths revert (forged/tampered seal, replay, stale position_root, tampered allocation_root, pre-deadline) → 8. benchmarks.
  - README quick-start rewritten: `make demo` is the headline; the host-CLI `prove`/`submit` shown for the live path. Repo map + the (now real) demo reference reconciled.
- **Next:** testnet deploy (factory + verifier stack + instrument) + live `submit`; `history-builder` (§10 qualifying-payment chain scan → witness); N=10 + 1k extrapolation benchmark on x86; frontend + 2–3 min video (P1, post-freeze).
- **Blocked:** none.

## 2026-06-14 (cont.) — R17: history-builder (the §10 witness input layer)
- **Done:**
  - `prover/host/src/history_builder.rs` — turns observed chain data into the guest witness's `snapshot` + `payments`, implementing the normative §10 rules: **§10.2 asset-received basis** (classic `Payment` ops + SAC `transfer` events + path payments all flatten to `RawTransfer` rows; representation-agnostic; filtered to the coupon asset), **§10.3 muxed→base** (`base_account_key` resolves M-addresses to their base G-account 32-byte key), **clawback** (carried through as `clawed_back` → guest counts it as shortfall), **§10.6 pluggable source** (`DataSource` trait; `FileSource` = archive/export; live RPC is a same-shape swap, not a rewrite). Honest scope: it normalizes *observed* data, not its canonicity — the truncated/withheld-history gap is G1 (documented in the module).
  - `parallar-prover history-builder --scan <scan.json> --params <template.json> --out <witness.json>` fills a witness template's snapshot+payments from a scan; smoke-tested end-to-end (native transfer correctly filtered out; muxed normalized; valid G-strkey parsed).
  - 4 new unit tests (G-key identity · muxed→base · non-account rejection · asset-received+muxed+clawback). Host lib **12** (+ groth16 `#[ignore]`); onchain_verify 3; contracts 23; guest 22 — workspace green. Added a history-builder beat to `demo.sh`.
- **Next:** testnet deploy + live `submit` (+ a live RPC `DataSource` impl behind the trait); N=10 + 1k extrapolation benchmark on x86; README N=10 numbers + instance-#2 section; frontend + video (P1).
- **Blocked:** none. (Live RPC `DataSource` impl deferred — the FileSource/archive path is the MVP source; the demo + tests run on it.)

## 2026-06-14 (cont.) — R18: sprint-plan coverage audit + Sprint-3 polish
- **Done:**
  - **Full sprint-plan COVERAGE AUDIT** (4-agent workflow): **P0 is complete and clean, ~10 days early.** R1–R7 all done with real test coverage; the four surfaces intact; **every** CLAUDE.md invariant has a real (non-stub) test (clawback/frozen/no-trustline/Σ≤collateral/one-per-epoch/forged-reverts at two layers/stale-root/position_root+allocation_root binding/factory-rejects-clawbackable/versioning-immutable/no-proof-without-trigger); mock-verify fully deleted; no admin payout path; **zero P1/P2/non-goal code wrongly built** (instance #2, attested feeds, escape hatch, tranches, yield router — grep-confirmed absent → anti-gold-plating clean); README trust model faithful to TECH_SPEC §1. The genuinely-open items are **external-blocked** (live testnet `submit` = founder keys + verifier-stack deploy; N=10 timing = x86 VM) or **human** (video) — not code gaps. One minor coverage note: the vault window-freeze (`set_window`) is tested in isolation but never driven by `settle()` end-to-end — not a safety gap (the binding Σ≤collateral guard + withdraw cover-floor hold regardless), logged for later.
  - **Sprint-3 polish, 3 in-scope items via parallel sub-agents (disjoint files):**
    - `reset.sh` + `make reset` — the DoD/PRD `demo.sh`/`reset.sh` pair: idempotent `cargo clean` of both workspaces + throwaway-artifact cleanup, **preserving the committed fixtures**.
    - README **Instance #2 (`weather_v1`)** dedicated section — a surface-mapping table (vault/settlement/journal/registry all unchanged; only the per-type guest changes = a new image_id = a new type, never an in-place upgrade), honestly **"designed-for, not built"** (G8), citing the replication beat.
    - `parallar-prover bench --inputs <witness> --n <N>` — times the real prover N times (per-run + min/mean/max + 1k-extrapolation note + verify-cost context) so the founder's x86 N=10 run is one command; README Benchmarks note references it.
  - Workspace green: contracts 23, guest 22, host lib 12 (+ groth16 `#[ignore]`), onchain_verify 3. `demo.sh` unaffected.
- **Next:** live testnet deploy + `submit` (founder keys); N=10 run on x86 (`parallar-prover bench`); **frontend (P1 — held for explicit go-ahead; partly gated on a deployed instrument)**; video + DoraHacks submission (founder).
- **Blocked:** none — all open items are external-resource (keys/x86) or human/founder, not code.

## 2026-06-14/15 — R19 + R20: LIVE ON TESTNET (deploy + a real on-chain settlement)
- **R19 — full stack deployed to testnet** (Friendbot-funded admin/deployer, 10k XLM each; `~/.config` is root-owned so the CLI uses `--config-dir .stellar`):
  - groth16-verifier `CCGEOLVI…`, factory(admin, verifier) `CA5WGQMQ…`, uploaded vault/settlement wasm, `register_type(credit_v1, image_id 705ddac4…)`, native-XLM SAC `CDLZFC3S…` marked eligible (claw-proof), and **factory-deployed two instruments** (the replication beat) — real testnet, stellar.expert links. Verified on-chain: `settlement.image_id` == pinned guest id, `settlement.verifier` == deployed verifier, vault↔settlement cross-bound. MVP wiring: settlement points its verifier directly at the groth16-verifier (production router = thin selector-dispatcher, identical interface). IDs in `deployments/testnet.json`; reproducible `scripts/deploy_testnet.sh`.
- **R20 — 🎉 a REAL settlement executed LIVE on testnet (the whole pipeline, on-chain):**
  - New `prover/host/tests/gen_scenario.rs` emits a **matched** witness + on-chain config (config-hash parity holds on real testnet). Deployed a **settleable** 3rd instrument (`7b0e94c5…`, vault `CB3YEEDG…`, settlement `CD4IHJX6…`) with real config roots; **verified the on-chain instrument_id AND vault `position_root` matched the witness BEFORE spending the proof** (deposit 1000 + buy_protection with the Poseidon commitment, cover 800).
  - Generated a **real Groth16 proof** for that exact instrument via the `prove` CLI (~40 min Rosetta-x86; disk-guarded after a first attempt ran the volume dry — needs ~8 GiB transient).
  - **`settle()` on testnet** → the proof was **verified ON-CHAIN by the deployed groth16-verifier** → every binding passed → the vault **paid the buyer 800** (collateral 1000→200, `is_settled(1)=true`). tx `8b19cc71…`: events `transfer(vault→buyer, 800)` + `allocations_paid(epoch 1, 800)` + `Settled(epoch 1, 800)`. **The sprint-plan June-23 milestone — factory-deploy → default → prove → settle → confidential payout — LIVE.**
- **Next:** N=10 bench on x86 (`parallar-prover bench`); frontend (P1); video + DoraHacks submission (founder). The core AND the live testnet demo are DONE.
- **Blocked:** none. (Note: cleared regenerable build targets + cargo caches for the proof's disk headroom — repo source intact; `make test`/`make demo` rebuild from source.)

## 2026-06-15 — R21: frontend (the R9 live testnet console)
- **Done:**
  - `frontend/index.html` — a vanilla, no-build SPA matching the `site/` aesthetic (Spectral/Instrument Sans/Spline Mono, brass-on-navy, the parallax wordmark). **Data-driven from `deployments/testnet.json`**: the 5-step flow, registry+factory+verifier+type, the three deployed instruments, and a "proven on-chain" settlement panel (the real settle tx + the transfer/allocations_paid/Settled events), all with clickable stellar.expert links.
  - **Live on-chain reads:** a best-effort layer (via `@stellar/stellar-sdk` over testnet RPC, read-only `simulateTransaction`) renders the REAL per-instrument state — verified in the preview: instrument #3 `live · settled=true · collateral=200` (post-payout), #1/#2 `settled=false · collateral=0`. Degrades gracefully to the recorded deployment if RPC/CDN is unavailable.
  - `make frontend` serves it (repo root, so the relative `deployments/` + RPC reads resolve). Verified rendering desktop + responsive via the preview tool; no console errors (live layer connects).
  - This is Sprint-3's R9 frontend (the 1-day-cap deliverable) — now meaningful because there's a live deployment + settlement to render. README "Live on testnet" section + repo-map updated; `.claude/launch.json` preview config added.
- **Next:** N=10 bench on x86 (`parallar-prover bench`); 2–3 min video + DoraHacks submission (founder). **The build is feature-complete** — every Claude-buildable sprint item (P0 core, demo, history-builder, live testnet deploy + settlement, frontend) is done; only x86-benchmark + video/submission remain (external/founder).
- **Blocked:** none.

## 2026-06-15 (cont.) — R22: frontend redesign (plain-language) + drop proof-time claims
- **Done:**
  - Rewrote `frontend/index.html` as an **editorial product page** (per feedback: the prior console read as an AI dashboard + was too technical). A **plain-language hero** that states what Parallar IS for a non-technical reader ("When a bond skips a payment, the people it owes get paid" — default protection, reserve funded up front, payout decided by a proof not a claims department/committee/admin key, positions private). A 4-step **"how it works"** (coupons → buyers protect + reserve-funded-first + sealed positions → the math notices a miss and proves it → Stellar verifies and the reserve pays). A **"Seen working"** section that frames the REAL testnet settlement as evidence ("We didn't just describe it. We ran it." → holder unpaid → proof verified on Stellar → buyer paid · reserve 1,000→200) with the live tx + live on-chain reads. Plain trust rules (funded-first / untouchable / one-exit) + the §1-faithful trust note. Reuses the `site/` aesthetic + glyphs (intentional/human). Verified rendering (hero + full layout + live data) via the preview tool.
  - **Removed proof-GENERATION time claims** from all user-facing surfaces (frontend, README benchmarks + prove-command + demo note): the dev-loop figure (Apple-Silicon under Rosetta-x86 emulation) is **not representative** of real proving hardware, so it isn't quoted — proof-gen timings are captured on x86 via `parallar-prover bench`. Kept the **hardware-independent** on-chain verify cost (~35M insns).
- **Next:** capture N=10 proof-gen on representative x86 (founder's Hetzner box). (Attempted N=10 on this Mac — chronically disk-constrained: a single SNARK wrap drives free space to ~2 GiB, so the batch is unreliable here; the x86 box is the right place.)
- **Blocked:** none.

## 2026-06-15 (cont.) — R23: frontend rebuilt as a DeFi-native protocol page
- **Done (per feedback: em-dashes everywhere, hard to read, structure/keyword it like leading DeFi):**
  - Rewrote `frontend/index.html` modeled on leading lending/RWA protocols (Aave / Morpho / Maple / Ondo / Nexus Mutual): a stats-forward hero with DeFi copy (buy **cover**, **underwrite** the **reserve** to earn **premiums**, defaults **settle through a zero-knowledge proof**; **non-custodial / permissionless / private positions / no claims process**) + primary CTAs; a **4-stat bar** (Reserve/TVL · Protection markets · Settled on-chain · On-chain verifier); a **markets table** (the Aave/Compound pattern: Market · Collateral · Reserve · Cover sold · Status) reading **live** from testnet; a 4-step Underwrite → Cover → Prove → Payout; **6 feature cards** (non-custodial & fully-funded · proof-settled not adjudicated · private positions · permissionless · RWA · verifiable on Stellar); the live settlement as a clean receipt (Payout 800 / Reserve after 200 / Authorized by ZK proof) with the real tx link.
  - **Zero em-dashes / en-dashes** (verified by grep). Scannable: cards + table + short copy, no run-on lines.
  - Robustness: recorded `reserve`/`cover`/`settled` added to `deployments/testnet.json` so the markets table + stats show real numbers even when the esm.sh CDN flakes; live RPC reads override and flip the verifier to `live ✓` when available. Verified rendering (hero + stats + markets + steps + features + settlement) via the preview tool.
- **Next:** N=10 proof-gen on the x86 box; video + DoraHacks submission (founder).
- **Blocked:** none.

## 2026-06-15 (cont.) — R24: frontend made dual-sided + testnet-honest
- **Done (per feedback: it is a double-sided protocol, and a testnet hackathon build has no TVL / live protections):**
  - Added a prominent **"Two sides"** section so the page serves both audiences equally: **For bondholders · Buy cover** (protect a bond position; automatic proof-settled payout; cover size private) and **For underwriters · Provide the reserve** (back payouts, earn premium yield; non-custodial; solvency enforced on-chain), each with its own accent (brass / teal). The hero sub now leads with the two-sided framing.
  - Removed launched-protocol framing inappropriate for a testnet build: dropped the **"Reserve (TVL)"** stat and the live-markets table. Stats are now honest testnet facts (Instruments deployed · Settlement proven on-chain · On-chain verifier live · Network Testnet); the deployment is reframed as **"Deployed and proven on Stellar testnet"** with a Deployed/Settled instruments table (not a market to deposit into) and an explicit "hackathon build on testnet, not a launched market" note.
  - Cache-busted the deployment fetch + set the settled stat from the live reads (a stale browser cache had shown 0). **Zero em/en-dashes, zero "TVL"** (grep-verified). Verified rendering (hero, stats, two-sides, steps, deployed table, settlement) via the preview tool; live RPC reads connect (verifier `live ✓`).
- **Next:** N=10 proof-gen on the x86 box; video + DoraHacks submission (founder).
- **Blocked:** none.

## 2026-06-15 (cont.) — R25: frontend hero + overview copy/staging refinements
- **Done (per feedback):**
  - Hero subtext set to the exact three lines, each on its own line, with the "two-sided protocol" statement dropped: "Bondholders buy cover against default." / "Underwriters fund the reserve and earn premiums." / "When a bond defaults, payouts settle through a zero-knowledge proof, not a claims process."
  - Removed all literal "two sides / both sides" labeling (section eyebrow + nav link + footer link now "Overview"; meta + CTA reworded) while still marketing to both participants through the hero lines and the two cards.
  - Fixed the orphaned "in." in the heading: now "One reserve." / "Two ways in." on two balanced lines.
  - Split the overview card descriptions onto separate lines, one sentence per line, for readability.
  - Zero em/en-dashes (grep-verified). Verified via the preview: renders cleanly, live RPC reads connect (verifier live).
- **Next:** N=10 proof-gen on the x86 box; video + DoraHacks submission (founder).
- **Blocked:** none.

## 2026-06-15 (cont.) — R26: frontend made multi-page + orphan/subheader polish
- **Done (per feedback: kill remaining orphans, distribute subheader sentences, build the linked pages):**
  - Orphans fixed structurally: added `text-wrap:balance` to all headings (kills "bonds" in the hero h1 and "proof" in the how-it-works h2 without manual breaks) and `text-wrap:pretty` to body copy; widened the hero h1 to 18ch so it lands as two balanced lines ("Default protection" / "for tokenized bonds."). Section subheaders now distribute their sentences across lines via `<br>` (Overview, How it works, On testnet).
  - Converted the single page into a multi-page site. Extracted the shared design system to `frontend/styles.css` and the on-chain data loader to `frontend/app.js` (element-guarded so every page reuses it). Nav + footer now link real pages, not anchors; homepage sections link out ("For bondholders ›", "For underwriters ›", "Read the full mechanism ›", "See everything that is live ›").
  - Built the four linked pages, consistent with the design system and dual-sided / testnet-honest / no-dashes constraints: **how-it-works.html** (four-step flow, published shortfall/pro-rata formula, what a ZK proof does vs does not guarantee — faithful to TECH_SPEC §1, architecture, guarantees), **bondholders.html** (cover-buyer landing + payout walkthrough + FAQ), **underwriters.html** (reserve/premium landing + capital-protection invariants + risk stated plainly), **testnet.html** (data-driven: contracts, detailed instruments table, the live settlement, real-vs-mocked honesty note).
  - No proof-generation time quoted anywhere. Zero em/en-dashes across all five pages (verified). Verified every page in the preview: CSS/app.js load, active-nav states, headings balance with no orphans, testnet page populates contracts + 3 instruments + live settlement, live RPC reads connect (`live ✓`).
- **Next:** N=10 proof-gen on the x86 box; video + DoraHacks submission (founder).
- **Blocked:** none.

## 2026-06-15 (cont.) — R27: gap analysis + win-leverage bundles (differentiators, honesty hardening, demo narrative)

**Solvency checkpoint (TECH_SPEC §3.2; calendar June 17) — recorded 2026-06-15, ahead of the checkpoint:**
- **Choice:** Option B. Public aggregate `total_cover` with `cover ≤ collateral` enforced on every `buy_protection`; an individual cover is revealed transiently in the buy tx and never persisted per-buyer (`contracts/vault/src/lib.rs`).
- **Rationale:** B keeps the solvency floor on-chain and law-faithful (no per-buyer cover stored or exposed; payouts still move solely via verified settlement) while staying simple enough to ship and live-deploy. Option C (purchase-time solvency proof) is strictly better but adds a second proving path not needed for the demo; deferred to production behind the same vault interface.
- **Deferred:** Option C → PRODUCTION_GAP G3 (also adds seller withdrawal queues over the blunt freeze).

- **Done:**
  - Ran an exhaustive 16-agent gap-analysis workflow (6 audit axes × adversarial verification + synthesis) against the v0.3 specs + DoD. Verdict: P0/R1–R7 complete and green ~10 days before freeze, both architectural laws hold end-to-end, anti-gold-plating clean, DoD substantially met. Founder/external remainder: N=10 x86 proof-gen, video, DoraHacks submission.
  - **Surface differentiators (judge-facing copy):** README intro now states the qualified category-first claim; added a "committee CAN be convinced / this guest CANNOT" pull-quote and a 4-row "How it compares" table (Parallar vs DeFi cover mutual / TradFi CDS desk / embedded tranche, sourced from COMPETITION §6, category labels only, no forbidden words). Frontend index hero eyebrow → "The first protection protocol on Stellar"; added the same comparison table + contrast callout to `how-it-works.html` (new `.cmp` styles).
  - **Correctness + honesty hardening:** added two `#[should_panic]`/`try_*` vault tests proving the payout gate and freeze window REJECT an unauthorized caller (`pay_allocations_requires_the_bound_settlement_auth`, `set_window_requires_the_bound_settlement_auth`) — closes the untested half of Law #1 (every other test used `mock_all_auths`). 9/9 vault tests green. Reconciled doc/code precision: README clawback wording → curated allowlist (MVP) + on-chain `AUTH_CLAWBACK_ENABLED` flag-read (G6); PRODUCTION_GAP G3 names Option B, G6 adds the flag-read upgrade, G9 + TECH_SPEC §10.5 + CLAUDE.md invariant now say "archival-class (persistent or instance), never temporary" to match the implemented `instance`-storage bindings (no permanent-loss path; persistent tiering is the documented G9 production step).
  - **Demo storytelling (narration only):** demo.sh stages the EPOCH 0 (all paid → prover REFUSES) → EPOCH 1 (short-paid → same guest proves it) arc, and adds the confidential-payout punch (the vault paid having only ever seen a commitment + the public aggregate).
- **Next:** per explicit founder override, build beyond P0 toward a "statement protocol" — centerpiece options pending direction (instance #2 `weather_v1`; interactive testnet demo; verifier-router topology; richer benchmark scenario). Founder: N=10 x86 bench, video, DoraHacks, tag `v0.3-p0`.
- **Blocked:** none. (Two architectural laws remain non-negotiable regardless of the build-beyond override.)

## 2026-06-15 (cont.) — R28: Instance #2 (weather_v1) BUILT — the layer thesis, exercised

**Build-beyond-P0, centerpiece 1 of 4 (explicit founder override).** Two architectural laws held intact.
- **Done:**
  - New guest type `settle_weather_v1` (`prover/guests/settle_weather_v1/`): a parametric rainfall-shortfall (drought) cover. Published rule: `payout = cover × (trigger_mm − observed_mm) / (trigger_mm − exhaust_mm)`, capped at cover; `NoBreach` (unprovable, guest panics) when observed rainfall meets/exceeds the trigger — the weather analog of credit's fully-paid-epoch unprovability. Same soundness bindings as credit (instrument_id / terms / params(snapshot_root) / deadline / position_root). 16/16 native tests green.
  - **Versioning law honored:** `settle_credit_v1` untouched — its image_id is still `705ddac439284e…053d1891` (confirmed from the rebuilt ELF). `weather_v1` is a NEW type with its OWN image_id `d31246e6d19379…8db4a9bb`.
  - **Thesis proven in tests, not asserted:** weather_v1's Poseidon commitment, position/allocation roots, and config_hash/instrument_id derivation are byte-identical to credit_v1 (parity tests in the guest) AND match the on-chain factory/settlement encodings (host `weather_test`). So the SAME generic vault, settlement, and factory WASM accept a weather instrument with zero contract changes.
  - RISC Zero wiring: `prover/methods/guest-weather/` builds the weather ELF; `parallar-methods` now embeds both `SETTLE_CREDIT_V1_*` and `SETTLE_WEATHER_V1_*`. Methods build green (4 min). Host `prove_weather_settlement` added (mirrors `prove_settlement`, pinned to the weather id). Executor parity test `weather_zkvm_guest_journal_matches_native_settle` green — the cross-compiled circuit commits the same 116-byte journal as the native rule. Host lib 16/16 green.
  - README Instance #2 + PRODUCTION_GAP G8 updated honestly: guest BUILT (image_id, executor-verified, surface-parity); remaining = register the type + prove one live settlement on x86 (founder), then attested feeds (G1).
- **Next:** still in the build-beyond program — CLI `prove`/`submit` weather support + a weather witness/scenario + register/deploy on testnet (founder x86 for the live proof); then centerpieces 2–4 (live partial-default settlement, scale + production verifier-router topology, interactive testnet dApp).
- **Blocked:** live weather proof + deploy need the x86 box (founder). Everything up to it is built and green.

## 2026-06-15 (cont.) — R29: Instance #2 turnkey — CLI + scenario generator + deploy script

**Build-beyond-P0, centerpiece 1 finished (the founder's x86 step is now one flow).**
- **Done:**
  - `parallar-prover` CLI gained `--guest credit|weather` on `prove` and `bench` (dispatches to `prove_settlement` / `prove_weather_settlement`; `submit` is guest-agnostic, unchanged). Default `credit` keeps the existing flow byte-for-byte. Bench is generalized over the scale label (holders / observations).
  - `prover/host/tests/gen_weather_scenario.rs`: a one-off generator (mirrors `gen_scenario.rs`) that emits a weather `witness.json` (for `prove --guest weather`) + a `scenario.json` (the on-chain InstrumentConfig + commitment + payout) so the deployed instrument_id == the proof's journal instrument_id. Validated end-to-end against the deployed testnet addresses: instrument_id `6f83c10e…`, position_root `7c6a14ce…`, payout 400 (0.5 severity on 800 cover, drought 300mm vs trigger 500 / exhaust 100).
  - `scripts/deploy_weather.sh`: registers `weather_v1` (image_id `d31246e6…`) on the EXISTING factory and factory-deploys a weather instrument using the SAME generic vault + settlement WASM — the layer thesis on-chain. Reads the scenario.json so config matches the witness.
  - demo.sh beat 6b runs the weather rule + states the parity (same core, different guest).
  - Full prover suite green: host lib 16, onchain_verify 3 (credit unchanged), credit guest 22, weather guest 16.
- **Founder x86 flow (turnkey):** `gen_weather_scenario` (real testnet addrs) → `deploy_weather.sh` → fund vault + buy_protection(commitment) → `prove --guest weather` → `submit`. Deploy needs no x86; only the proof does.
- **Next:** centerpiece 3 scale half — a 1k-holder credit determination run for real determination numbers (executor, no proving, fully doable here); then the interactive testnet dApp; then live partial-default + verifier-router topology scripts.
- **Blocked:** live weather proof needs the x86 box (founder). Everything up to it is built, tested, and turnkey.

## 2026-06-15 (cont.) — R30: scale benchmark — determination cycles (hardware-independent)

**Build-beyond-P0, centerpiece 3 (scale half) — done, fully measured here (no x86 needed).**
- **Done:**
  - `prover/host/tests/scale.rs`: runs the credit + weather guests in the RISC Zero EXECUTOR over 10/100/1000 inputs and reports zkVM cycle counts (user cycles + Σ 2^po2 proving cycles). Cycles are deterministic + hardware-independent, so these are representative measured on this machine.
  - **Measured:** credit_v1 determination 10/100/1000 holders = 2.2M / 3.6M / 17.5M user cycles (≈15k cycles per added bondholder); weather_v1 10/100/1000 observations = 2.1M / 2.7M / 8.5M. On-chain Groth16 verify stays FLAT at ~35M insns regardless — unbounded private determination off-chain, constant-size settlement on-chain (the structural ZK payoff, now quantified).
  - README Benchmarks + demo.sh beat 7 now carry the scale table. This closes the gap-analysis "no scale numbers" finding honestly without x86 (the x86 N=10 is still the separate proof-gen wall-clock, founder).
- **Next:** centerpiece 4 — the interactive testnet dApp (read+commit, Law-1-safe), fully doable here; then live partial-default + verifier-router topology scripts.
- **Blocked:** none for the items I can do solo. Live weather proof + the x86 N=10 wall-clock remain founder/x86.

## 2026-06-15 (cont.) — R31: browser commitment via wasm (dApp parity foundation)

**Build-beyond-P0, centerpiece 4 (interactive dApp) — slice 1: the parity-critical core.**
- **Done:**
  - `frontend/commit-wasm/`: a cdylib that compiles the guest's EXACT `settle_credit_v1::commitment` (Poseidon2 BN254) to `wasm32-unknown-unknown`, exposed via manual C-ABI exports (`parallar_alloc` + `parallar_commit`) — no wasm-bindgen/wasm-pack toolchain needed. getrandom (transitive via ark-std) is satisfied with a no-op CUSTOM backend so the wasm carries NO external imports (75KB).
  - **Parity verified end-to-end:** native `commitment([0x12;40], 800, [7;32])` == wasm output via Node == `0687524d192ea03c7fd3345e9764be493714f4afd1b2ddfbadfe789c0575af21`. The browser will compute byte-identical commitments, so a cover bought in the dApp is genuinely private AND settleable (no JS-Poseidon parity gap — it IS the guest's function).
  - Artifact committed at `frontend/commit.wasm`; rebuild via the Cargo.toml header command.
- **Next (dApp slice 2):** `app.html` + `dapp.js` — Freighter connect, a REAL signed underwriter deposit, and buy-cover (commitment via this wasm) on testnet, with live readouts. Signing path needs a real-wallet smoke-test (can't run Freighter in the headless preview).
- **Blocked:** none for the build; wallet-signing verification needs a founder smoke-test.

## 2026-06-15 (cont.) — R32: interactive testnet dApp (centerpiece 4 complete)

**Build-beyond-P0, centerpiece 4 — slice 2: the wallet-connected dApp.**
- **Done:**
  - `frontend/app.html` + `frontend/dapp.js`: connect Freighter, deposit collateral (underwriter), buy cover (bondholder) — real signed transactions against the deployed testnet contracts. Live instrument picker (reserve/cover/settled read from RPC). Buy-cover computes the position commitment in-browser via `commit.wasm` (the guest's exact Poseidon), shows the commitment + the opening to save. "Try it" added to the nav across all pages.
  - **Read+commit only (Law #1):** the only writes are `vault.deposit` and `vault.buy_protection` — neither moves the reserve to a payee; a prominent callout states there is no "pay" button and cannot be. Payouts remain solely via a verified settlement proof.
  - **Verified in the preview:** page + SDK + wasm load; the in-browser commitment for the fixed input equals the parity reference `0687524d…` exactly (so bought positions are genuinely settleable); 3 instruments render live (`live ✓`); actions gate on wallet connect.
  - README testnet section documents the dApp.
- **Caveat (founder smoke-test):** the wallet sign-and-submit path can't run in the headless preview (no Freighter) — everything else is verified. Founder: connect a funded testnet Freighter, deposit, buy cover; confirm the txs land.
- **Build-beyond-P0 status:** centerpieces 1 (instance #2), 3 (scale), 4 (dApp) DONE. Remaining: centerpiece 2 (live partial-default settlement) — scenario/script buildable here, live proof on x86 (founder).
- **Blocked:** none for buildable items; live proofs + wallet smoke-test are founder/x86.

## 2026-06-15 (cont.) — R33: partial-default scenario (centerpiece 2 buildable part)

**Build-beyond-P0, centerpiece 2 — the richer live settlement (multi-holder, multi-buyer, pro-rata).**
- **Done:**
  - `prover/host/tests/gen_partial_scenario.rs`: generator for a PARTIAL default that exercises the pro-rata formula richly — 3 holders (unpaid / half-paid / full → Σ shortfall 1500 / Σ owed 3000, severity 0.5), 2 cover buyers (600 + 400) paid 300 + 200. Emits a witness + a per-buyer scenario.json (two commitments). Validated: instrument_id `de8b9bc7…`, total_payout 500, both buyers paid pro-rata.
  - The founder's x86 flow: deploy with the scenario's terms_hash/snapshot_root/deadline (deploy_testnet.sh env overrides), buy_protection for both commitments, `prove`, `submit` — a second, richer live settlement than the canonical full-default #3.
- **BUILD-BEYOND-P0 COMPLETE (buildable parts):** all four centerpieces done —
  1. Instance #2 (weather_v1) guest + turnkey CLI/scenario/deploy,
  2. live partial-default scenario,
  3. scale determination benchmark,
  4. interactive testnet dApp (wallet + wasm commitment).
  Two architectural laws intact throughout; credit_v1 image_id unchanged.
- **Founder/x86 remainder:** live weather proof+deploy, live partial-default proof, the N=10 proof-gen wall-clock, the dApp wallet smoke-test, video, DoraHacks, `v0.3-p0` tag.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R34: audit-readiness pack (property tests + CI + threat model)

**Build-beyond-P0 follow-on (next-steps program), item 1 of 4: rigor + backability.**
- **Done:**
  - **Property tests (proptest)** over both guests' published rules — fuzz randomized books and assert the invariants ALWAYS hold: Σ payouts ≤ collateral and ≤ Σ cover; a fully-paid book is unprovable (NoDefault) / rainfall meeting the trigger is unprovable (NoBreach); determinism; tampered position_root always rejected. credit guest 22→26 tests, weather 16→19. (`#[cfg(test)]` so image_ids are unaffected.)
  - **CI** (`.github/workflows/ci.yml`): native contract suite + guest determination/property tests, plus a hygiene job that FAILS the build if a `mock-verify` path reappears in settlement (Law #1 guard), if the frontend gains an em/en dash, or if a DoD artifact is missing. Fast — no risc0 toolchain (the ELF/host tests run on the x86 box).
  - **`SECURITY.md`**: the threat model + trust boundary, faithful to TECH_SPEC §1 (correct computation over supplied inputs, not input canonicity; G1 hardens it), the two laws, the tested invariants, the real-vs-mocked summary, and disclosure.
  - Fixed em/en dashes that had crept into frontend JS comments + app.html (now CI-guarded). Verified dApp still loads + the in-browser commitment parity holds.
- **Next (next-steps program):** G1 attested feeds (in-guest issuer-signature verification), G2 escape hatch (claim_direct), verify-it-yourself guide.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R35: G1 attested feeds (credit_v2) + reproducibility fix

**Next-steps program item 2 (G1) DONE, and a reproducibility regression from R34 fixed.**
- **G1 — attested data feeds, BUILT:** new guest type `settle_credit_v2` = credit_v1's determination PLUS in-guest verification of an issuer **Ed25519 signature over the payment snapshot** (issuer key committed in `terms_hash`). "Trust the keeper's data" becomes "trust the issuer's signature." Image_id `d07e6aaf…` (its OWN, pinned); credit_v1 stays `705ddac4…` (versioning law — hardening ships as a NEW type, not an edit). Reuses credit_v1's exact generic primitives (same vault/settlement/factory). ed25519-dalek compiles to the zkVM (default-features=false, verify-only). 7 native tests (tampered/injected/wrong-key → AttestationInvalid; determination matches v1) + host `prove_credit_v2_settlement` + CLI `--guest credit-v2` + an executor test proving the **in-circuit** attestation commits the native journal.
- **Reproducibility fix (R34 regression):** R34 added `proptest` as a DEV-DEPENDENCY of the guest crates, which perturbed the methods/guest build resolution and shifted credit_v1's ELF to `97c0a4d7…` (≠ the deployed `705ddac4…`). Root cause + fix: **guest crates must carry ZERO test deps.** Moved all property tests into a new `prover/proptests` crate (depends on the guests; native-only). Verified credit_v1 rebuilds to `705ddac4…` and weather to `d31246e6…`. (Lesson: a guest dev-dep can change its image_id — never add one.)
- **CI** updated to run the relocated property tests + all guest example tests natively. README trust model + PRODUCTION_GAP G1 record credit_v2 as the built first cut of G1.
- Full suite green: contracts 4/4, prover 18/18; image_ids stable.
- **Next:** G2 escape hatch (claim_direct); verify-it-yourself guide. (credit_v2 live deploy + attested-witness generator are founder/x86 follow-ons.)
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R36: G2 escape hatch — permissionless settle + claim_credit_v1 ZK core

**Next-steps program item 3 (G2).** Built the law-safe parts; FLAGGED the contract surface decision (did not bend a frozen surface).
- **Done:**
  - **Base escape hatch made explicit + tested:** settlement is permissionless — after the deadline, anyone (incl. a covered buyer assembling the witness from public chain data) can submit a valid proof and settle; no privileged keeper. New test `settle_is_permissionless_no_privileged_caller` clears ALL auths and settles successfully (contrast: the vault payout gate still requires the bound settlement's auth). Settlement suite 8/8.
  - **claim_credit_v1 guest (ZK core of the single-buyer claim):** proves ONE buyer's allocation against the committed position_root from the PUBLIC commitments + that buyer's OWN opening (others' openings private), emitting a single-allocation journal. 6 native tests incl. `claim_matches_what_a_full_settlement_would_pay` (claim == the buyer's full-settlement share). Reuses credit_v1's primitives + determination (credit_v1 untouched). "ZK as unconditional claimability."
  - PRODUCTION_GAP G2 updated.
- **FLAGGED (Law #2):** the on-chain `claim_direct` entrypoint needs a SECOND verification key (the claim guest's image_id), which the deployed settlement/registry surface does not carry. Two clean NEW-instrument-family paths: (a) commit the claim image_id in config + a claimable settlement variant with deadline+grace, per-claimant dedup, Σ≤collateral; or (b) Merkle-tree position_root (new vault) for true knows-only-own inclusion. Per CLAUDE.md, surfaced for a decision rather than retrofitted. The claim guest's methods/host wiring is deferred until that decision (no speculative ELF).
- **Next:** verify-it-yourself guide; then take stock against PRODUCTION_GAP G1–G13 + PRD + TECH_SPEC and lay out the production-extension roadmap (incl. the G2 surface decision).
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R37: verify-it-yourself guide

**Next-steps program item 4.** Self-serve verification — judges/SDF confirm every claim from public data.
- **Done:** `VERIFY.md` + `scripts/verify.sh` (+ `make verify`). The script decodes the 116-byte journal, checks sha256(journal)==journal_digest + the 260-byte selector-wrapped seal + image_id==the deployed type's pinned id, prints the live settlement tx + contract explorer links, and lists the exact commands to verify the proof on-chain (`onchain_verify`), confirm no mock-verify path, reproduce the pinned image_id, and re-run the determination/property/scale tests. Ends with the honest trust boundary (computation over supplied inputs; G1).
- Caught + documented an honesty nuance: the committed fixture is the reproducible DEMO proof (its own scenario instrument `5caa6baf…`), distinct from the live on-chain settlement (instrument `7b0e94c5…`, tx `8b19cc71`) — both real, both bound to the pinned image_id `705ddac4…`. verify.sh states this rather than conflating them. README links VERIFY.md; CI checks VERIFY.md/SECURITY.md/verify.sh exist.
- **Next-steps program (this round) COMPLETE:** audit-readiness (R34), G1 attested (R35), G2 escape-hatch core (R36), verify guide (R37).
- **Next:** take stock against PRODUCTION_GAP G1–G13 + PRD + TECH_SPEC; lay out the production-extension roadmap (incl. the G2 on-chain surface decision) for continued build-out.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R38: submission-grade demo + image_id reproducibility guard

**Production push, item 1/4 (submission-grade, buildable-now).**
- **Done:**
  - demo.sh beat 6c: the one-command demo now tells the FULL production-direction story — runs credit_v2 (G1: settlement requires the issuer's signature; tampered/unattested data can't settle), the permissionless-settlement test (G2: no privileged keeper), and claim_credit_v1 (G2: a buyer proves their own allocation from public commitments + their own opening). The versioning law shown live: new guest TYPES harden the trust model; pinned originals never change.
  - `scripts/check_image_ids.sh`: reproducibility guard — rebuilds the guest ELFs and asserts each image_id matches its pinned value (credit_v1 read from deployments/testnet.json = the deployed type; weather d31246e6; credit_v2 d07e6aaf). Catches the R34-class drift. Passes (all three match).
  - CI `image-ids` job (workflow_dispatch + weekly schedule, so it never slows the fast push CI): installs the RISC Zero toolchain and runs the guard.
- **Next (production push):** 2/4 G2 on-chain claim_direct (claimable settlement variant, config-carried claim image_id); 3/4 hardening pack (G6 + G9 + credit_principal_v1); 4/4 G3 Option C.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R39: G2 on-chain claim_direct — claimable settlement variant

**Production push, item 2/4.** The escape hatch, on-chain. Two laws + frozen surfaces intact (new family).
- **Done:**
  - `contracts/claim_settlement` (`parallar-claim-settlement`): a NEW settlement variant with `settle` (full keeper path) AND `claim_direct(proof, journal, claimant, amount)`. claim_direct verifies a single-allocation proof against the CLAIM image_id, gated by deadline+grace, per-claimant dedup, and `settle`/`claim_direct` MUTUAL EXCLUSION (a full settle blocks later claims; any claim blocks a later full settle) so neither double-pays; the vault's Σ≤collateral bounds cumulative claims. 8 tests (pays after grace; reverts before grace / on double-claim / after full settle / forged proof; mutual exclusion; two-buyers-each-claim; keeper path still works). The deployed generic settlement is untouched.
  - claim_credit_v1 wired into the zkVM: `methods/guest-claim-credit-v1` builds the ELF; image_id `b4319def9a29fe76…132124cc`; host `prove_claim_credit_v1` + executor test `claim_zkvm_guest_journal_matches_native` (host lib 19/19). All four image_ids stable (credit_v1 705ddac4, weather d31246e6, credit_v2 d07e6aaf, claim b4319def).
  - `scripts/check_image_ids.sh` now guards all four; PRODUCTION_GAP G2 records the claimable contract as built (path a).
- **Remaining for a live claim:** a claimable-family deploy path (wire both image_ids) + a live claim on x86 (founder).
- **Next (production push):** 3/4 hardening pack (G6 + G9 + credit_principal_v1); 4/4 G3 Option C.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R40: production hardening pack (G6 ops/governance + G9 TTL + credit_principal-by-config)

**Production push, item 3/4.** Built the law-safe parts; corrected two spec-optimisms honestly (anti-gold-plating + flag-don't-bend).
- **Done:**
  - **credit_principal needs NO new guest:** principal protection is `credit_v1` with `coupon_rate_bps = 10000` (owed = full principal) over a single maturity epoch. Demonstrated by 2 new credit_v1 tests (full principal default → full cover; partial repayment → pro-rata). credit_v1 now 24 tests; image_id unaffected (cfg(test)). The generic-core thesis: a new product, zero new code. (G13's "credit_principal_v1" is a deploy-time config, not a guest.)
  - **G6 ops/governance (docs/OPERATIONS.md):** governance model (registry admin = the only privileged op → multisig; no upgrade/pause/admin-pay; deploy keys → HSM), incident runbook (a guest bug ships as a NEW type, e.g. credit_v3, exactly as credit_v2 did; existing instruments immutable; comms protocol), keeper-offline (permissionless settle + claim_direct cover it).
  - **Honest correction — clawback eligibility:** a Soroban contract CANNOT introspect a classic asset's issuer flags (no SAC getter), so the earlier "production reads AUTH_CLAWBACK_ENABLED on-chain" is infeasible. Corrected in README + the factory comment + PRODUCTION_GAP G6 to: curated allowlist governed by the registry multisig + off-chain issuer-flag monitoring + de-list on change (native XLM is structurally claw/freeze-proof).
  - **G9 TTL ops:** `scripts/ttl_monitor.sh` (keeper runbook — lists the live contracts + the `stellar contract extend` command for each) + the operating design in OPERATIONS.md; tenor-derived TTLs noted as the v-next contract change. CI artifact checks extended.
- **Next (production push):** 4/4 G3 Option C — purchase-time solvency proof (a new vault version + a small guest).
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R41: G3 Option C — confidential purchase-time solvency proof (ZK core)

**Production push, item 4/4 (the ZK core).**
- **Done:** `solvency_v1` guest — proves a purchase preserves solvency (`new_total ≤ collateral`) while hiding BOTH the cover and the running totals (the aggregate is a Poseidon commitment), and binds the same hidden cover to the buyer's position commitment (no cover-swap). Closes Option B's transient-cover leak (B reveals each cover in the buy tx). 7 native tests incl. `the_cover_never_appears_in_the_journal` + `cover_must_match_the_position_commitment`. Reuses credit_v1's Poseidon commitment (same BN254 field). 112-byte solvency journal (prev/new aggregate commitments + position commitment + collateral; no cover).
- **Flagged (Law #2, new vault version — not retrofitted):** the consuming confidential-cover vault is a NEW instrument-family version — `buy_protection_proven` advances the committed aggregate against a solvency proof, `withdraw` proves collateral-after ≥ committed cover, plus the keeper/sequencer coordination supplying the running aggregate opening. Documented in PRODUCTION_GAP G3. solvency_v1's methods/ELF wiring deferred until the vault version consumes it (anti-gold-plating).
- **PRODUCTION PUSH (this round) COMPLETE:** 1/4 submission demo + image_id guard (R38); 2/4 on-chain claim_direct (R39); 3/4 hardening pack (R40); 4/4 Option C ZK core (R41). Two laws + frozen surfaces intact throughout; five guest types now (credit_v1, weather_v1, credit_v2, claim_credit_v1, solvency_v1) all on the unchanged surfaces.
- **Next:** founder/x86 deliverables + the flagged new-vault-family builds (G2 claimable deploy path, G3 confidential-cover vault) when prioritized.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R42: money-flow layer 1/3 — premium-aware reserve vault (G11/G12 foundation)

**Production money-flow build (gated items, per founder mandate). Both sides' economics, made real.**
- **Done:** `contracts/yield_vault` (`parallar-yield-vault`) — a NEW instrument-family vault version (deployed generic vault stays frozen). Buyers PAY a premium on `buy_protection` (cover × premium_bps), or premium arrives via `receive_premium` (the router's coupon waterfall); underwriters EARN it pro-rata to collateral via a rewards-per-share accumulator (`claim_premium`); the protocol takes a base fee (`claim_protocol_fee`, ~12% §5A). Liquidity haircut on the solvency floor (`total_cover ≤ (1−h)·collateral`, §3.2) + the float-adapter seam (G12). 9 tests: pro-rata split, NO retroactive accrual for late depositors, withdraw settles premium, haircut/insolvency floors, and — critically — `pay_allocations` pays from the reserve only, never the premium pool (Law #1 separation; defaults still proof-gated + settlement-only).
- PRODUCTION_GAP G11 records the premium foundation built.
- **Next:** the `YieldRouter` (G11) — wrap → pBOND, route_coupon waterfall (premium to the vault, net to holders), unwrap; then G12 float (eligible-reserve-asset list + covenant) and G13 (pBOND token + lending docs).
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R43: money-flow layer 2/3 — YieldRouter / protected share class (G11)

**Production money-flow build. Both sides' economics now complete end-to-end.**
- **Done:** `contracts/yield_router` (`parallar-yield-router`, TECH_SPEC §5A) — upstream of the frozen core. `wrap(holder, amount)` mints pBOND 1:1 (cover auto-sizes to the wrapped balance, registered with the vault via `set_routed_cover` which enforces the shared solvency floor — wrapping past the reserve reverts). `route_coupon(payer, epoch, gross)` runs the waterfall: premium = wrapped × premium_bps; distribution fee (router, §5A(b)); premium − fee → vault.receive_premium (→ underwriters pro-rata + the vault's base fee); NET = gross − premium → pBOND holders pro-rata (rewards-per-share). `unwrap` burns + returns the bond (cover lapses). pBOND `transfer` for external-collateral composability (G13). 4 tests incl. the full 14%-gross → 12%-net-to-holder waterfall (600 net + 80 premium + 10 vault fee + 10 router fee = 700).
- Vault gained `set_router` + `set_routed_cover` + a `routed_cover` getter; effective cover = sealed + routed. `receive_premium` is now router-only + transfer-first (the Soroban contract-auth pattern; no `authorize_as_current_contract` needed). yield_vault still 9/9.
- **Both sides, made whole:** underwriters earn premium (+ future float) pro-rata; pBOND holders get the net protected coupon; the protocol earns base + distribution fees; defaults still pay wrapped holders from the reserve via the proof-gated settlement (Law #1 intact, the router is purely upstream).
- **Next:** G12 (reserve float: eligible-reserve-asset list + the non-circularity covenant + haircuts) and G13 (pBOND-as-collateral docs + the covenant boundary).
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R44: money-flow layer 3/3 — reserve float (G12) + pBOND collateral (G13)

**Production money-flow build COMPLETE (buildable, law-safe, non-x86 parts).**
- **Done:**
  - **G12 float:** `yield_vault::harvest_float(amount)` distributes reserve float yield to underwriters pro-rata (the same accrual as premium — both are their yield) minus the protocol's float share (`set_float_fee_bps`, ~10% §3.2); reserve principal untouched. The liquidity haircut (`total_cover ≤ (1−h)·collateral`) is already enforced in the solvency floor. yield_vault 10/10.
  - **G13 docs:** pBOND is a transferable receipt (`yield_router::transfer`) → posts as external collateral today; principal protection = a credit_v1 config (no new guest). The covenant boundary + LTV/loss-truncation thesis documented.
  - **docs/ECONOMICS.md** — the full both-sides money flow, the coupon waterfall (worked 14%→12% example), the §5A layered fee model, reserve float, the **non-circularity covenant (4 rules)** + haircuts + denomination matching, and pBOND-as-collateral. The SDF-grade business-model + safety doc. PRODUCTION_GAP G12/G13 updated; CI checks ECONOMICS.md.
- **MONEY-FLOW LAYER (G11/G12/G13) built:** premium collection + pro-rata underwriter distribution + protocol base fee (yield_vault); the protected-share-class router + pBOND + coupon waterfall + distribution fee (yield_router); float-yield distribution (harvest_float); pBOND transferability. Both sides' economics are correct and Law #1 holds (defaults still pay only via proof-gated settlement; premium/float are separate pools). 48 contract tests green.
- **Remaining (all external / audit-gated / x86 — not buildable here):** the live yield-strategy adapter (real BENJI-class asset + NAV oracle), the factory deploying the router + the on-chain eligible-reserve list, the Blend lending listing, the G5 audit, the (P)SPI counsel review, and the founder/x86 live proofs + video + submission.
- **Blocked:** none for buildable items — the production build-out is at the boundary of what can be built without x86, an external yield/lending protocol, or an audit.

## 2026-06-15 (cont.) — R45: surface the money-flow layer in the demo

- demo.sh beat 6d runs the yield_vault + yield_router suites: buyers pay premium → underwriters earn it pro-rata + protocol base fee + reserve float; wrap → pBOND, the coupon waterfall (premium → vault, net → holders), pBOND transferable. The both-sides revenue layer now shows in the one-command demo, on the unchanged core; defaults still pay only via proof-gated settlement (Law #1).
- **Production build-out status:** the money-flow layer (G11/G12/G13) is complete + tested (48 contract tests). Buildable, law-safe, non-x86 production items now substantially done across G1/G2/G3-core/G6/G9/G11/G12/G13. Remaining buildable items need a scope/surface steer (the G3 confidential-cover vault; a factory v-next that deploys the new families; G4 per-epoch snapshots). Everything else is external/audit-gated/x86.

## 2026-06-15 (cont.) — R46: tiered protected-share-class factory (G11) — risk-priced, many bonds

**Per founder clarification: the protected share class is a FACTORY model wrapping many bonds, risk-priced, with a standardised tier framework for underwriter appetite.**
- **Done:** `contracts/yield_factory` (`parallar-yield-factory`, a NEW factory version; the deployed credit factory stays frozen). `register_tier(id, min_premium_bps, max_premium_bps, haircut, label)` defines standardised RISK TIERS (e.g. investment-grade 100–300, high-yield 500–1500). `deploy_protected(tier, cfg)` cross-binds a full family (yield_vault + settlement + yield_router) for ANY bond in ONE tx via the deployer pattern, with the instrument's premium RISK-PRICED within its tier's band (validated; out-of-band reverts). Per-instrument premium → different net coupons. Each instrument keeps its own reserve (no cross-instrument correlation). 3 tests incl. `risk_priced_tiers_yield_different_net_coupons` (same factory, two bonds: IG 2%→600 net vs HY 10%→200 net on a 700 coupon). yield_vault gained `settlement`/`collateral_token` getters (wasm rebuilt).
- The tier is the unit of underwriter appetite: back a risk profile under one standard, not bespoke per bond. Documented in ECONOMICS.md + PRODUCTION_GAP G11.
- Full contract suite 51 tests green (bond 4, claim_settlement 8, factory 5, settlement 8, vault 9, yield_factory 3, yield_router 4, yield_vault 10).
- **Remaining:** shared-reserve-per-tier (correlated-default decision); the one-epoch-escrow alternative; G3 confidential-cover vault; G4 per-epoch snapshots; harden/CI for the new contracts. External/gated: live yield adapter, Blend, audit, (P)SPI counsel, founder/x86.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R47: harden + verify (CI fix for the new contracts + invariant + verify guide)

**Production push: consolidation of the large new contract surface.**
- **Fixed a real CI defect:** the contract test job ran `cargo test` but never built the wasm that the factory tests `contractimport!`, so those tests couldn't compile in CI. Split into a fast native job (`cargo test --workspace --exclude parallar-factory --exclude parallar-yield-factory` + the guests incl. claim/solvency/proptests) and a `contracts-wasm` job that runs `cargo build --release --target wasm32v1-none` (no stellar-cli needed) before the factory + yield_factory deploy tests. Both command sets verified locally.
- **Value-conservation invariant:** `yield_vault` test `premium_distribution_conserves_value` (Σ sellers' premium + protocol fee == premium paid, dust bounded by seller count). yield_vault 11/11.
- **VERIFY.md** extended with a money-flow section (verify yield_vault + yield_router + yield_factory; pointer to ECONOMICS.md).
- All suites green: 52 contract tests + the prover guests/host. The production build-out's new surface (claim_settlement, yield_vault, yield_router, yield_factory + the solvency/claim guests) is now CI-guarded + invariant-tested.
- **Remaining buildable:** G3 confidential-cover vault; G4 per-epoch snapshots; a shared-reserve tier option. External/gated: live yield adapter, Blend, audit, (P)SPI counsel, founder/x86.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R48: tranches — first-loss capital structure (G14, human-override)

**Per founder request: underwriters commit to different tranches with varying first-loss payout responsibility.** (PRD §4 v0.4+ non-goal; built under explicit override.)
- **Done:** `contracts/tranched_vault` (`parallar-tranched-vault`, a NEW instrument-family version; deployed + yield vaults stay frozen). Underwriters `deposit(tranche, amount)` by seniority rank (0 = junior/first-loss). On a default, `pay_allocations` absorbs the payout JUNIOR-FIRST (junior collateral consumed before senior). Premium split across tranches by configured weight (junior largest) then pro-rata within a tranche via a per-tranche SHARE model (a loss lowers collateral-per-share, spreading it pro-rata; premium accrues per share independently). 7 tests: junior-first absorption, senior protection, 3:1 premium-by-weight, intra-tranche pro-rata loss, loss-adjusted withdrawal, solvency floor, unknown-tranche reject.
- **Law #1 intact:** `pay_allocations` is settlement-only + proof-gated; the first-loss ordering is a pure accounting waterfall, never an admin payout path. Position-root binding + persistent accounting (TECH_SPEC §10).
- ECONOMICS.md (tranche section) + PRODUCTION_GAP G14 document it. Full contract suite 59 tests green (9 crates). CI fast job picks it up via `--workspace`.
- **Remaining:** factory `deploy_tranched`; a confidential-tranche variant; G3 confidential-cover vault; G4 record-date guest. External/gated: live yield adapter, Blend, audit, (P)SPI counsel, founder/x86.
- **Blocked:** none for buildable items.

## 2026-06-15 (cont.) — R49: factory deploys tranched families (deploy_tranched)

**Tranches made first-class in the factory model (the "one factory, many bonds" frame).**
- **Done:** `yield_factory::deploy_tranched(tier, cfg)` deploys a `tranched_vault` + settlement, cross-bound in one tx via the deployer pattern (salts tags 3/4, distinct from the protected family's 0/1/2). Premium still RISK-PRICED within the tier band; `cfg.weights` sets the seniority structure (rank 0 = junior). `set_tranched_wasm` (admin, one-time) registers the tranched wasm without changing the constructor surface. `TranchedConfig` / `TranchedInstrument` / `get_tranched` / `TranchedDeployed` event added. The SAME settlement binds transparently (identical `pay_allocations(epoch, allocations)`; the tranched vault absorbs it junior-first).
- Test `deploy_tranched_creates_cross_bound_tranched_family`: deploys a 3-tranche (junior/mezz/senior) high-yield family, asserts vault↔settlement binding, num_tranches=3, junior weight=3, and underwriters depositing into different tranches on the factory-deployed vault.
- ECONOMICS.md + PRODUCTION_GAP G14 updated (factory deploy built). Full contract suite 60 tests green.
- **Remaining:** a confidential-tranche variant; G3 confidential-cover vault; G4 record-date guest. External/gated: live yield adapter, Blend, audit, (P)SPI counsel, founder/x86.
- **Blocked:** none for buildable items.

## 2026-06-16 — R50: review-driven hardening (honesty + Law #1 coverage + TTL + reproducibility)

**An 11-agent comprehensive review (grounded, adversarial honesty pass + SDF strategist) flagged several checkable issues; fixed the buildable correctness/honesty items (the x86/founder items — video, 2nd on-chain proof, benchmark recapture — remain for the founder).**
- **#2 Verifier overclaim fixed (honesty):** the deployed verifier uses native BN254 pairing + sha256 only — Poseidon runs in the GUEST commitment layer, never on-chain. Reworded index.html / testnet.html / how-it-works.html / README.md (Poseidon stays correctly credited to commitments).
- **#3 Law #1 negative tests:** added `pay_allocations_requires_the_bound_settlement_auth` to yield_vault + tranched_vault and `claim_dist_fee_requires_admin` to yield_router (the `set_auths(&[]) -> try_* -> err` pattern). Previously the auth gate was proven for only 1 of the payout-capable contracts; claim_settlement's gate is proof-verification (already tested via `forged_claim_proof_reverts`).
- **#4 tranched_vault TTL bricking fixed:** it had ZERO `extend_ttl` (vs yield_vault 12). Added instance bumps on every state change + persistent bumps on write (set_i128 / position root / init), mirroring yield_vault — archival-class state can no longer expire (TECH_SPEC §10).
- **#5 Clawback-flag drift reconciled:** SECURITY.md + TECH_SPEC §3.0/§10.1 now describe the curated on-chain allowlist (a contract cannot read `AUTH_CLAWBACK_ENABLED` on-chain), matching PRODUCTION_GAP G6 + the code.
- **#11 Weather guest dev-dep removed (reproducibility):** relocated the cross-guest parity tests to `prover/proptests/tests/parity.rs`; `settle_weather_v1` now carries ZERO dev-deps (R35 — a guest dev-dep once shifted credit_v1's image_id).
- **#10 Demo robustness:** `run()` now fails on a zero-match filter (no more false-green on a renamed test); demo.sh falls back to `cargo build --target wasm32v1-none` when stellar-cli is absent (the build never needed it) — fresh-clone runnable.
- **#12 yield_factory instrument_id trust-delta documented:** it accepts instrument_id as a supplied field (vs the base factory's H-derivation); the guest's M1/M2 re-derivation is the soundness backstop (not a Law #1 break). Documented in code + this note.
- **Verified green:** 63 contract tests + 68 guest/proptest tests; frontend dash-free; demo.sh syntax ok; settlement mock-verify-free. Law #1/#2 independently grep-confirmed intact across all 9 contracts.
- **Remaining (x86/founder):** demo video (DoD), 2nd guest type proven live on testnet, scale.rs N=10 recapture + row-label check. **Remaining (buildable):** add onchain_verify to a CI job (#9). **Do NOT build:** the confidential-cover/tranche/record-date items (correctly remaining; keep as the SDF pilot workplan).
- **Blocked:** none for buildable items.

## 2026-06-16 (cont.) — R52: G3 confidential-cover vault (consumes solvency_v1)

**Full production build-out (founder override: build the whole protocol, keep the demo tight separately).**
- **solvency_v1 extended:** added the symmetric WITHDRAW check (`check_withdraw` / `WithdrawInputs` / `WithdrawJournal`, a length-distinct 48-byte journal vs the 112-byte buy journal) — proves the hidden aggregate still fits under post-withdrawal collateral, hiding the aggregate. 11 guest tests (buy + withdraw, insolvent-unprovable, wrong-opening, no-leak).
- **`contracts/confidential_vault` (NEW instrument-family version):** keeps the running AGGREGATE cover as a Poseidon COMMITMENT (no public total_cover getter). `buy_protection_proven` verifies a solvency_v1 purchase proof (advances the commitment, folds position_root, collects a DECLARED premium distributed rewards-per-share); `withdraw_proven` verifies a withdrawal proof (reserve can't drop below the hidden book); `pay_allocations` stays settlement-only + proof-gated (Law #1). 10 tests incl. forged-proof revert, wrong-prev-commitment, collateral>reserve, wrong-journal-length, premium distribution, proven withdrawal, and the Law #1 negative-auth test.
- **Honest scope (documented in the contract):** per-buyer position committed + aggregate book hidden (proven adequate); the declared premium is public but the chain never computes premium=cover×bps, so it doesn't reveal cover (adequacy priced by the keeper that supplies the aggregate opening — solvency_v1's coordination model). TTL discipline applied from the start (the tranched_vault lesson).
- 73 contract tests + solvency 11 green. confidential_vault is in the CI fast job via `--workspace`.
- **Next:** wire solvency_v1 into methods (ELF + image_id) + a host prove fn; confidential-tranche variant; G4 record-date guest; G2 factory deploy path.
- **Blocked:** none for buildable items (real solvency PROOF generation needs x86; the ELF build is local).

## 2026-06-16 (cont.) — R53: solvency_v1 wired to the zkVM (image_id + host + guard)

- **solvency_v1 now provable end-to-end:** added a `SolvencyRequest` dispatch enum (Buy | Withdraw); created the `guest-solvency` zkVM wrapper (`prover/methods/guest-solvency`) + registered it in the methods list. The methods build mints **`SOLVENCY_V1` image_id `c0b358d4606fa821…`** — and credit_v1 stays `705ddac4…` (adding a guest doesn't perturb the others; versioning law intact, confirmed by the guard).
- **Host prove path:** `prove_solvency_buy` / `prove_solvency_withdraw` (`prover/host`) return a `SolvencyProofArtifact` (seal + journal + digest, no cover) that the confidential_vault consumes. Two executor parity tests (buy + withdraw) confirm the zkVM guest commits the same journal as the native check — run locally; the Groth16 prove step runs on x86.
- **Guard:** `scripts/check_image_ids.sh` now asserts all 5 image_ids (incl. solvency) — passing.
- This closes the skeptic's "solvency_v1 has no ELF/image_id, the only guest not provable end-to-end" finding. G3 is now BUILT end-to-end (guest + image_id + host + the confidential_vault consumer). PRODUCTION_GAP G3 updated.
- **Next:** confidential-tranche variant; G4 record-date guest; G2 factory deploy path. **Remaining (x86):** generate a real Groth16 solvency proof + a live confidential settlement.
- **Blocked:** none for buildable items.

## 2026-06-16 (cont.) — R54: confidential-tranche variant (build sequence 2/4)

- **`contracts/confidential_tranched_vault` (NEW instrument-family version):** composes the two built primitives — CONFIDENTIAL cover (aggregate is a Poseidon commitment, advanced by a solvency_v1 proof on buy/withdraw; no public total_cover) + FIRST-LOSS TRANCHES (per-tranche share model, junior-first absorption, premium split by weight). The solvency bound is on the TOTAL reserve (Σ tranche collateral); tranches only order loss absorption, so one solvency_v1 proof serves the tranched reserve unchanged.
- 7 tests: confidential buy advances the hidden commitment + splits premium 3:1 by tranche weight; junior-first loss under confidential cover; proven tranche withdrawal (collateral_after must match the real post-withdraw reserve); forged-proof / wrong-prev / collateral-after-mismatch reverts; Law #1 negative-auth test. TTL discipline from the start.
- Law #1 intact: pay_allocations settlement-only + proof-gated (junior-first is a pure accounting waterfall); solvency proofs gate buy/withdraw, never a payout.
- 80 contract tests green (11 crates).
- **Next:** G4 record-date guest (credit_v3); G2 claimable-family factory deploy path.
- **Blocked:** none for buildable items.

## 2026-06-16 (cont.) — R55: G4 record-date guest credit_v3 (build sequence 3/4)

- **`settle_credit_v3` (NEW guest type, image_id `dd07a743…`):** extends credit_v2's attestation to the PER-EPOCH holder snapshot (record-date model for TRADED bonds). The issuer signs `sha256(epoch ‖ snapshot_digest ‖ payments_digest)` — whoever holds on the record date is the attested set; the snapshot is no longer pinned to `config.snapshot_root`; the epoch is in the signed message (no cross-epoch replay). Reuses credit_v1's generic primitives → the same vault/settlement/factory accept it unchanged.
- 8 native tests: attested record-date default settles; tampered-holder-set rejected (THE record-date guarantee); tampered-payments rejected; cross-epoch-replay rejected; wrong-key rejected; fully-paid unprovable; terms bind the key; two DIFFERENT record-date sets settle across epochs (the headline — holder set not fixed at issuance).
- **Wired to the zkVM:** image_id minted (credit_v1 stays `705ddac4…`); host `prove_credit_v3_settlement` + CLI `--guest credit-v3` (prove + bench) + executor parity test; `check_image_ids.sh` now guards all 6 image_ids.
- PRODUCTION_GAP G4 → BUILT. **Next:** G2 claimable-family factory deploy path (4/4). **Remaining (x86):** live credit_v3 settlement.
- Note: cleared the rebuildable contract `target/` to recover disk (Mac at ~100%); contracts rebuild on demand.
- **Blocked:** none for buildable items.

## 2026-06-16 (cont.) — R56: G2 claimable-family factory deploy path (build sequence 4/4 — COMPLETE)

- **`contracts/claim_factory` (`parallar-claim-factory`, a NEW factory version):** deploys the CLAIMABLE family — a vault + `ClaimableSettlement` (permissionless settle + buyer self-claim after a keeper-grace window), cross-bound in one tx. `register_claimable_type` records BOTH guest image_ids (settle + claim) + the grace window; `deploy_claimable` deploys + cross-binds behind the eligibility gate. Reuses the base factory's byte-identical config_hash / instrument_id derivation (reimplemented locally — depending on the factory rlib collides export symbols in the cdylib). 3 tests: cross-bound family, ineligible-collateral reject, immutable type.
- The base ParallarFactory + its frozen InstrumentType surface are untouched (Law #2); this is a sibling factory, matching the yield_factory pattern. Closes the review's "claim_settlement is built but unreachable via any deploy path" finding. PRODUCTION_GAP G2 deploy path → BUILT.
- CI: claim_factory joins the contracts-wasm job (it uses contractimport!). 83 contract tests green (12 crates).
- **BUILD SEQUENCE COMPLETE:** (1) G3 confidential-cover vault + solvency wiring; (2) confidential-tranche variant; (3) G4 record-date guest credit_v3; (4) G2 claimable factory.
- **Remaining = x86/founder only:** real Groth16 proofs (solvency, credit_v3) + live testnet settlements; the demo video; the N=10 benchmark recapture. Note: cleared rebuildable target/ dirs to manage disk (Mac near-full).
- **Blocked:** none for buildable items.

## 2026-06-16 (cont.) — R58: founder ops toolchain (witness generators + CLI completion + RUNBOOK)

**Answering "are the setup/testing scripts in place to make proof-gen, benchmarking, demo recording straightforward" — the audit found them PARTIAL; this closes the buildable gaps.**
- **Witness generators for the new proof types:** `prover/host/tests/gen_new_scenarios.rs` emits prove-ready witnesses for credit_v2 (G1), credit_v3 (G4 record-date), and solvency_v1 (G3 — buy + withdraw, plus the printed `initial_cover_commitment` for the confidential_vault init arg). Deterministic synthetic inputs (issuer demo key) so a founder can prove + benchmark with one command, no setup. Verified: all 3 generators run green, 4 witness JSONs emitted.
- **CLI completion:** all 6 guests now reachable. Added `GuestKind::Claim` (prove/bench) and a `prove-solvency` subcommand (solvency's SolvencyProofArtifact is distinct — no allocations). credit_v2/v3 were already wired (R55).
- **`docs/RUNBOOK.md`:** the x86 founder runbook — one-time setup, proof-gen per guest (table + worked examples), benchmarking (cycle + wall-clock), deploy, demo recording (with the layer-story shot list + the explicit demo-scope decision the audit asked for), submit. Honestly flags the remaining small gaps: claim has no generator yet (CLI path wired; mirror the test helper), the new families have no deploy script (drive stellar-cli per init), scale.rs benches only credit_v1/weather.
- **Earlier today (R57):** the final 8-agent audit returned GO-with-quick-fixes; fixed the one CI hole (credit_v3 untested) + reconciled the stale scope docs (PRD/PRODUCTION_GAP/README/VERIFY).
- **Verdict: push-ready** once the founder is content with the demo scope. Remaining is genuinely x86/founder-only: real Groth16 proofs, live testnet settlements of the new families, the N=10 wall-clock capture, the video, DoraHacks submission.
- **Blocked:** none for buildable items. (Disk: the 6-guest methods build is at this Mac's limit; `cargo clean` between phases.)
