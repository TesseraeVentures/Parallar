# Parallar — Sprint Plan v0.3 (June 13 – 29, 2026)

**Deadline:** Mon June 29, 12:00 PM PST. Internal deadline **June 28 EOD**. **P0 freeze June 24.**
**Change from v0.2:** factory/registry (R7) added to Sprint 1 — it replaces hand-written deploy plumbing so net cost ≈ +1 day; frontend stays at a 1-day cap; everything else holds.
**Golden rule:** threats to June 24 get cut, not negotiated. Cut order at bottom.

---

## Sprint 0 — Derisk (Sat June 13 – Sun June 14)

Environment + other people's examples only.

- [ ] Toolchains (Rust, stellar-cli, rzup); funded testnet accounts
- [ ] **Spike 1 — Groth16 path:** local x86 Docker wrap, 4h box → else remote proving / x86 VPS. Record in STATUS.md.
- [ ] **Spike 2 — Poseidon parity:** byte-compare guest-side vs Soroban host-function hash of identical input. Architecture-blocking. SHA-256 fallback if unmatched in a day.
- [ ] **Read (not build): Soroban deployer pattern** — deploy-from-contract, WASM hash upload flow, atomic deploy+init, deterministic salts. Capture the exact API in STATUS.md for the June 18 spike.
- [ ] **Stellar-mechanics verification (½ day, feeds TECH_SPEC §10):** confirm (a) how to read an asset's `AUTH_CLAWBACK_ENABLED` / auth flags from a contract (factory gate, §3.0); (b) classic payment ops vs SAC transfer events as seen from RPC — confirm history_builder can normalize both; (c) current testnet TTL defaults for persistent entries; (d) RPC event-retention window on testnet. Record findings + exact APIs in STATUS.md.
- [ ] Run unmodified: Nethermind stellar-risc0-verifier E2E (pin commit), soroban-examples groth16_verifier, Bachini tutorial
- [ ] #zk-chat + Telegram + DoraHacks registration; skim Protocol 25/26 ZK docs

**Exit:** external proof verified in a Soroban testnet contract on this setup AND Poseidon parity resolved.

## Sprint 1 — Contracts + Factory (Mon June 15 – Fri June 19)

**June 15:** Scaffold per TECH_SPEC §9. BondContract: SAC to 10 holders, terms/snapshot commitments, `pay_coupon` with partial payment list + real transfers, **no payments getter**. Tests incl. partial pay.
**June 16:** Generic VaultContract: deposits, commitment `buy_protection`, position_root accumulator, settlement-only `pay_allocations`, window freeze. Invariant tests.
**June 17:** Generic SettlementContract with **stubbed verification** (cfg flag): 116-byte journal decode, bindings, replay, allocation_root check, vault call. **Checkpoint: solvency C/B/A — decide and record.**
**June 18:** **Factory day.** Registry storage + `register_type` + `deploy_instrument` via deployer pattern (atomic deploy + init + cross-bind). Time-box the deployer atomicity fight to the day; fallback = registry-real / deploy-by-script (pre-agreed, README-documented).
**June 19:** Deploy through the factory on testnet; stub-verified settlement end-to-end **on a factory-deployed instrument**. `history_builder` working (payment ops from RPC).

**Exit:** factory-deployed instrument settles (stub) on testnet + history reconstruction. *Safety net + the layer story both established.*

## Sprint 2 — The Guest (Sat June 20 – Wed June 24)

**June 20:** `settle_credit_v1` part 1: snapshot verify, owed, payment scan, missed/short determination. Tests: full/partial/short/paid-panics/boundary.
**June 21:** Part 2: commitment opening, position_root recompute, pro-rata allocation + cap, generic journal. Tests: mismatch panics, cap, wrong-instrument binding panics. Host CLI `prove` started.
**June 22:** Host CLI complete → receipt → Groth16 wrap (Sprint 0 path). Benchmark at N=10, record. Real verifier replaces stub; redeploy via factory.
**June 23:** Full integration: factory deploy → partial default → prove → settle → confidential payouts. Then live negative paths: paid epoch (no proof possible), forged proof, replay, stale root.
**June 24:** **P0 FREEZE.** Solvency = whatever option works today (C not done → B or A, no exceptions). Three clean `demo.sh` runs including the second `deploy_instrument` (replication beat). Tag `v0.3-p0`.

**Slip rule:** P0 not frozen June 24 → June 25–26 are overflow, ALL P1 cancelled; the working `demo.sh` from the June 24 freeze (three clean runs) is the accepted DoD minimum — only its fresh-clone/one-command *hardening* (R10) is a target, not a gate. CLI demo with stellar.expert links is sufficient to win on substance.

## Sprint 3 — Polish (Thu June 25 – Sat June 27)

1. **June 25:** `demo.sh`/`reset.sh` hardened (fresh clone, one command). README complete: layer framing, architecture diagram, trust-model (faithful to TECH_SPEC §1), published formula, real-vs-mocked, benchmarks table, instance #2 section, PRODUCTION_GAP link.
2. **June 26:** Frontend (1-day hard cap: registry view + instrument page + settlement panel). Evening: video dry-run #1.
3. **June 27:** Exposure proof (R8) only if frontend landed by midday June 26; else polish + dry-run #2.

## Sprint 4 — Ship (Sun June 28 – Mon June 29)

**June 28:** Final video (script TECH_SPEC §7 — the layer story in the first 30 seconds; factory beat; panic-on-honest-data beat). Fresh-clone test from README alone. **Submit tonight.** Post in #zk-chat + Telegram.
**June 29:** Buffer only.

---

## Daily ritual
Tests green first → one vertical slice → `R<n>:` commits → 3-line STATUS.md → blocked >2h on Stellar internals = repro to #zk-chat.

## Cut order (pre-agreed)
1. Exposure proof (R8) → 2. Frontend (R9) → 3. Solvency C→B→A → 4. Factory deployer → registry-real/deploy-by-script fallback → 5. N=10 → N=5 → **never:** R1–R6, the commitment architecture, the generic journal/guest boundary, or the negative-path demos.
