//! `parallar-prover` — the settlement prover host CLI.
//!
//! * `prove`  — run the RISC Zero settlement guest under the real Groth16 prover and write a
//!              submittable proof artifact (needs Docker/x86; ~34 min via Rosetta — §2).
//! * `submit` — invoke `settlement.settle(proof, journal, allocations)` on a deployed
//!              instrument via `stellar-cli`, from a proof artifact.
//!
//! `history-builder` (assemble the witness `Inputs` by scanning qualifying payments per
//! TECH_SPEC §10) is the next subcommand; until then the witness JSON is produced out-of-band.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parallar_prover_host::{prove_settlement, ProofArtifact};
use settle_credit_v1::Inputs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "parallar-prover", about = "Parallar settlement prover host: prove + submit")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the settlement guest under the real Groth16 prover; write a submittable artifact.
    Prove {
        /// Witness JSON — the `settle_credit_v1::Inputs` (snapshot, payments, positions, config…).
        #[arg(long)]
        inputs: PathBuf,
        /// Where to write the proof artifact JSON.
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
    /// Submit a proof artifact to a deployed settlement contract via stellar-cli.
    Submit {
        /// Proof artifact JSON produced by `prove`.
        #[arg(long)]
        artifact: PathBuf,
        /// Deployed settlement contract id (C…).
        #[arg(long)]
        settlement: String,
        /// Source account / identity for the tx (stellar-cli `--source`).
        #[arg(long, default_value = "deployer")]
        source: String,
        /// Network name/passphrase for stellar-cli (`--network`).
        #[arg(long, default_value = "testnet")]
        network: String,
        /// Print the stellar-cli invocation instead of executing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Assemble a witness (holder snapshot + qualifying payments, TECH_SPEC §10) from a data-
    /// source scan + a params template, ready for `prove`.
    HistoryBuilder {
        /// Scan JSON — the data source's observed holders + transfers (archive export / RPC dump).
        #[arg(long)]
        scan: PathBuf,
        /// Witness params JSON — an `Inputs` template with config/terms/positions/epoch/etc. set;
        /// `snapshot` + `payments` are filled in from the scan.
        #[arg(long)]
        params: PathBuf,
        /// Where to write the completed witness JSON (feed to `prove`).
        #[arg(long, default_value = "witness.json")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Prove { inputs, out } => cmd_prove(inputs, out),
        Cmd::Submit { artifact, settlement, source, network, dry_run } => {
            cmd_submit(artifact, settlement, source, network, dry_run)
        }
        Cmd::HistoryBuilder { scan, params, out } => cmd_history_builder(scan, params, out),
    }
}

fn cmd_history_builder(scan_path: PathBuf, params_path: PathBuf, out: PathBuf) -> Result<()> {
    use parallar_prover_host::history_builder::{fill_witness, DataSource, FileSource};
    let scan = FileSource { path: scan_path.clone() }.scan()?;
    let params: Inputs = serde_json::from_str(
        &std::fs::read_to_string(&params_path)
            .with_context(|| format!("reading params {}", params_path.display()))?,
    )
    .context("parsing params JSON")?;
    let witness = fill_witness(params, &scan)?;
    std::fs::write(&out, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("writing witness {}", out.display()))?;
    eprintln!(
        "✓ witness → {} | holders={} payments={} (from scan {})",
        out.display(),
        witness.snapshot.len(),
        witness.payments.len(),
        scan_path.display(),
    );
    Ok(())
}

fn cmd_prove(inputs_path: PathBuf, out: PathBuf) -> Result<()> {
    let raw = std::fs::read_to_string(&inputs_path)
        .with_context(|| format!("reading witness {}", inputs_path.display()))?;
    let inputs: Inputs = serde_json::from_str(&raw).context("parsing witness JSON")?;

    eprintln!("proving settlement — real Groth16 STARK→SNARK (needs Docker/x86; this is slow)…");
    let artifact = prove_settlement(&inputs)?;

    std::fs::write(&out, serde_json::to_string_pretty(&artifact)?)
        .with_context(|| format!("writing artifact {}", out.display()))?;
    eprintln!(
        "✓ proof → {} | seal={}B image_id={} epoch={} total_payout={} payouts={}",
        out.display(),
        hex::decode(&artifact.seal).map(|s| s.len()).unwrap_or(0),
        artifact.image_id,
        artifact.epoch,
        artifact.total_payout,
        artifact.allocations.len(),
    );
    Ok(())
}

fn cmd_submit(
    artifact_path: PathBuf,
    settlement: String,
    source: String,
    network: String,
    dry_run: bool,
) -> Result<()> {
    let raw = std::fs::read_to_string(&artifact_path)
        .with_context(|| format!("reading artifact {}", artifact_path.display()))?;
    let a: ProofArtifact = serde_json::from_str(&raw).context("parsing artifact JSON")?;

    // allocations as the JSON stellar-cli expects for a Vec<(Address, i128)> arg:
    // [["G…","300"], …] — i128 passed as a string, order preserved (allocation_root is ordered).
    let allocs: Vec<(String, String)> =
        a.allocations.iter().map(|x| (x.payee.clone(), x.amount.to_string())).collect();
    let allocs_json = serde_json::to_string(&allocs)?;

    let mut cmd = Command::new("stellar");
    cmd.args([
        "contract", "invoke",
        "--id", &settlement,
        "--source", &source,
        "--network", &network,
        "--send", "yes",
        "--",
        "settle",
        "--proof", &a.seal,
        "--journal", &a.journal,
        "--allocations", &allocs_json,
    ]);

    if dry_run {
        let rendered: Vec<String> =
            cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        println!("stellar {}", rendered.join(" "));
        return Ok(());
    }

    eprintln!("submitting settle() to {settlement} on {network} ({} payouts)…", a.allocations.len());
    let status = cmd
        .status()
        .context("running `stellar contract invoke` (is stellar-cli on PATH?)")?;
    anyhow::ensure!(status.success(), "stellar invoke failed with status {status}");
    eprintln!("✓ settled");
    Ok(())
}
