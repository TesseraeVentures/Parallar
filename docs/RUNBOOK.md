# Parallar — founder runbook (x86: proof generation · benchmarking · deploy · demo)

Everything in this repo builds, tests, and verifies on any machine. **Real Groth16 proof
generation and live testnet settlement need an x86 host** (or Rosetta-x86) with the RISC Zero
toolchain — STARK→SNARK proving is x86-only, and each proof is ~minutes. This runbook is the
turnkey sequence for that host. (The committed `real_proof.json` fixture already lets `make demo`
and `make verify` run with no proving at all — see [VERIFY.md](../VERIFY.md).)

## 0 · One-time setup

```bash
# Rust + the RISC Zero toolchain (the prover/methods build cross-compiles the guests)
curl -L https://risczero.com/install | bash && rzup install
# Soroban CLI (deploy + submit); a funded testnet identity in a gitignored .env (Friendbot)
cargo install --locked stellar-cli
# sanity: the whole suite + the pinned image_ids
make test                 # 12 contracts + 6 guests, all green
./scripts/check_image_ids.sh   # all 6 image_ids reproduce (credit_v1 == the deployed 705ddac4…)
```

> **Disk note.** The 6-guest `parallar-methods` cross-compile is heavy (~8 GB of build artifacts).
> On a constrained machine, `cargo clean` the rebuildable `target/` dirs between phases. The x86 box
> should have ≥ 30 GB free.

## 1 · Proof generation (per guest type)

Each guest needs a **witness JSON**. Generators emit a prove-ready witness; then `parallar-prover
prove` runs the real Groth16 prover and writes a submittable artifact (260-byte selector-wrapped
seal + the journal). The witness generators are `#[ignore]` tests run with `--ignored --nocapture`.

| Guest | Witness generator | Prove command |
|---|---|---|
| `credit_v1` (deployed) | `--test gen_scenario` (needs `SCENARIO_XLM/BUYER/REFERENCE` testnet addresses) | `parallar-prover prove --guest credit --inputs witness.json --out proof.json` |
| `credit_v1` partial | `--test gen_partial_scenario` | `… --guest credit …` |
| `weather_v1` (G8) | `--test gen_weather_scenario` | `… --guest weather …` |
| `credit_v2` (G1 attested) | `--test gen_new_scenarios gen_credit_v2_witness` | `… --guest credit-v2 …` |
| `credit_v3` (G4 record-date) | `--test gen_new_scenarios gen_credit_v3_witness` | `… --guest credit-v3 …` |
| `solvency_v1` (G3 confidential) | `--test gen_new_scenarios gen_solvency_witnesses` | `parallar-prover prove-solvency --inputs buy_witness.json --out buy_proof.json` |
| `claim_credit_v1` (G2 escape hatch) | *(see note)* | `… --guest claim …` |

Example — generate + prove the record-date (`credit_v3`) instrument:

```bash
cd prover
cargo test -p parallar-prover-host --test gen_new_scenarios gen_credit_v3_witness -- --ignored --nocapture
# → /tmp/parallar_credit_v3/witness.json
cargo run -p parallar-prover-host -- prove --guest credit-v3 \
    --inputs /tmp/parallar_credit_v3/witness.json --out /tmp/credit_v3_proof.json
```

Example — confidential cover (`solvency_v1`), which the `confidential_vault` consumes:

```bash
cargo test -p parallar-prover-host --test gen_new_scenarios gen_solvency_witnesses -- --ignored --nocapture
# prints the confidential_vault init arg: initial_cover_commitment = commit_total(0,[1;32])
cargo run -p parallar-prover-host -- prove-solvency \
    --inputs /tmp/parallar_solvency/buy_witness.json --out /tmp/solvency_buy_proof.json
```

**Notes / known gaps (all small, x86-box work):**
- The new generators (`gen_new_scenarios`) use **deterministic synthetic** inputs (issuer demo key
  `[42;32]`, synthetic buyer) — perfect for proving + benchmarking the pipeline. For a
  **testnet-settleable** scenario (real `G…` buyer in the position commitment, the real issuer
  signing key), adapt them the way `gen_scenario.rs` reads `SCENARIO_*` env addresses.
- **`claim_credit_v1` has no dedicated generator yet** — the CLI path (`--guest claim`) is wired,
  but you must hand-build a `ClaimInputs` witness (public commitments + the claimant's own opening +
  the folded `position_root`). Mirror the `claim_inputs()` test helper in `prover/host/src/lib.rs`.
- `history-builder` assembles a `credit_v1` witness from a payment-scan + a params template
  (`parallar-prover history-builder --scan … --params … --out witness.json`).

## 2 · Benchmarking

Two layers. **Cycle counts** are hardware-independent and run anywhere (they report zkVM
instructions, the figure quoted in the README/§10.7). **Wall-clock** proving needs the x86 box.

```bash
# hardware-independent cycle counts (the committed determination figures)
cd prover && cargo test -p parallar-prover-host --test scale -- --ignored --nocapture

# wall-clock N=10 over a witness (the DoD benchmark; ~minutes/run on x86)
cargo run -p parallar-prover-host -- bench --guest credit --inputs witness.json --n 10
```

**Gap:** `scale.rs` benches only `credit_v1` + `weather_v1` today; `bench` (CLI) now covers
credit/weather/credit-v2/credit-v3/claim. Add scale rows + a solvency wall-clock path if you want
cycle figures for every guest — otherwise quote credit_v1 + weather and note the others are the
same determination order of magnitude. **Capture the numbers into a committed file** (the DoD asks
for printed benchmarks; none is committed yet) and confirm the README rows match the live output.

## 3 · Deploy to testnet

```bash
./scripts/deploy_testnet.sh     # credit_v1 family (factory + verifier + a credit instrument)
./scripts/deploy_weather.sh     # weather_v1 (same base factory, new type)
./scripts/ttl_monitor.sh        # keep the deployed instruments' archival state alive (run periodically)
```

**Gap — the new families have no deploy script yet** (test-green only, never on testnet):
`yield_factory` (protected + tranched — 5-arg constructor, `register_tier`, `set_tranched_wasm`,
`deploy_protected`/`deploy_tranched`), `claim_factory` (`register_claimable_type` +
`deploy_claimable`), and the confidential vaults (NOT factory-deployed — each `init` takes a
hand-computed `initial_cover_commitment = commit_total(0, salt0)`, printed by the solvency
generator). To deploy one live, drive `stellar contract deploy/invoke` directly per the contract's
`init`/constructor signature, then settle with the matching proof from §1. Add `deploy_yield.sh` /
`deploy_claim.sh` / `deploy_confidential.sh` (mirror `deploy_testnet.sh`) if you want them turnkey,
and extend `ttl_monitor.sh` + `deployments/testnet.json` to track the new instruments.

## 4 · Record the demo (the 2–3 min DoD video)

```bash
make demo        # the full local walkthrough (fresh-clone, no proving needed — uses the fixture)
```

**Layer story in the first 30 seconds**, then the money shots (see the strategist beats in
`docs/COMPETITION.md` / the audit): the real on-chain `settle_tx` on stellar.expert → the
**unprovable** fully-paid epoch (no proof, no payout) → a forged-proof revert → instance #2 on the
unchanged core.

**Decision to make before recording — demo scope.** `demo.sh` currently exercises the original
`credit_v1` flow (register → 2× factory-deploy → committed cover → fully-paid-unprovable → partial
default → real on-chain verify → forged/replay/stale reverts → benchmarks). It does **not** show the
R52–R56 families (confidential cover, tranches, `credit_v3` record-date, `claim_factory`). Either:
- **(a)** keep the demo tight to the core narrative (recommended for a 2–3 min video) and *say*
  "the same core now carries a family of instruments — confidential cover, tranches, record-date,
  escape-hatch — all on the unchanged surfaces" with the test suite as proof; or
- **(b)** add `run()` beats to `demo.sh` driving the (already-green) tests for the new families.

There is **no separate screencast/capture script** — record with your own tool (asciinema/OBS).

## 5 · Submit

DoraHacks by **June 28 EOD**. The submission rests on: the live `settle_tx` (stellar.expert link),
this repo (12 contracts + 6 guests, all tests green, all 6 image_ids reproducible), the README +
VERIFY trust story, and the video. `docs/PRODUCTION_GAP.md` is the post-hackathon pilot workplan —
the SDF grant/audit-support conversation maps onto it 1:1.

---
*This runbook is the operational counterpart to [PRODUCTION_GAP.md](PRODUCTION_GAP.md) (what's built
vs remaining) and [VERIFY.md](../VERIFY.md) (how anyone checks the claims).*
