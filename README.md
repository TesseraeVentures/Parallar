# Parallar

**A verifiable settlement layer on Stellar. Credit default instruments are product #1.**

> Determination off-chain. Positions confidential. Settlement proven. Deploying a new protected instrument is one transaction.

The first protection protocol we know of on Stellar — and the only one anywhere that settles default cover **by proof, over confidential positions.** *(Category survey, June 2026; re-verified quarterly.)*

*Submission to Stellar Hacks: Real-World ZK, June 2026.*

---

## The problem

Tokenized RWAs are arriving on Stellar, but the instruments built on top of them — protection, hedges, structured payouts — all hit the same three walls:

1. **Determination is heavy.** "Did the trigger occur?" (a coupon missed across 1,000 bondholders; an index breach over a data window) is computation no smart contract can run.
2. **Institutions won't trade on a transparent book.** Public position sizes leak credit exposure and strategy.
3. **Settlement requires trusting someone** — a committee, an operator, an admin key — to compute the payouts.

## What Parallar is

A factory of instruments sharing one provable settlement core:

- **Registry/Factory** — instrument *types* are registered: a settlement guest's image ID + version-pinned published rules + contract WASM hashes. Instrument *instances* (vault + settlement pair, cross-bound) deploy in **one transaction** via the Soroban deployer pattern. New guest version = new type; live instruments are pinned to their settlement logic forever — immutability institutions can underwrite against.
- **Generic core** — a vault holding seller collateral (public) and buyer positions as **Poseidon commitments only** (cover sizes never touch public state), and a settlement contract whose sole authorization path is verifying a Groth16 proof against the type's pinned image ID. No admin path exists.
- **Pluggable guests** — per-type RISC Zero programs. **Instance #1 (this repo):** parametric credit protection. The guest scans real payment history across all bondholders, establishes the coupon was missed or short-paid, opens every position commitment privately, computes payouts pro-rata to the default's severity, and proves the entire settlement. **Instance #2 (specified):** parametric weather/index protection — same core, different guest.

```
factory.deploy_instrument(credit_v1, config)          ← one tx, instrument live
        │
payment history ─┐
                 ├─► RISC Zero guest ─► Groth16 proof ─► settlement verifies ─► payouts
hidden positions ┘   (determine+settle)  (constant size)  (sole auth path)
```

**Why ZK is structural, not decorative:** the chain *cannot* compute these payouts — the determination is too heavy and the positions are hidden by construction. Remove the proof and no instrument in the family can exist. If the trigger did not occur, the guest panics on honest data: **no valid settlement proof can exist for a false claim.**

> A claims committee can be *convinced* to pay a default that never happened. This guest cannot. Feed it a fully-paid epoch and it panics on honest data — no proof, no payout, by construction. Discretion is the thing Parallar removes.

**Published settlement rule (credit_v1):** `payout = cover × (Σ shortfall / Σ owed)`, capped at cover. The proof enforces exactly this; the contract accepts no other allocation set. **Stellar-native default rules:** payments count net of clawback (an issuer cannot pay a coupon and take it back); freezing a holder's trustline counts as issuer default; a holder who removed their own trustline is excluded from owed (the issuer *cannot* pay them — holder-side failure is not issuer default). The factory only accepts collateral from a curated eligible-asset list governed by the registry multisig (a Soroban contract can't introspect a classic asset's issuer flags on-chain, so eligibility is verified off-chain at listing and monitored for changes — PRODUCTION_GAP G6 / [docs/OPERATIONS.md](docs/OPERATIONS.md)); native XLM, used here, is structurally claw-proof and freeze-proof — so the payout pool itself can never be pulled. Full normative list: TECH_SPEC §10.

## Trust model — stated plainly

Each proof guarantees the settlement **computation** — the version-pinned rules executed over the supplied input data and the committed positions — is correct (verifiable execution). It does not guarantee the input data is canonical: correct computation on inputs, not fair inputs. Mitigations today: permissionless keeping (anyone can settle) and independent on-chain deadline enforcement. The real fix, attested data feeds, is gap **G1** in [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md), which enumerates the complete path from this demo to a live issuance pilot. Within the proof boundary, no party — including us — can inflate, favor, omit, or fabricate a payout.

**A first cut of G1 is already built.** `credit_v2` is a registered type (image_id `d07e6aaf…`) that verifies an issuer **Ed25519 signature over the payment snapshot in-guest** — so the proof itself certifies the data was signed by the committed issuer key (the key is bound in `terms_hash`). "Trust the keeper's data" becomes "trust the issuer's signature": a keeper can no longer fabricate, omit, or alter payments and still produce a proof. `credit_v1` stays pinned forever (the versioning law) — the hardening ships as a **new type, not an edit**, which is exactly how production upgrades are meant to work here.

## How it compares

Protection over credit is not new. What is new is *who is allowed to decide a payout* — and Parallar is the only one that answers "a proof, and no one else."

| | Who authorizes a payout | Positions | Funding | Portable |
|---|---|---|---|---|
| **Parallar** | a verified proof — no party can override, **including us** | confidential (commitments only) | fully funded by construction (cover ≤ reserve) | yes — an on-chain asset |
| DeFi cover mutual | a discretionary member / assessor vote | public | shared pool, discretionary | limited |
| TradFi CDS desk | a committee + the counterparty's promise to pay | private, off-chain | counterparty credit | OTC, bilateral |
| Issuer-embedded tranche | the issuer's capital structure | n/a | structural subordination | non-portable |

Every alternative settles by *someone's discretion*; Parallar settles by *proof*. A committee can be convinced to pay a claim that should fail — the guest cannot, because a false claim is unprovable.

## What's real / what's mocked

| Real | Mocked / simplified (→ production gap) |
|---|---|
| Factory deployment via the Soroban deployer pattern (vault+settlement cross-bound in one tx) | Issuance is a mock modeled on a live Stellar corporate bond (unnamed) |
| Bond as a Stellar asset — 10 holders, genuine (incl. partial) coupon transfers | Holder snapshot fixed at issuance |
| Poseidon-committed positions — cover sizes never touch public state | Flat premium (no pricing curve) |
| RISC Zero settlement proofs — real Groth16 generation (STARK→SNARK, Rosetta x86) | Demo keeper reads buyers' commitment openings from a local file (→ buyer-held openings + self-claim escape hatch, **G2**) |
| On-chain Groth16 verification is the settlement's **sole** payout path — a cross-contract call to the RISC Zero verifier router (Stellar's native BN254 pairing host fn; the journal is committed as its SHA-256 digest, Protocols 25–26); **no admin path exists** | Qualifying-payment history is supplied to the guest (not yet attested → **G1**) |

The proofs guarantee correct *computation over supplied inputs*, not input canonicity — see the trust model above and [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md).

## Quick start

```bash
# prerequisites: rust, stellar-cli — docs/TECH_SPEC.md §2
make demo          # fresh-clone, one command: the full verified scenario (below)
make test          # build all four contracts to wasm + run the full suite
make verify        # independently check the live settlement + determination — see VERIFY.md
```

Don't trust us — **[verify it yourself](VERIFY.md)**: decode the on-chain journal, confirm the proof through the real verifier, reproduce the pinned image_id, and re-run the determination, all from public data.

**`make demo`** (or `./demo.sh`) walks the whole scenario against real code — register `credit_v1` → **two instruments factory-deployed** → deposits + Poseidon-committed cover → a fully-paid epoch for which **no settlement proof can exist** → a partial default → a **real Groth16 proof verified on-chain** by the actual RISC Zero verifier → confidential payouts through the vault → forged-proof / replay / stale-root / tampered-allocation / pre-deadline attempts **reverting** → benchmarks. It's fresh-clone runnable in seconds: it uses the committed real-proof fixture (`prover/host/tests/fixtures/`), so **no testnet keys and no live proof generation are needed**.

To generate a proof live and submit it to testnet, use the host CLI:

```bash
# generate a real Groth16 proof (needs Docker + x86, or Rosetta-x86 emulation):
cargo run -p parallar-prover-host --bin parallar-prover -- prove  --inputs witness.json --out proof.json
# submit it to a deployed instrument:
cargo run -p parallar-prover-host --bin parallar-prover -- submit --artifact proof.json --settlement <C…>
```

## Live on testnet

The full stack is deployed on Stellar testnet, and a real settlement has executed on-chain — a Groth16 proof **verified by the deployed verifier**, paying a buyer from hidden positions:

- **factory** [`CA5WGQMQ…`](https://stellar.expert/explorer/testnet/contract/CA5WGQMQ4DLCB5TTKRCFXBB2VKUALVZ2WB4GJVI6V3DQ2CQASLCH2ATD) · **verifier** [`CCGEOLVI…`](https://stellar.expert/explorer/testnet/contract/CCGEOLVIWOXEK6JAW3SJRMXJFYUO3DOHQCLUZQOGBQTSLTIJ2IXSEDBM)
- **settlement tx** [`8b19cc71…`](https://stellar.expert/explorer/testnet/tx/8b19cc711c8d5e5242acfdbe33d8bdcbc47648a659264feb8ab104c7bc401f65) — events `transfer(vault→buyer, 800)` + `Settled(epoch 1)`

All ids + the reproducible deploy script are in [deployments/testnet.json](deployments/testnet.json) / [scripts/deploy_testnet.sh](scripts/deploy_testnet.sh). **`make frontend`** serves the site (`frontend/`): a multi-page overview that reads *live* on-chain state straight from testnet RPC, plus an **interactive dApp** (`app.html`) — connect a Freighter wallet to deposit collateral or buy cover against the live contracts. Buying cover commits the position with the guest's exact Poseidon function compiled to wasm (`frontend/commit.wasm`), so the in-browser commitment is byte-identical to what the settlement guest reproduces — private and genuinely settleable. The only writes are `deposit` / `buy_protection`; nothing in the UI can move the reserve (Law #1).

## Benchmarks

| Metric | Measured | Notes |
|---|---|---|
| On-chain Groth16 verify | **≈ 35M CPU insns** (Bn254Pairing 17.5M + G2-subgroup 11.8M + G1Mul 5.8M) | ~3× headroom under Soroban's ~100M/tx budget — hardware-independent (runs on-chain) |

**Determination scales with the book; on-chain settlement does not.** zkVM *cycle counts* are deterministic and hardware-independent, so these are representative measured on any machine (unlike proving wall-clock):

| Determination (zkVM user cycles) | 10 | 100 | 1,000 |
|---|---|---|---|
| `credit_v1` (bondholders) | 2.2M | 3.6M | 17.5M |
| `weather_v1` (observations) | 2.1M | 2.7M | 8.5M |

The off-chain determination grows with the book (≈15k cycles per added bondholder), while **on-chain Groth16 verification stays flat at ~35M insns** — settlement cost is constant in the size of the thing being settled. This is the structural payoff: unbounded private determination off-chain, one constant-size proof on-chain. Reproduce: `cargo test -p parallar-prover-host --test scale -- --ignored --nocapture`.

Proof-generation *wall-clock* (N=10 + a 1k-holder extrapolation, per TECH_SPEC §10.7) is captured on representative x86 proving hardware with `parallar-prover bench --inputs witness.json --n 10`. We don't quote a dev-loop figure: proof generation needs x86, and an Apple-Silicon-under-Rosetta-emulation number isn't representative of real proving hardware.

## Instance #2 (`weather_v1`) — same core, different guest

Parallar's claim is that protection is a *family* of instruments over one provable core, not a single product. The cleanest proof of that is a second instrument that reuses every frozen surface and changes only the one piece that is *meant* to change — the per-type guest.

A parametric weather/index instrument (e.g. a payout when rainfall over a window breaches a published threshold) maps onto the unchanged surfaces:

| Surface | `credit_v1` (built) | `weather_v1` (designed-for) | Changes? |
|---|---|---|---|
| Generic vault WASM | seller collateral + Poseidon-committed buyer positions | identical | **no** |
| Generic settlement WASM | sole auth path = verify one Groth16 proof vs the type's image ID | identical | **no** |
| 116-byte journal | the contract reads roots + allocation commitment, never the determination | identical layout | **no** |
| Registry interface | `register_type(image_id, rules, wasm hashes)` → `deploy_instrument` | identical call | **no** |
| Per-type RISC Zero guest | scans coupon payments, proves shortfall/owed, settles pro-rata | scans attested rainfall, proves the index shortfall, settles per the published parametric rule | **yes — and it is the only new code, now proven** |

A new guest is a **new `image_id`, i.e. a new registered type** — never an in-place upgrade of an existing one. Live instruments stay pinned to the guest they were deployed with forever (the versioning law). So `weather_v1` ships the way `credit_v1` did: register the type, then factory-deploy instances against the *same* vault and settlement WASM already in this repo.

**Status: the guest is BUILT.** `settle_weather_v1` is implemented (a rainfall-shortfall parametric rule: `payout = cover × (trigger − observed) / (trigger − exhaust)`, capped, with `NoBreach` unprovable when rainfall meets the threshold), compiles to its own pinned `image_id` `d31246e6…`, and runs in the RISC Zero zkVM — executor-verified that the circuit commits the same 116-byte journal as the native rule. It is **parity-tested against the generic surfaces**: its Poseidon position commitment, position/allocation roots, and `config_hash`/`instrument_id` derivation are byte-identical to `credit_v1`, so the same factory, vault, and settlement WASM accept it with **zero contract changes**. What remains is the live testnet beat — register the type and prove one settlement on x86 (gap **G8**; attested feeds are **G1**). The layer thesis is no longer asserted; it is exercised by a second working guest. (The demo's replication beat — a second factory-deploy of `credit_v1` — exercises the deploy path; `weather_v1` exercises the *guest* boundary.)

## Roadmap

[PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md) in full; headlines: attested history feeds (G1) → buyer-held openings + escape-hatch self-claims (G2) → audit (G5) → testnet pilot on a mirrored real issuance → the **yield router** (wrap a bond, receive the protected share class: gross coupon in, premium to the reserve, net yield out — e.g. 14% → 12% protected) → further instrument types on the unchanged core: `weather_v1` (parametric index settlement) and `trade_settlement_v1` (commodity provisional-to-final invoicing — pricing, quality, demurrage — with buyer funds as the funded reserve) → multi-tranche vaults → regulated wrapper via Bermuda's BMA (P)SPI framework.

## Repo map

`demo.sh` · `reset.sh` · `Makefile` · `contracts/` (factory, bond, vault, settlement) · `prover/` (`guests/settle_credit_v1`, `host` + `parallar-prover` CLI, `methods`) · `frontend/` (live testnet console) · `deployments/` (testnet ids) · `scripts/` (deploy_testnet.sh) · `spikes/poseidon_parity/` · `docs/` (PRD, TECH_SPEC, SPRINT_PLAN, PRODUCTION_GAP, STATUS) · `site/` (landing page) · `deck/` · `external/` (vendored Nethermind RISC Zero verifier, commit-pinned, gitignored)

MIT
