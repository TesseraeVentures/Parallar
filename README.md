# Parallar

**A verifiable settlement layer on Stellar. Credit default instruments are product #1.**

> Determination off-chain. Positions confidential. Settlement proven. Deploying a new protected instrument is one transaction.

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

**Published settlement rule (credit_v1):** `payout = cover × (Σ shortfall / Σ owed)`, capped at cover. The proof enforces exactly this; the contract accepts no other allocation set. **Stellar-native default rules:** payments count net of clawback (an issuer cannot pay a coupon and take it back); freezing a holder's trustline counts as issuer default; a holder who removed their own trustline is excluded from owed (the issuer *cannot* pay them — holder-side failure is not issuer default). The factory refuses clawbackable or freezable assets as vault collateral, so the payout pool itself can never be pulled. Full normative list: TECH_SPEC §10.

## Trust model — stated plainly

Each proof guarantees the settlement **computation** — the version-pinned rules executed over the supplied input data and the committed positions — is correct (verifiable execution). It does not guarantee the input data is canonical: correct computation on inputs, not fair inputs. Mitigations today: permissionless keeping (anyone can settle) and independent on-chain deadline enforcement. The real fix, attested data feeds, is gap **G1** in [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md), which enumerates the complete path from this demo to a live issuance pilot. Within the proof boundary, no party — including us — can inflate, favor, omit, or fabricate a payout.

## What's real / what's mocked

- ✅ Real: factory deployment via the Soroban deployer pattern; bond as a Stellar asset with 10 holders and genuine (including partial) coupon transfers; Poseidon-committed positions; RISC Zero settlement proofs; on-chain Groth16 verification (BN254 + Poseidon host functions, Protocols 25–26); payouts authorized solely by proof.
- ⚠️ Mocked/simplified: the issuance is a mock modeled on a live Stellar corporate bond (unnamed); holder snapshot fixed at issuance; flat premium; demo keeper reads buyers' commitment openings from a local file (production: buyer-held openings + self-claim escape hatch — G2).

## Quick start

```bash
# prerequisites: rust, stellar-cli, rzup — docs/TECH_SPEC.md §2
./scripts/demo.sh     # self-contained one-command path: registers credit_v1, factory-deploys two instruments, runs the full scenario — negative paths, benchmarks
./scripts/deploy.sh   # deploy-only subset: register + a single factory deploy to testnet (for manual poking)
```

The demo shows: a type registered and **two instruments factory-deployed** → a fully-paid epoch for which no proof can exist → a partial default (7 of 10 holders paid) detected by the scan → proof → verification → confidential payouts → forged-proof, replay, and stale-root attempts reverting → proof-time and fee benchmarks (N=10 measured, 1k+ extrapolated).

## Roadmap

[PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md) in full; headlines: attested history feeds (G1) → buyer-held openings + escape-hatch self-claims (G2) → audit (G5) → testnet pilot on a mirrored real issuance → the **yield router** (wrap a bond, receive the protected share class: gross coupon in, premium to the reserve, net yield out — e.g. 14% → 12% protected) → further instrument types on the unchanged core: `weather_v1` (parametric index settlement) and `trade_settlement_v1` (commodity provisional-to-final invoicing — pricing, quality, demurrage — with buyer funds as the funded reserve) → multi-tranche vaults → regulated wrapper via Bermuda's BMA (P)SPI framework.

## Repo map

`contracts/` (factory, bond, vault, settlement) · `prover/guests/` (settle_credit_v1, prove_exposure) · `prover/host/` · `scripts/` · `docs/` (PRD, TECH_SPEC, SPRINT_PLAN, PRODUCTION_GAP) · `frontend/`

MIT
