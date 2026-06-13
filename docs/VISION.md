# Parallar — Vision: Risk-Stripped Capital Markets

**Status:** North star. Nothing in this document is build scope; everything in it is reachable through the existing factory and the frozen settlement surfaces. The hackathon ships `credit_v1`; this document explains why that wedge matters.

---

## The end state

Every risk inside a security, separated into its own token, each leg protectable, the legs recombinable into assets shaped to institutional mandates.

```
TOKENIZED BOND  (principal + coupons + credit + duration, bundled)
      │
      ├── LAYER 1 · SPLIT  (Pendle mechanics — adjacent protocol, not Parallar)
      │     PT  = principal at maturity (isolated duration + credit)
      │     YT  = coupon stream to maturity
      │
      ├── LAYER 2 · PROTECT  (Parallar — two factory instruments per bond per maturity)
      │     protected-PT  ← credit_principal_v1   (failure-to-redeem cover)
      │     protected-YT  ← credit_v1             (missed/short coupon cover)
      │     each leg also available naked; sealed bespoke cover for institutions
      │
      └── LAYER 3 · RECOMPOSE  (capital-markets layer)
            protected-PT                  = credit-enhanced zero-coupon bond
            ladder of protected-PTs       = fixed annuity (LDI-shaped)
            YT strips                     = income products
            combinations                  = structured notes
```

## Why institutions care

Investment mandates confine large pools of capital to principal-protected, investment-grade, fixed-income shapes. The historical machine for converting paper into mandate-eligible paper was the **monoline wrap** (MBIA, Ambac): insurance that lifted sub-grade issuance to AAA — until 2008 revealed the wrap was a promise written against a thin balance sheet.

**Parallar is the monoline rebuilt without the monoline balance sheet:** the wrap is fully funded by construction, the reserve is verifiable on-chain in real time, and every payout is proven correct before it moves. That is the difference between credit enhancement an allocator must trust and credit enhancement an allocator can audit from their desk.

## Honest constraints (in priority order)

1. **Mandate eligibility is legal, not merely structural.** Mandates key off ratings and legal opinions. Structure helps; the unlock runs through the BMA (P)SPI wrapper and, at scale, a rated reinsurance partner standing behind or alongside the reserve. This is a multi-year regulatory build, sequenced in PRODUCTION_GAP, not a smart-contract feature.
2. **Duration needs no token yet.** A PT *is* isolated duration + credit; an explicit duration token is an interest-rate-swap instrument requiring an on-chain benchmark rate market that does not meaningfully exist. Duration is expressed through the PT maturity curve until it does.
3. **Liquidity fragmentation is the known tax of the Pendle model.** Every maturity is its own market. Mitigants: concentrate on few maturities per issuance early; the yield router (G11) as default distribution; market-making partnerships at the capital-markets layer.
4. **The splitter is not Parallar.** Splitting/AMM mechanics live in an adjacent protocol layer (the Tesserae lineage). Parallar's contribution is protection + verifiable settlement per leg, delivered through the unchanged factory. The four frozen surfaces (contract separation, generic journal, registry interface, guest plug-in boundary) are what make this vision additive rather than a rebuild.
5. **Recomposed products tighten the regulatory perimeter.** Annuity-equivalents and structured notes are product manufacturing in most jurisdictions. Counsel review precedes each recomposition product class; the (P)SPI is the chassis.

## What this does to the business model

Each split doubles the instrument surface per bond (PT-cover + YT-cover), each maturity multiplies it again, and lending-market loops (G13) multiply protected notional on top. Premium throughput — the revenue base — compounds along three independent axes: assets tokenized × legs per asset × leverage per leg. The factory was built for exactly this shape of growth: new instruments are registrations, not engineering.

## Sequencing (indicative)

| Horizon | Milestone | Depends on |
|---|---|---|
| Now | `credit_v1` shipped at hackathon | — |
| Seed (18mo) | Audited mainnet · first live issuance · yield router · `credit_principal_v1` | G1–G7, G11 |
| Series A | Lending-market listings (G13) · weather + trade types · PT/YT partnership or build at the adjacent layer | seed milestones |
| Growth | Recomposed products (ZCB, annuity ladders) under (P)SPI + rated partner · mandate-eligibility workstream | regulatory maturity |

## Lineage note

The four-token architecture (principal / yield / protected-principal / protected-yield) predates Parallar in the founder's Tesserae design work. Parallar supplies what that design lacked: a settlement layer that makes the protected legs *provable*. The vision is old; the missing piece is what's being built now.
