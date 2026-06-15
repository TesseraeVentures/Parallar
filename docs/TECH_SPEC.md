# Parallar — Technical Specification

**Version:** v0.3 (verifiable settlement layer + factory) · **Companions:** `PRD.md`, `SPRINT_PLAN.md`, `PRODUCTION_GAP.md`, root `CLAUDE.md`

**Design stance:** this is production architecture built at hackathon speed. Contract separation, the journal format, the registry interface, and the guest plug-in boundary are **final** — post-hackathon work hardens (PRODUCTION_GAP.md), it does not restructure. Anything marked MVP-simplification must be replaceable without touching these four surfaces.

---

## 1. System Overview

```
                        ┌──────────── ParallarFactory / Registry ────────────┐
                        │ types: type_id → { image_id, rules_version,        │
                        │                    vault_wasm, settlement_wasm }    │
                        │ deploy_instrument(type_id, config) ── one tx ──┐    │
                        └────────────────────────────────────────────────┼────┘
                                                                         ▼
┌──────────────── OFF-CHAIN (per settlement) ───────────┐   ┌── instrument instance ──┐
│ history_builder: input data from RPC/Horizon          │   │  SettlementContract      │
│ prover host (risc0): runs the TYPE's guest            │   │   ├ Groth16 verify vs    │
│   instance #1: settle_credit_v1                       │   │   │  type's image_id     │
│    1 verify reference commitments (snapshot, terms)   │──►│   ├ bindings + replay    │
│    2 determination (scan payment history, epoch n)    │   │   └ execute allocations  │
│    3 open position commitments (private)              │   │  VaultContract           │
│    4 compute payouts per published rules vX           │   │   ├ collateral (public)  │
│    5 assert Σ ≤ collateral                            │   │   ├ Poseidon commitments │
│    6 commit generic journal                           │   │   └ pay via Settlement   │
│ STARK → Groth16 wrap → proof + journal                │   └─────────┬───────────────┘
└───────────────────────────────────────────────────────┘             │ reference
                                                          ┌───────────▼─────────────┐
                                                          │ BondContract (instance  │
                                                          │ #1 reference: mock SAC  │
                                                          │ issuance, N holders,    │
                                                          │ real coupon transfers)  │
                                                          └─────────────────────────┘
```

**Trust model (canonical statement — README stays faithful to this per CLAUDE.md law #4):** each proof guarantees that the instrument type's settlement rules (version-pinned) were executed correctly over the supplied input data and the committed position set. It does **not** guarantee the input data is canonical — verifiable execution means correct computation on inputs, not fair inputs. Mitigations today: permissionless keeping + independent on-chain deadline enforcement. The real fix is attested data feeds (PRODUCTION_GAP G1). Within the proof boundary, no party — including the operator — can inflate, favor, omit, or fabricate a payout.

## 2. Toolchain

| Component | Choice | Notes |
|-----------|--------|-------|
| Contracts | Rust + `soroban-sdk`, `stellar-cli`, testnet | Factory uses the **Soroban deployer pattern** (deploy-from-contract via uploaded WASM hashes, deterministic salt, atomic init) — Sprint 0 reading + Sprint 1 spike |
| zkVM | RISC Zero | Guests are ordinary Rust — the reason zkVM beats hand-written circuits for rule-heavy settlement logic |
| Proof on-chain | Groth16 / BN254, Protocol 25–26 host fns | `NethermindEth/stellar-risc0-verifier` pattern; pin commit |
| Commitments | Poseidon (Protocol 25 host fn) | Guest↔host parity = Sprint 0 blocking spike; SHA-256 fallback documented |
| Groth16 wrap | Local x86 Docker or remote proving | Sprint 0 decision, 4h box, recorded, never re-litigated |
| Frontend | React/Vite + stellar-sdk JS, read-only | P1, 1-day cap |

## 3. Contract Specifications

### 3.0 ParallarFactory / Registry (R7)

Storage:
- `admin: Address` (type registration auth — MVP governance; production: PRODUCTION_GAP G6)
- `types: Map<Symbol, InstrumentType { image_id: BytesN<32>, rules_version: u32, rules_uri: String, vault_wasm: BytesN<32>, settlement_wasm: BytesN<32> }>`
- `instruments: Map<BytesN<32>, Instrument { type_id, vault: Address, settlement: Address, reference_config_hash }>`

Interface:
```rust
fn register_type(env, type_id: Symbol, t: InstrumentType);          // admin
fn deploy_instrument(env, type_id: Symbol, config: InstrumentConfig)
    -> (BytesN<32> /*instrument_id*/, Address /*vault*/, Address /*settlement*/);
fn get_type(env, type_id) -> InstrumentType;
fn get_instrument(env, instrument_id) -> Instrument;
```

`deploy_instrument` semantics (one transaction, atomic):
1. Derive deterministic salts from `(type_id, config_hash, nonce)`.
2. Deploy vault + settlement from the type's WASM hashes via `env.deployer()`.
3. Initialize both with cross-bindings: settlement gets `{image_id, instrument_id, vault}`; vault gets `{settlement, collateral_token, params}`.
4. `instrument_id = H(type_id ‖ rules_version ‖ config_hash)` — this is what guests bind proofs to.
5. **Asset-policy gate:** refuse deployment if the collateral asset has `AUTH_CLAWBACK_ENABLED` (or issuer-revocable auth that could freeze the vault's balance). A protection vault whose collateral can be clawed back or frozen by a third party is hollow; the factory enforces this at birth (§10.1). XLM and verified non-clawback stablecoins pass.
6. Record in `instruments`; emit `instrument_deployed`.

**Versioning rule (final):** a new guest version is a **new type entry** (`credit_v2`); existing instances stay pinned to their image_id forever. No in-place image upgrades — settlement logic for a live instrument is immutable by construction. This is the institutional guarantee and it costs nothing to enforce from day one.

`InstrumentConfig` (instance #1): reference asset address, terms hash, schedule root, holder snapshot root, collateral token, premium bps, epoch deadlines.

Fallback (pre-agreed, SPRINT_PLAN): if deployer-pattern atomicity fights past its time-box → registry stays fully real, deployment happens via script, factory records the instance. The registry interface (the production surface) is unchanged; the limitation goes in README + PRODUCTION_GAP.

### 3.1 BondContract — instance #1 reference (mock issuance, realistic mechanics)

As v0.2, unchanged in substance: real SAC with N holders; `pay_coupon(epoch, payments: Vec<(Address, i128)>)` executes genuine transfers and supports partial payment; terms/schedule/snapshot as Poseidon commitments; **no payments getter** — determination must come from ledger history or ZK becomes decorative (the one architectural law). Holder snapshot fixed at issuance (MVP simplification, documented).

### 3.2 VaultContract (generic — identical WASM for every instrument type)

As v0.2: public seller collateral + aggregates; buyer positions only as Poseidon commitments `H(buyer ‖ cover ‖ salt)`; `position_root` accumulator; `pay_allocations(epoch, Vec<(Address, i128)>)` callable solely by the bound settlement contract; withdrawals frozen during settlement windows.

**Solvency with hidden covers — decide June 17:**
- **C (preferred):** purchase-time mini-proof: `new_total_cover ≤ total_collateral` over the aggregate commitment.
- **B:** running public aggregate cover (reveal in tx footprint; documented).
- **A (floor):** public cover tiers {1k, 5k, 10k}.
Degradation C→B→A pre-authorized if the freeze is threatened.

MVP opening escrow: buyers' `(cover, salt)` openings provided to the demo keeper via local file — the chain never sees them. Production: buyer-held openings + self-claim escape hatch (PRODUCTION_GAP G2).

**Reserve yield & the non-circularity covenant (sketched; production = G12):**
Reserves run as insurance float: held in yield-bearing eligible assets (tokenized T-bill/MMF tokens — e.g. BENJI-class instruments on Stellar, subject to the clawback gate) so underwriter return = premium + risk-free float yield. Eligibility is enforced at the factory's asset-policy gate via an **eligible-reserve-asset list** per instrument (distinct from the frozen type/instrument *registry* surface), governed by four hard rules:
1. **Reference exclusion** — reserve assets may not be the reference asset, nor issued/guaranteed by the reference issuer or its affiliates.
2. **No self-reference** — no Parallar receipts (pBOND) and no Parallar-protected assets as reserves; recursive cover is prohibited by construction.
3. **No rehypothecation** — the vault holds sole title; reserve assets cannot simultaneously collateralize any other obligation.
4. **Trigger-correlation gate** — assets correlated with the instrument's trigger are excluded (e.g., USDC reserves backing USDC-depeg cover).
Plus: liquidity haircuts for NAV-floating reserves (`total_cover ≤ (1−h) × reserve_value`, h per asset tier), and denomination matching (reserve asset = payout asset, or instantly redeemable into it within the settlement window). MVP holds XLM (h=0 equivalent, no yield); the vault interface is designed so a yield strategy adapter slots in without touching the settlement surfaces.

### 3.3 SettlementContract (generic — identical WASM for every type)

Storage: `image_id`, `instrument_id`, `vault`, `epoch_deadlines`, `settled: Map<u32,bool>`.

```rust
fn settle(env, proof: Bytes, journal: Bytes, allocations: Vec<(Address, i128)>);
```
1. Groth16-verify against `image_id` (set at factory deploy; immutable).
2. Decode generic journal (§3.4).
3. Require: `instrument_id` match; `!settled[epoch]`; `journal.deadline == epoch_deadlines[journal.epoch]` (the journal's scan cutoff must be the instrument's canonical deadline, not a prover-chosen one); `now > epoch_deadlines[journal.epoch]`; `position_root ==` vault's current root; `poseidon(allocations) == allocation_root`.
4. Mark settled; `vault.pay_allocations(epoch, allocations)`; emit.

Note what is *absent*: no admin path, no pause-and-pay, no alternative authorization. Generic by construction — nothing in this contract knows it's settling credit.

### 3.4 Generic Journal (final format, 116 bytes fixed-width)

```
instrument_id    32   (H(type_id ‖ rules_version ‖ config_hash))
epoch             4   (u32 BE — instance #1 maps: coupon index)
deadline          8   (u64 BE)
position_root    32
allocation_root  32
total_payout      8   (u64 BE)
```
This layout is shared by **every** instrument type — it is the contract between any guest and the generic settlement contract, and the most expensive thing to change later. Treat as frozen; any change updates this section in the same commit and bumps rules_version.

## 4. Guest Interface & Instance #1

**Guest contract (the plug-in boundary, final):** a settlement guest receives type-specific private inputs + the instrument's public bindings, and MUST (a) verify the reference commitments in `config_hash`, (b) perform its determination, **panicking if the trigger did not occur**, (c) open position commitments and recompute `position_root`, (d) compute allocations per its published rules version, (e) assert `Σ ≤ collateral`, (f) commit the generic journal. Anything satisfying this runs on the unchanged core.

**`settle_credit_v1` (built):** snapshot verification; owed = balance × rate; payment scan with **qualifying payment** defined per §10.2 (asset-received basis: classic payment ops AND SAC transfer events both count; path payments qualify by asset received; muxed addresses normalized to base account); per-holder shortfall; missed iff Σ shortfall > 0 else panic; pro-rata `payout = cover × (Σ shortfall / Σ owed)` capped at cover (integer math, rounding documented); journal commit. No `current_time` input — the contract enforces `now > deadline`; truncated-history risk is the named input-canonicity gap.

**Default-event rules (published, normative — the asymmetry matters):**
- *Clawback = default.* A coupon payment subsequently clawed back by the issuer within the determination window does not qualify: payments count **net of clawback operations**. An issuer cannot "pay" a coupon and take it back.
- *Freeze = default.* If the issuer revokes a holder's trustline authorization (freeze) such that the coupon cannot be received, that holder's owed amount counts as **shortfall** — issuer-side failure is issuer default.
- *Missing/closed holder trustline = excluded.* If a holder has no valid trustline for the coupon asset (removed it, or never authorized), the issuer **cannot** pay them on Stellar; that holder's owed amount is **excluded from Σ owed** for the epoch — holder-side failure is not issuer default. (Production note: per-epoch record-date snapshots, PRODUCTION_GAP G4, make this crisp.)
- Boundary: `paid_at ≤ deadline` counts as paid; deadlines are ledger close timestamps (~5s granularity, §10.4).

Guest tests (P0-blocking): full miss / partial / short-pay / fully-paid-panics / boundary / **clawed-back-payment-counts-as-shortfall** / **frozen-holder-counts-as-shortfall** / **no-trustline-holder-excluded-from-owed** / commitment-mismatch panics / cap respected / wrong-instrument binding panics.

**`settle_weather_v1` (instance #2 — specified, not built):** determination = index computation over an attested observation window vs strike; same commitments, same allocation shape, same journal. Documented in README as proof the layer generalizes; implementing it post-hackathon is the whether.market convergence.

**`trade_settlement_v1` (instance #3 — sketched, not built):** commodity trade settlement — the provisional-to-final invoice computation for a physical cargo. Determination = final price per the contract's pricing annex (quotational-period averaging against licensed indices, assay-based quality adjustments, laytime/demurrage computation from port timestamps); the buyer's funds are the funded reserve; the certificate authorizes the final payment split (seller balance, buyer refund of provisional overpayment, demurrage either way). Same journal, same vault mechanics, same verifier. Two structural notes: (a) inputs are off-chain documents, so G1-style attestation (signed eBLs per DCSA, SGS/BV inspection certificates verified in-guest, licensed index feeds) is the centerpiece, not residual; (b) plain condition-release escrow is explicitly out of scope — it fails the load-bearing test (one fact, checkable on-chain); the product is the disputed *computation*, which is where the zkVM earns its place.

## 5. P1 Guests

`prove_exposure` (eligibility without balance reveal) and, if solvency Option C: `prove_solvency` (purchase-time aggregate check). Both small; both after P0 freeze (exposure) / by June 17 decision (solvency).

## 5A. Yield Router — the protected share class (sketched, not built; post-hackathon)

The distribution layer: nobody buys "insurance," they buy *the protected version of the bond's yield*. A `YieldRouter` contract per instrument (optionally deployed by the factory alongside the vault/settlement pair):

```rust
fn wrap(env, holder, bond_amount)    -> mints pBOND receipt; registers routed cover
fn route_coupon(env, epoch)          -> waterfall: gross coupon for wrapped holders →
                                        premium_bps to VaultContract (sellers + protocol fee),
                                        net to pBOND holders
fn unwrap(env, holder, amount)       -> burns receipt, returns bond tokens; cover lapses next epoch
```

Properties:
- **Cover auto-sized and auto-renewed:** wrapped balance defines cover; no separate purchase flow. Example economics: 14% gross coupon, 2%-equivalent premium → holder receives 12% net, protected.
- **Settlement unchanged:** on default the standard certificate pays wrapped holders from the reserve; the router is purely upstream of the core (vault, journal, guests, verifier untouched — the architecture's surfaces hold).
- **Revenue model (layered, MGA-benchmarked):** premium flows primarily to protection sellers (their capital is at risk). Parallar's take is fee-for-function: **(a)** base protocol fee on all premium, ~12% (the rail); **(b)** distribution fee of +10–15% on premium Parallar originates via the protected share class — the MGA-equivalent work earning the MGA-equivalent fee, blended take on distributed flow ≈ 22–27% vs MGA benchmarks of 20–30% of GWP; **(c)** structuring fees (bps on notional) for bespoke instruments; **(d)** ~10% of reserve float yield (§3.2). Marketplace-only flow stays at the 12% base to remain fork-resistant. Under the (P)SPI wrapper Parallar operates formally as program manager to the insurer, making MGA fee benchmarks the literal comp set.
- **Tension 1 — visibility:** the router must know wrapped balances to split coupons, so pBOND cover is effectively public. Resolution: two product lines on one core — the transparent **protected share class** (treasuries, funds, retail) and **sealed bespoke cover** (institutions concealing their book, the existing commitment path). Stated plainly in any router documentation.
- **Tension 2 — premium-in-arrears:** premium deducted from coupons means a defaulted epoch yields no premium exactly when sellers pay out. Resolution: pricing reflects it, or `wrap` escrows one epoch's premium upfront. Decide at build time; never paper over.
- **Regulatory note:** routing coupons and deducting premium strengthens the resemblance to insurance distribution — reinforces the (P)SPI wrapper pathway (PRODUCTION_GAP, roadmap) and should be scoped with counsel before mainnet.

**pBOND as enhanced collateral (composability thesis; production = G13):**
A protected bond has a truncated loss distribution, which is what lending-market risk parameters price. pBOND should therefore command **higher LTV and gentler liquidation parameters** than the naked bond in external lending markets (Blend on Stellar is the natural first venue). Illustrative loop economics at 6% borrow: naked bond 14% @ 60% LTV → ~26% levered; pBOND 12% net @ 80% LTV → ~36% levered — the premium pays for itself in capital efficiency, with a fatter safety margin. Flywheel: looping multiplies protected notional on the same capital, multiplying premium throughput (a 3x loop ≈ 3x premium flow).
Honest constraints, stated in order of importance:
1. **Coupon ≠ principal.** `credit_v1` protects coupons; the full LTV-uplift case requires principal protection — a `credit_principal_v1` guest (failure-to-redeem at maturity) on the unchanged core. Coupon cover alone justifies partial uplift (cash-flow certainty + the verified missed-coupon certificate as a superior early liquidation trigger), and that distinction must be presented plainly to lenders.
2. **Covenant boundary.** pBOND in external lending markets = user leverage, permitted. pBOND inside Parallar reserves = prohibited (non-circularity rule 2). The loop must never route back into the collateral backing the protection.
3. **What lenders can verify:** fully-funded-by-construction means reserve adequacy is provable on-chain — credit enhancement a risk team can price, unlike a bilateral CDS counterparty promise. Unwrap/redemption mechanics (incl. mid-epoch premium accrual) must be liquidation-compatible.
4. Loop leverage is user risk; recommended LTV bands and unwind behavior under stress are documentation obligations, not protocol guarantees.

## 6. Frontend (P1, 1-day cap)

Registry view (types + live instruments — the "layer" beat) · instrument page (epochs paid/short/missed; vault aggregates, explicitly no cover amounts) · settlement panel with stellar.expert links. CLI drives the demo.

## 7. Demo & Testing

`scripts/demo.sh` (the submission's spine):
1. `register_type(credit_v1)` → **`deploy_instrument` — one tx, instrument live** (the factory beat; show it twice: deploy a second instrument to prove replication).
2. Sellers deposit; buyers buy committed cover.
3. Epoch 0 fully paid → **no proof can exist** (guest panic shown).
4. Epoch 1: issuer pays 7 of 10 → keeper proves → on-chain verify → confidential payouts.
5. Forged proof / replay / stale-root → reverts.
6. Benchmarks printed (proof time, verify fee; N=10 measured, 1k extrapolation table).

Video (2–3 min): 0:00–0:30 the layer story ("RWAs on Stellar can't be hedged; determination is heavy, institutions need confidentiality, settlement needs no trusted operator — Parallar is the settlement layer; here's a credit instrument deployed in one transaction") → 0:30–2:00 live flow incl. the panic-on-honest-data beat → 2:00–2:30 trust-model honesty + instance #2 + roadmap. First 30 seconds carry the placement.

## 8. Risks

| Risk | L | Mitigation |
|------|---|------------|
| Deployer-pattern atomicity (deploy+init in one tx) trickier than docs suggest | M | Sprint 1 spike day; pre-agreed fallback = registry-real/deploy-by-script |
| Poseidon guest↔host mismatch | M-H | Sprint 0 blocking spike; SHA-256 fallback |
| Groth16 wrap environment | M-H | Sprint 0 decision, 4h box, remote-proving fallback |
| Guest size/time (scan + commitments + allocations) | M | N=10; benchmark at Sprint 2 start; cache known-good proof for video day |
| Solvency Option C overrun | M | C→B→A degradation, June 17 checkpoint |
| Generalization gold-plating (building abstractions instance #2 doesn't need yet) | M-H | Rule: the core is exactly as generic as the journal + guest contract require — no trait towers, no config DSLs. CLAUDE.md enforces |
| Scope creep (tranches, feeds, escape hatch) | H | PRD non-goals binding; refuse-and-flag |

## 9. Repo Layout

```
parallar/
├── CLAUDE.md
├── README.md
├── docs/                    # PRD, TECH_SPEC, SPRINT_PLAN, PRODUCTION_GAP, STATUS
├── contracts/
│   ├── factory/             # registry + deployer
│   ├── bond/                # instance #1 reference (mock issuance)
│   ├── vault/               # generic
│   └── settlement/          # generic
├── prover/
│   ├── guests/
│   │   ├── settle_credit_v1/
│   │   └── prove_exposure/  # P1
│   └── host/                # prove / submit / history_builder
├── scripts/                 # deploy.sh, demo.sh, reset.sh
└── frontend/                # P1
```

## 10. Stellar Asset Policy & Network Idiosyncrasies (normative)

These are the Stellar-specific mechanics that determine whether Parallar is built correctly from day one. Each item states the fact, the design rule, and where it's enforced. This section is the canonical reference; rules here override intuition imported from other chains.

### 10.1 Clawback & authorization flags (CAP-35 / Protocol 17)
**Fact:** transactions are irreversible, but issuers of assets with `AUTH_CLAWBACK_ENABLED` can pull balances back from holders; issuers with revocable auth can freeze trustlines. These exist *for* regulated RWAs — the bonds Parallar protects will plausibly use them.
**Rules:** (a) collateral asset must be claw-proof and freeze-proof — factory enforces at deploy (§3.0 step 5); (b) coupon payments count net of clawback (§4); (c) freeze of a holder = issuer-side shortfall; missing holder trustline = excluded from owed (§4). **Enforced:** factory check + guest determination rules + tests.

### 10.2 Two payment representations (classic ops vs SAC events)
**Fact:** the same economic payment can occur as a classic `Payment` operation or as a Stellar Asset Contract `transfer` — different ledger artifacts. Path payments deliver the asset via a different op type again.
**Rule:** qualifying payment is defined on an **asset-received basis**: any mechanism delivering ≥ the owed amount of the coupon asset to the holder by the deadline qualifies. `history_builder` scans both representations; the guest is representation-agnostic (it sees normalized PaymentRecords). **Enforced:** history_builder + published rules + a mixed-representation guest test.

### 10.3 Muxed accounts
**Fact:** payments may target muxed addresses (M...) that resolve to a base account (G...).
**Rule:** holder matching normalizes to the base account. **Enforced:** history_builder normalization + test.

### 10.4 Finality & time semantics (SCP)
**Fact:** SCP gives deterministic finality (~5s ledgers); no reorgs, ever — under stress the network halts rather than forks. `env.ledger().timestamp()` is ledger close time.
**Rules:** determination windows are crisp — no confirmation-depth logic needed (a genuine advantage; say so in the demo). All deadlines are ledger timestamps; a network halt spanning a deadline delays settlement but never corrupts it (history is still scanned to the true deadline). **Enforced:** by construction; documented in README.

### 10.5 State archival / TTL (rent)
**Fact:** Soroban ledger entries carry a TTL. Persistent entries whose TTL expires are archived (restorable, auto-restored since Protocol 23 at extra fee — and note the Oct 2025 P23 archival incident as a reason for vigilance); **temporary entries are deleted permanently**.
**Rules:** position commitments, position_root, registry state, settled flags → **archival-class storage (`persistent` or `instance`), never `temporary`**; extend TTLs on access; instruments carry rent implications for their lifetime (a 10-year bond's vault must not quietly archive). MVP: the registry uses `persistent`, and per-instrument vault/settlement bindings use `instance` storage — both archival-class (auto-restorable, never deleted) with extend-on-access. Production: tier the long-lived bindings into dedicated `persistent` entries with per-entry rent budgeting, plus TTL monitoring + scheduled extension as an operational requirement (PRODUCTION_GAP G9). **Enforced:** storage-type review in the code-review checklist (the bar is archival-class, never `temporary`); CLAUDE.md invariant.

### 10.6 RPC event retention (~7 days)
**Fact:** Stellar RPC retains events for roughly the last 7 days; older history requires an archive RPC / Horizon / history archives.
**Rule:** `history_builder` must not assume default-RPC completeness. MVP demo windows are fresh (fine); the design treats data-source selection as pluggable so archive-RPC support is a swap, not a rewrite. Settlement of an old epoch must remain possible forever. **Enforced:** history_builder abstraction + PRODUCTION_GAP G1 (the attestation layer subsumes this).

### 10.7 Soroban per-transaction resource limits
**Fact:** transactions have CPU/memory/footprint budgets; verification cost plus a large allocation list could exceed them.
**Rule:** measure verify-fee + max allocation-list size early (Sprint 2 benchmark); MVP N is small; production design allows **chunked payout execution** (one verified proof authorizing allocation batches via the committed allocation_root) without journal changes. **Enforced:** benchmark task + PRODUCTION_GAP G7.

### 10.8 Trustline limits
**Fact:** a holder's trustline has a limit; a coupon payment exceeding remaining capacity fails.
**Rule:** treated identically to missing trustline — holder-side failure, excluded from owed for that portion. Documented; demo doesn't exercise it. **Enforced:** published rules note.
