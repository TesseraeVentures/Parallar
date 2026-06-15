# CLAUDE.md — Parallar

Instructions for Claude Code in this repository. Read `docs/PRD.md`, `docs/TECH_SPEC.md`, `docs/SPRINT_PLAN.md`, `docs/PRODUCTION_GAP.md` (all v0.3) before implementation. Specs are source of truth; this file is how to work.

## What this is

Parallar: a **verifiable settlement layer on Stellar**. A factory/registry deploys instrument instances (generic vault + generic settlement contract) from registered types; per-type RISC Zero guests prove that each settlement — determination over real data, payouts over **Poseidon-committed confidential positions** — followed the type's version-pinned published rules; the settlement contract verifies one Groth16 proof and executes. Instance #1 (built): parametric credit protection on a mock tokenized bond. Hackathon deadline **June 29 2026 12:00 PM PST; internal June 28 EOD; P0 freeze June 24.**

## The two architectural laws

1. **ZK stays structurally necessary.** No code path may let the chain or any admin compute/authorize payouts from public state — no payments getter on the bond, no plaintext covers, no settlement override, no pause-and-pay. A simplification that makes verification skippable is wrong even if easier; flag it.
2. **The four production surfaces are final.** Contract separation (factory / bond / vault / settlement), the 116-byte generic journal, the registry interface, and the guest plug-in contract (TECH_SPEC §4) do not change for convenience. MVP simplifications must be replaceable behind these surfaces (that's what PRODUCTION_GAP.md enumerates). If an implementation difficulty seems to require changing a surface, stop and present options — don't quietly bend it.

## Anti-gold-plating rule

The core is exactly as generic as the journal + guest contract require. No trait hierarchies "for future instruments," no config DSLs, no premature abstraction — instance #2 is served by the existing surfaces by design, not by speculative flexibility. When tempted to generalize beyond the spec: don't; note it in STATUS.md instead.

## Non-negotiable constraints

1. **P0 first (R1–R7).** No P1/P2 before the June 24 freeze without explicit human override. Asked early for tranches, attested feeds, escape hatch, instance #2, or pricing → cite PRD non-goals, propose deferral, await confirmation. (R7's registry *interface* is P0/never-cut; only its deployer-pattern *implementation* carries the pre-authorized registry-real/deploy-by-script fallback — SPRINT_PLAN cut order.)
2. **Testnet only; no real keys.** Friendbot keys in gitignored `.env`.
3. **No client names anywhere** (code, comments, README, commits). Reference bond = "modeled on a live Stellar corporate issuance, unnamed."
4. **Honest trust model everywhere.** Proofs guarantee computation over supplied inputs, not input canonicity. TECH_SPEC §1 is the canonical trust statement; the README trust-model section stays **faithful to §1** — no weaker, never overclaiming input canonicity — and survives every edit; mocks labeled at point of occurrence.
5. **Versioning rule is law:** new guest = new type entry; live instruments are pinned to their image_id forever. Never implement in-place image upgrades.
6. **External examples run unmodified first** (Nethermind verifier, soroban-examples, deployer examples); pin commits in STATUS.md.
7. **Sprint 0 decisions are settled** (Groth16 path, Poseidon parameterization). Re-open only with evidence + options, never silently.

## Working agreements

- Vertical slices; repo runnable + tests green at every session end.
- **Stub discipline:** settlement `mock-verify` cfg exists for Sprint 1 only; real verification is default from June 22; never demo/record on the stub.
- **Invariants never losing coverage:** payouts only via verified settlement · one settlement per epoch · allocations calldata hashes to the proof's allocation_root · position_root binding · trigger-didn't-occur admits no proof · Σ payouts ≤ collateral · factory-deployed pair correctly cross-bound · instrument pinned to type image_id · factory rejects clawback/freezable collateral assets · clawed-back coupon counts as shortfall · frozen holder counts as shortfall · no-trustline holder excluded from owed.
- **Stellar mechanics are law (TECH_SPEC §10):** position commitments, roots, settled flags, registry → archival-class storage (persistent or instance), NEVER temporary; qualifying payments are asset-received basis (classic ops AND SAC transfers, muxed addresses normalized); deadlines are ledger close timestamps; history_builder never assumes default-RPC retention covers the window.
- Guests are ordinary, readable Rust; benchmark at N=10 before optimizing anything.
- Commits `R<n>: <what>`; STATUS.md 3 lines per session (done / next / blocked).

## Calendar-bound checkpoints

- **June 17:** solvency C→B→A decision (TECH_SPEC §3.2); record choice + rationale.
- **June 18:** factory deployer-pattern time-box = the day; fallback registry-real/deploy-by-script is pre-authorized.
- **June 24:** P0 freeze — whatever solvency option works ships.
- **June 26 midday:** exposure proof go/no-go, contingent on frontend.

## When stuck

Stellar internals >2h → minimal repro for the human to post in #zk-chat. Fee/limit issues → measure, report numbers, then propose. Schedule pressure → SPRINT_PLAN cut order; never cut R1–R6, the commitment architecture, the generic surfaces, or negative-path demos.

## Definition of done

- [ ] Fresh-clone `demo.sh`: register type → **factory-deploy instrument (×2 — replication beat)** → deposits → committed cover → epoch 0 fully paid (**no proof possible**, shown) → epoch 1 partial default → prove → verify → confidential payouts → forged/replay/stale-root reverts → benchmarks printed
- [ ] README: layer framing, diagram, published formula, trust model, real-vs-mocked, benchmarks (N=10 + extrapolation), instance #2 section, PRODUCTION_GAP link
- [ ] 2–3 min video; layer story in first 30 seconds
- [ ] Submitted on DoraHacks by June 28 EOD
