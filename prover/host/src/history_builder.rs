//! `history_builder` — assembles the guest witness's `snapshot` + `payments` from observed
//! chain data, implementing the normative qualifying-payment rules (TECH_SPEC §10).
//!
//! What it guarantees and what it does NOT: this turns *observed* transfers into the guest's
//! representation-agnostic `Payment` records and the holder `snapshot`. It does not attest that
//! the observation is canonical — truncated or withheld history is the named input-canonicity
//! gap (README trust model; PRODUCTION_GAP **G1**, the attestation layer, subsumes it). The
//! proof guarantees correct computation over *these* inputs, not that these inputs are complete.
//!
//! Spec rules implemented here:
//! - §10.2 asset-received basis: a qualifying payment is any delivery of the coupon asset to a
//!   holder, regardless of representation (classic `Payment` op, SAC `transfer` event, or a
//!   path payment delivering the asset). The data source flattens all three into [`RawTransfer`]
//!   rows; this module is representation-agnostic and simply keeps the coupon-asset rows.
//! - §10.3 muxed accounts: holder matching normalizes M-addresses to their base G-account key.
//! - clawback (§10.1/§4): a clawed-back delivery is carried through as `clawed_back = true`; the
//!   guest then excludes it from `paid`, so it counts as shortfall.
//! - §10.6 retention: the data source is a trait — RPC is the default, an archive source is a
//!   swap, not a rewrite. Settlement of an old epoch must remain possible forever.
//!
//! `paid_at` is a ledger **close** timestamp (§10.4); the guest compares it to the committed
//! deadline. This module does not filter by deadline — the guest does (binding it to the
//! committed `deadline`), so the witness can carry the full scanned window.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use settle_credit_v1::{Holder, Inputs, Payment};

/// A Stellar asset identity, for selecting the coupon asset out of observed transfers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asset {
    Native,
    Credit { code: String, issuer: String },
}

/// One observed delivery of an asset to an account, already flattened from whichever ledger
/// artifact produced it (classic op / SAC event / path payment) by the data source. The `to`
/// address is as observed (G… or M…); `amount` is the asset *received*; `paid_at` is the
/// ledger close timestamp; `clawed_back` is set if a later clawback reversed this delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawTransfer {
    pub asset: Asset,
    pub to: String,
    pub amount: i128,
    pub paid_at: u64,
    #[serde(default)]
    pub clawed_back: bool,
}

/// A bond holder observed at the snapshot ledger. `account` is the base G-address; the flags
/// follow §10.1 (no trustline → excluded from owed; frozen trustline → issuer-side shortfall).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderRecord {
    pub account: String,
    pub balance: i128,
    pub has_trustline: bool,
    #[serde(default)]
    pub frozen: bool,
}

/// The output of a data source for one instrument/epoch window: the holder snapshot and every
/// observed transfer (all representations, all assets — this module selects the coupon asset).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scan {
    pub coupon_asset: Asset,
    pub holders: Vec<HolderRecord>,
    pub transfers: Vec<RawTransfer>,
}

/// Pluggable observation layer (§10.6). The default is RPC/Horizon; an archive source for old
/// epochs is a swap behind this trait, not a rewrite. A `Scan` loaded from a file (e.g. an
/// archive export) is itself a valid source — see [`FileSource`].
pub trait DataSource {
    fn scan(&self) -> Result<Scan>;
}

/// A data source backed by a JSON `Scan` file — the archive/export path, and what the demo and
/// tests use. (The live RPC/Horizon source is a separate impl that produces the same `Scan`.)
pub struct FileSource {
    pub path: std::path::PathBuf,
}

impl DataSource for FileSource {
    fn scan(&self) -> Result<Scan> {
        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading scan {}", self.path.display()))?;
        serde_json::from_str(&raw).context("parsing scan JSON")
    }
}

/// Normalize any Stellar account address to its base account's 32-byte ed25519 key. A muxed
/// (M…) address resolves to the underlying G-account (§10.3); a G… address is itself.
pub fn base_account_key(addr: &str) -> Result<[u8; 32]> {
    use stellar_strkey::{ed25519, Strkey};
    match Strkey::from_string(addr).map_err(|e| anyhow::anyhow!("bad address {addr}: {e}"))? {
        Strkey::PublicKeyEd25519(ed25519::PublicKey(k)) => Ok(k),
        Strkey::MuxedAccountEd25519(ed25519::MuxedAccount { ed25519: k, .. }) => Ok(k),
        other => bail!("address {addr} is not an account (got {other:?})"),
    }
}

/// Turn a scan into the guest's `snapshot` + `payments`, applying §10.2 (coupon-asset, all
/// representations) and §10.3 (muxed → base). Order is preserved; the guest sums per-holder.
pub fn build_snapshot_and_payments(scan: &Scan) -> Result<(Vec<Holder>, Vec<Payment>)> {
    let snapshot = scan
        .holders
        .iter()
        .map(|h| {
            Ok(Holder {
                id: base_account_key(&h.account)?,
                balance: h.balance,
                has_trustline: h.has_trustline,
                frozen: h.frozen,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let payments = scan
        .transfers
        .iter()
        .filter(|t| t.asset == scan.coupon_asset) // asset-received basis: only the coupon asset
        .map(|t| {
            Ok(Payment {
                holder: base_account_key(&t.to)?, // muxed normalized to the base account
                amount: t.amount,
                paid_at: t.paid_at,
                clawed_back: t.clawed_back,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((snapshot, payments))
}

/// Fill a witness template's `snapshot` + `payments` from a scan, leaving the operator-supplied
/// fields (config, terms, positions, epoch, deadline, collateral, instrument binding) intact.
/// The scan supplies only what is observed on-chain; everything else is the instrument's
/// committed config + the keeper's local position openings (TECH_SPEC §3.2).
pub fn fill_witness(mut params: Inputs, scan: &Scan) -> Result<Inputs> {
    let (snapshot, payments) = build_snapshot_and_payments(scan)?;
    params.snapshot = snapshot;
    params.payments = payments;
    Ok(params)
}

#[cfg(test)]
mod test {
    use super::*;
    use stellar_strkey::{ed25519, Strkey};

    fn g_addr(key: [u8; 32]) -> String {
        Strkey::PublicKeyEd25519(ed25519::PublicKey(key)).to_string()
    }
    fn m_addr(base: [u8; 32], id: u64) -> String {
        Strkey::MuxedAccountEd25519(ed25519::MuxedAccount { ed25519: base, id }).to_string()
    }

    #[test]
    fn g_address_is_its_own_base_key() {
        let k = [7u8; 32];
        assert_eq!(base_account_key(&g_addr(k)).unwrap(), k);
    }

    #[test]
    fn muxed_normalizes_to_base_account() {
        let base = [9u8; 32];
        // an M-address (with a routing id) resolves to the same base key as the G-address
        assert_eq!(base_account_key(&m_addr(base, 42)).unwrap(), base);
        assert_eq!(base_account_key(&m_addr(base, 42)).unwrap(), base_account_key(&g_addr(base)).unwrap());
    }

    #[test]
    fn rejects_non_account_address() {
        // a contract (C…) is not a holder account
        let c = Strkey::Contract(stellar_strkey::Contract([3u8; 32])).to_string();
        assert!(base_account_key(&c).is_err());
    }

    fn coupon() -> Asset {
        Asset::Credit { code: "BOND".into(), issuer: g_addr([1u8; 32]) }
    }

    #[test]
    fn payments_are_asset_received_muxed_normalized_clawback_preserved() {
        let holder = [5u8; 32];
        let scan = Scan {
            coupon_asset: coupon(),
            holders: vec![HolderRecord {
                account: g_addr(holder),
                balance: 10_000,
                has_trustline: true,
                frozen: false,
            }],
            transfers: vec![
                // classic op to the G-address
                RawTransfer { asset: coupon(), to: g_addr(holder), amount: 300, paid_at: 100, clawed_back: false },
                // SAC transfer to a MUXED address of the SAME holder — must normalize to base
                RawTransfer { asset: coupon(), to: m_addr(holder, 7), amount: 200, paid_at: 110, clawed_back: false },
                // a delivery that was later clawed back — carried through, not dropped
                RawTransfer { asset: coupon(), to: g_addr(holder), amount: 500, paid_at: 120, clawed_back: true },
                // a DIFFERENT asset — excluded by the asset-received filter
                RawTransfer { asset: Asset::Native, to: g_addr(holder), amount: 999, paid_at: 130, clawed_back: false },
            ],
        };

        let (snapshot, payments) = build_snapshot_and_payments(&scan).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, holder);

        // three coupon-asset payments survive (the native one is filtered out)
        assert_eq!(payments.len(), 3);
        // all normalized to the same base holder key (incl. the muxed one)
        assert!(payments.iter().all(|p| p.holder == holder));
        assert_eq!(payments[1].amount, 200, "muxed delivery counted at its base account");
        assert!(payments[2].clawed_back, "clawback flag preserved");
        // mixed representations (classic op + SAC transfer) both counted — §10.2
        assert_eq!(payments[0].amount + payments[1].amount, 500);
    }
}
