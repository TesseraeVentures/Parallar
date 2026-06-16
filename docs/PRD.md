# Parallar — Product Requirements Document

**Product:** Parallar — a verifiable settlement layer on Stellar. Credit default instruments are product #1.
**Version:** v0.3 (Hackathon MVP, production-architecture-first)
**Target:** Stellar Hacks: Real-World ZK (submissions June 15 – 29 2026, 12:00 PM PST)
**Owner:** Ben (solo founder + Claude Code)
**Status:** Approved for build

> **Scope note (updated 2026-06-16).** This is the v0.3 June-13 baseline. Under explicit human override *after* the P0 freeze, the protocol was built out well beyond this document: instance #2 (weather), attested feeds (`credit_v2`/`credit_v3`, G1/G4), the self-claim escape hatch (`claim_direct`, G2), confidential cover (`solvency_v1` + `confidential_vault`, G3), first-loss tranches incl. a confidential variant (G14), and the yield router/factory distribution layer (G11/G12). Several §4 non-goals and §6/§8 entries below therefore **lag the code**. [PRODUCTION_GAP.md](PRODUCTION_GAP.md) + [STATUS.md](STATUS.md) are the live source of truth for built-vs-remaining; the two architectural laws and the four frozen surfaces held through every addition.

**Change log v0.2 → v0.3:**
- Reframed: Parallar is a **verifiable settlement layer** — a factory-deployed family of instruments sharing one settlement architecture. Credit protection is instance #1; weather/parametric indices are named instance #2.
- **Factory model is P0 (R7):** instrument types registered (guest image ID + WASM hashes + rules version); instrument instances (vault + settlement pair) deployed and cross-bound in one transaction via the Soroban deployer pattern.
- Core vocabulary generalized: `instrument_id`, `epoch` (the credit instance maps these to bond/coupon). Journal layout unchanged in size, renamed in meaning.
- New `PRODUCTION_GAP.md`: the enumerated checklist between hackathon demo and a live client issuance pilot — "production-right first time" means the **architecture** is final; the gap register is what hardening means.
- Image IDs versioned in the registry from day one: a new guest version is a new type entry, and existing instruments stay pinned to their image_id forever — never re-pointed. Adopting a new guest means deploying new instruments; only the type catalog grows without redeploys.

**Language precision (use everywhere, incl. with regulators):** Parallar provides **verifiable execution** — cryptographic proof that each specific settlement followed the published rules — not formal verification of the code. The claim constrains the operator, not just the code: no settlement can deviate from the rules and still verify.

---

## 1. Problem Statement

Tokenized RWAs are arriving on Stellar, but the instruments built *on top of* them — protection, hedges, structured payouts — have no trustworthy settlement layer. Every such instrument faces the same three problems:

1. **Determination is heavy.** "Did the trigger event occur?" (a missed coupon across 1,000 bondholders; an index breach over a data window) is computation no smart contract can run.
2. **Institutions won't write or hold positions on a transparent book.** Public position sizes leak credit exposure and strategy.
3. **Settlement requires trust.** Someone computes the payouts. Today that someone is a committee, an operator, or an admin key.

Solving this per-instrument is wasteful and unauditable. The cost of not solving it: RWAs on Stellar stay tokenized but inert — no hedging, no structuring, no institutional depth.

## 2. Product Concept

**Parallar is a verifiable settlement layer: a factory of instruments whose payouts are computed off-chain over confidential positions and proven correct on-chain.**

The layer has three parts:
1. **The factory/registry** — instrument *types* are registered (a settlement guest's image ID + contract WASM hashes + a published-rules version). Instrument *instances* (a vault + settlement pair bound to a reference asset and trigger config) deploy in one transaction.
2. **The settlement core** — generic vault (Poseidon-committed positions, public aggregates) and settlement contract (Groth16 verification against the type's pinned image ID, binding checks, replay guard, allocation execution). Identical for every instrument type.
3. **Pluggable settlement guests** — per-type RISC Zero programs encoding the determination + payout rules. **Instance #1 (built):** parametric credit protection — scan bond payment history across N holders, detect missed/short coupon, pay pro-rata to shortfall over committed covers. **Instance #2 (BUILT under override):** parametric weather/index protection on the same core — `settle_weather_v1` (image_id `d31246e6`), parity-tested byte-identical to credit_v1's generic surfaces.

ZK is structurally necessary, twice, for every instrument type: the determination is too heavy for on-chain execution, and payout over hidden positions is impossible on-chain by construction. Remove the proof and no instrument in the family can exist.

**Trust model (faithful to TECH_SPEC §1 — the canonical statement — everywhere):** each proof guarantees the settlement *computation* over the supplied input data is correct; it does not yet guarantee the input data is canonical. Verifiable execution means correct computation on inputs, not fair inputs. Attested data feeds are the named next layer (see PRODUCTION_GAP.md G1). Within the proof boundary, no party — including the operator — can inflate, favor, omit, or fabricate a payout.

## 3. Goals

| # | Goal | Measure |
|---|------|---------|
| G1 | End-to-end verifiable settlement of the credit instrument on testnet, deployed **via the factory** | Demo: one factory tx deploys the instrument; partial default → one proof → confidential payouts |
| G2 | ZK structurally necessary (judge criterion) | No code path computes payouts from public state; verification is sole authorization |
| G3 | The layer story is shown, not told | Demo includes `factory deploy` as a beat; README documents instance #2 against the same interfaces |
| G4 | Win or top-3 placement | Judging |
| G5 | **Zero architectural rebuild for production** | Post-hackathon path = PRODUCTION_GAP.md checklist only; contract separation, journal, registry, guest interface all final |
| G6 | Founder fluency in the full stack | Founder can explain and modify every component |

## 4. Non-Goals (v0.3)

| Non-goal | Why |
|----------|-----|
| Instance #3 (trade settlement) implementation | Post-hackathon (instance #2 weather was **BUILT** under override — `settle_weather_v1`, G8) |
| Full oracle/attested data integration | First cut **BUILT** under override (`credit_v2` issuer-signed payments, `credit_v3` per-epoch record-date — G1/G4); full Reflector/RedStone wiring is post-hackathon |
| Buyer-held openings & self-claim escape hatch (`claim_direct`) | **BUILT** under override (`claim_credit_v1` + `claim_settlement` + `claim_factory` deploy path — G2); production wallet-held openings remain post-hackathon |
| Real client bond integration | No client paper in a public repo; mock issuance modeled on a live Stellar corporate bond, unnamed |
| Audit, key management, mainnet | Sequenced after ecosystem support + counterparty commitment (PRODUCTION_GAP G5–G7) |
| Bonding curves, standalone pricing engine | v0.4+ (multi-tranche vaults were **BUILT** under override — `tranched_vault` + `confidential_tranched_vault`, G14; `yield_factory` does tier-band risk pricing, not a bonding curve) |
| Rollup/L2 ambitions | Parallar is a settlement layer *on* Stellar, not a parallel chain. Never imply otherwise |
| Sequencer/batching | One-shot settlement per epoch is the model |

## 5. Users & User Stories

**Personas:** Instrument Deployer (issuer/structurer), Protection Buyer (institutional hedger), Protection Seller (underwriter), Keeper (anyone).

P0:
1. As an **instrument deployer**, I want to deploy a new protected instrument from a registered type in one transaction, so that structuring on Parallar is replication, not engineering.
2. As an **instrument deployer**, I want each type pinned to a versioned guest image ID and published rules, so that counterparties know exactly which settlement logic governs.
3. As a **protection seller**, I want to deposit collateral and earn premiums on instruments I choose, so that I'm paid for underwriting.
4. As an **institutional buyer**, I want cover recorded only as a commitment, so the market cannot read my exposure.
5. As a **keeper**, I want to settle any instrument permissionlessly with a proof, so that no operator is in the payout path.
6. As a **buyer/seller**, I want settlement provably executed per the type's published rules — payouts can't be inflated, favored, omitted, or fabricated.
7. As a **seller**, I want no proof to exist when the trigger didn't occur, so my collateral can't be drained.

P1:
8. As an **observer/judge**, I want a minimal UI: registry, instrument status, settlement flow.
9. As a **buyer**, I want to prove eligibility (holdings ≥ cover) without revealing my balance.

## 6. Requirements

### P0 — Must-have

| ID | Requirement | Acceptance (abbrev.) |
|----|-------------|----------------------|
| R1 | **Reference bond issuance** (mock, realistic): SAC with 10 demo holders; coupons = real transfers; partial payment representable; terms + holder-snapshot commitment on-chain; **no payments getter** — determination from history only | Issuer short-pays epoch 1 (7 of 10 holders); history reconstructable from ledger |
| R2 | **Generic vault**: seller deposits public; positions as Poseidon commitments only; position_root accumulator; `pay_allocations` callable solely by its settlement contract; solvency per §3.2 options; withdrawals frozen in settlement windows | Public state = commitments + aggregates; invariants tested |
| R3 | **Credit settlement guest** (`settle_credit_v1`): snapshot verification → owed computation → payment scan → missed/short determination → open commitments → pro-rata allocation per published formula → cap assertion → generic journal | Trigger-occurred → valid proof + correct allocations; fully-paid → guest panics, no proof exists |
| R4 | **Generic settlement contract**: Groth16 verify against the instrument type's pinned image_id; binding checks (instrument_id, epoch, position_root, allocation_root); execute allocations | Valid → payouts; forged/mismatched/stale → revert |
| R5 | **Replay & binding**: one settlement per epoch; proof bound to instrument_id + epoch + deadline + exact position_root | Resubmission and stale-root submissions revert |
| R6 | **Submission package**: repo, README (trust model, real-vs-mocked, published formula, benchmarks), 2–3 min video | Submitted by June 28 EOD internal deadline |
| R7 | **Factory & registry**: `register_type(type_id, image_id, rules_version, rules_uri, vault_wasm, settlement_wasm)`; `deploy_instrument(type_id, config)` deploys + initializes + cross-binds a vault/settlement pair via Soroban deployer; registry getters; image-ID versioning (new guest version = new type entry; instances pinned forever to their version) | One tx → live instrument; demo deploys the credit instrument through it; second `deploy_instrument` call shown to prove replicability |

### P1 — After P0 freeze only

| ID | Requirement |
|----|-------------|
| R8 | Exposure proof guest (eligibility without balance reveal) |
| R9 | Frontend: registry view + instrument page + settlement flow (read-only, 1-day cap) |
| R10 | `demo.sh`/`reset.sh` **fresh-clone hardening only** — a working `demo.sh` already ships at P0 freeze; treat hardening as P0-adjacent (the video depends on it) but not a gate under a slip |

### P2 — Designed-for (much of this was BUILT under override post-freeze — see PRODUCTION_GAP/STATUS)

> Built since this list was written: the yield router / protected share class (`yield_vault` + `yield_router` + `yield_factory`, G11/G12), instance #2 (`settle_weather_v1`), `credit_principal_v1` (demonstrated as a `credit_v1` config at 100% over the maturity epoch — no new guest), attested feeds (G1), and the escape hatch (G2). Items genuinely still unbuilt: instance #3 (trade settlement), premium/secondary markets, governance over type registration.


- **Yield router / protected share class (the distribution layer):** holders wrap bond tokens into a protected receipt (pBOND); coupons route through a waterfall — premium to the protection vault, net yield to the holder (e.g., 14% gross → 12% protected) — with cover auto-sized to wrapped balance and auto-renewed per epoch. Sits upstream of the unchanged settlement core. **Economics:** premium flows primarily to protection sellers; Parallar's take is the layered fee model in TECH_SPEC §5A — a ~12% base rail fee on all premium plus a +10–15% distribution fee on premium it originates via the protected share class (blended ≈22–27% on distributed flow), with structuring fees and ~10% of reserve float yield on top — this is Parallar's revenue model. Two honest tensions documented in TECH_SPEC §5A: router-visible balances (resolved as two product lines: transparent protected share class vs sealed bespoke cover) and premium-in-arrears (priced in, or one epoch escrowed upfront)
- Instance #2: weather/index settlement guest on the unchanged core (whether.market convergence)
- **`credit_principal_v1`** — principal protection (failure-to-redeem at maturity) on the unchanged core; the prerequisite for the full pBOND-as-enhanced-collateral thesis (TECH_SPEC §5A, PRODUCTION_GAP G13: higher LTV in external lending markets, loop-multiplied premium throughput, on-chain-provable reserve adequacy as the lender pitch). Coupon-only cover (`credit_v1`) supports partial uplift and is presented to lenders as exactly that
- Instance #3: **commodity trade settlement** (`trade_settlement_v1`) — provisional-to-final invoice computation for physical cargoes (quotational-period pricing, assay-based quality adjustments, laytime/demurrage), buyer funds as the funded reserve, payment released by certificate. Confidential terms are the institutional requirement; attested inputs (G1: signed eBL/DCSA documents, inspection certificates, licensed index data) are the centerpiece dependency, not residual. Note: plain condition-release escrow is explicitly NOT a Parallar use case — it fails the load-bearing test; the product is the settlement *computation*
- Attested data feeds (G1) and buyer-held openings + escape-hatch self-claims (G2) — both in PRODUCTION_GAP.md. Tranches/bonding curves are a §4 non-goal (v0.4+), not a tracked gap
- Premium/secondary markets; governance over type registration

## 7. Success Metrics

**At submission:** factory-deployed instrument settles end-to-end, 3 clean runs; second instrument deployment shown; proof time + verification fee benchmarked (N=10 measured, 1k extrapolated); negative paths on video; video < 3 min with the layer story in the first 30 seconds.
**Post:** placement; SDF RWA conversation anchored on the demo; PRODUCTION_GAP.md becomes the literal workplan for the client-bond pilot.

## 8. Open Questions

| Question | Owner | Status |
|----------|-------|-----------|
| Groth16 wrap path (local x86 vs remote proving) | Sprint 0 | **RESOLVED** — local x86 via Rosetta; 260-byte selector-wrapped seal, selector `73c457ba` |
| Poseidon parity guest↔host parameterization | Sprint 0 spike | **RESOLVED** — Poseidon2 BN254 t=3 parity GREEN (guest↔host↔wasm) |
| Nethermind verifier exact API | Sprint 0 | **RESOLVED** — `verify(seal, image_id, journal_digest)`; runs on-chain ~35M insns |
| Soroban deployer pattern: init-in-deploy atomicity, wasm-hash upload flow | Sprint 1 spike | **RESOLVED** — atomic deploy+cross-init works (R7 + every factory since) |
| Solvency option C/B/A (TECH_SPEC §3.2) | June 17 | **RESOLVED** — Option B shipped; Option C also BUILT (`solvency_v1` + `confidential_vault`, G3) |
| Registry governance for type registration (admin key for MVP; what for production?) | Founder, post-hackathon (PRODUCTION_GAP G6) | Open (non-blocking) |

## 9. Timeline

Hard deadline June 29 12:00 PM PST; internal June 28 EOD; **P0 freeze June 24**. Factory adds ~1 day to Sprint 1 (it replaces hand-written deploy plumbing, so net cost is below a day). Cut order pre-agreed in SPRINT_PLAN; R7 sits behind only R1–R6 in protection — if the deployer pattern fights back past its time-box, the fallback is factory-as-registry (registry real, deployment via script) with the limitation documented. The architecture goal G5 survives either way: the registry interface is the production contract surface.
