//! `parallar-prover` — the settlement prover host CLI.
//!
//! * `prove`  — run the RISC Zero settlement guest under the real Groth16 prover and write a
//!              submittable proof artifact (needs Docker/x86; ~34 min via Rosetta — §2).
//! * `submit` — invoke `settlement.settle(proof, journal, allocations)` on a deployed
//!              instrument via `stellar-cli`, from a proof artifact.
//! * `bench`  — time the real prover over a witness N times (the founder's x86 N=10 run,
//!              TECH_SPEC §10.7); same Docker/x86 + ~34 min/proof cost as `prove`.
//!
//! `history-builder` (assemble the witness `Inputs` by scanning qualifying payments per
//! TECH_SPEC §10) is the next subcommand; until then the witness JSON is produced out-of-band.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use parallar_prover_host::{prove_settlement, prove_weather_settlement, ProofArtifact};
use settle_credit_v1::Inputs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "parallar-prover", about = "Parallar settlement prover host: prove + submit")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Which registered guest type a witness targets. `prove`/`bench` dispatch on this; `submit`
/// is guest-agnostic (it consumes a finished artifact). A new guest = a new type, never an edit.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum GuestKind {
    /// settle_credit_v1 — coupon-default protection (instrument #1).
    Credit,
    /// settle_weather_v1 — parametric rainfall-shortfall protection (instrument #2).
    Weather,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the settlement guest under the real Groth16 prover; write a submittable artifact.
    Prove {
        /// Witness JSON — the guest `Inputs` (config + witness; shape depends on `--guest`).
        #[arg(long)]
        inputs: PathBuf,
        /// Where to write the proof artifact JSON.
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
        /// Which guest type the witness targets.
        #[arg(long, value_enum, default_value_t = GuestKind::Credit)]
        guest: GuestKind,
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
    /// Benchmark the real Groth16 prover: run `prove_settlement` N times over a fixed witness
    /// and print per-run + min/mean/max timings (the founder's x86 N=10 run, TECH_SPEC §10.7).
    /// Each run needs Docker/x86 and is ~34 min — this is a deliberate command, NOT a test.
    Bench {
        /// Witness JSON — the guest `Inputs` to prove on every run (shape depends on `--guest`).
        #[arg(long)]
        inputs: PathBuf,
        /// Number of proving runs to time (the DoD figure is 10).
        #[arg(long, default_value_t = 10)]
        n: u32,
        /// Which guest type the witness targets.
        #[arg(long, value_enum, default_value_t = GuestKind::Credit)]
        guest: GuestKind,
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
        Cmd::Prove { inputs, out, guest } => cmd_prove(inputs, out, guest),
        Cmd::Submit { artifact, settlement, source, network, dry_run } => {
            cmd_submit(artifact, settlement, source, network, dry_run)
        }
        Cmd::Bench { inputs, n, guest } => cmd_bench(inputs, n, guest),
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

fn cmd_prove(inputs_path: PathBuf, out: PathBuf, guest: GuestKind) -> Result<()> {
    let raw = std::fs::read_to_string(&inputs_path)
        .with_context(|| format!("reading witness {}", inputs_path.display()))?;

    eprintln!("proving settlement — real Groth16 STARK→SNARK (needs Docker/x86; this is slow)…");
    let artifact = match guest {
        GuestKind::Credit => {
            let inputs: Inputs = serde_json::from_str(&raw).context("parsing credit witness JSON")?;
            prove_settlement(&inputs)?
        }
        GuestKind::Weather => {
            let inputs: settle_weather_v1::Inputs =
                serde_json::from_str(&raw).context("parsing weather witness JSON")?;
            prove_weather_settlement(&inputs)?
        }
    };

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

/// On-chain `settle()` verify cost captured in Sprint 0 (real Groth16 verify on Soroban
/// testnet) — context for the prover-side timings. Pinned figure; ~3x headroom under the
/// ~100M-CPU-insn per-tx budget.
const ONCHAIN_VERIFY_INSNS: u64 = 35_000_000;
const SOROBAN_TX_INSN_BUDGET: u64 = 100_000_000;

fn cmd_bench(inputs_path: PathBuf, n: u32, guest: GuestKind) -> Result<()> {
    anyhow::ensure!(n >= 1, "--n must be at least 1");

    let raw = std::fs::read_to_string(&inputs_path)
        .with_context(|| format!("reading witness {}", inputs_path.display()))?;

    // Dispatch once: parse the right witness type and capture a closure that proves it, so the
    // timing loop below is identical for both guests.
    let (scale_label, scale, prove_once): (&str, usize, Box<dyn Fn() -> Result<ProofArtifact>>) =
        match guest {
            GuestKind::Credit => {
                let inputs: Inputs =
                    serde_json::from_str(&raw).context("parsing credit witness JSON")?;
                let scale = inputs.snapshot.len();
                ("holders", scale, Box::new(move || prove_settlement(&inputs)))
            }
            GuestKind::Weather => {
                let inputs: settle_weather_v1::Inputs =
                    serde_json::from_str(&raw).context("parsing weather witness JSON")?;
                let scale = inputs.observations.len();
                ("observations", scale, Box::new(move || prove_weather_settlement(&inputs)))
            }
        };

    eprintln!(
        "benchmarking real Groth16 prover: {n} run(s) over {} ({scale_label}={scale}).",
        inputs_path.display(),
    );
    eprintln!(
        "NOTE: each run is the real STARK→SNARK wrap — needs Docker/x86 (Rosetta on Apple\n\
         Silicon), ~34 min/proof. This is a deliberate command, not part of `cargo test`.",
    );

    let mut secs: Vec<f64> = Vec::with_capacity(n as usize);
    println!("run    seconds");
    println!("---    -------");
    for run in 1..=n {
        let t0 = Instant::now();
        let _artifact = prove_once().with_context(|| format!("prove failed on run {run}/{n}"))?;
        let elapsed = t0.elapsed().as_secs_f64();
        secs.push(elapsed);
        println!("{run:>3}    {elapsed:>9.1}");
    }

    let min = secs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = secs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = secs.iter().sum::<f64>() / secs.len() as f64;

    println!();
    println!("summary (n={n}, {scale_label}={scale})");
    println!("  min  {min:>9.1}s");
    println!("  mean {mean:>9.1}s");
    println!("  max  {max:>9.1}s");
    println!();
    // Crude extrapolation: proving cost is dominated by the fixed STARK→SNARK wrap, so the
    // per-item marginal is small at these N — an order-of-magnitude note, not a measured 1k figure.
    println!(
        "scale note: prove time here is dominated by the fixed Groth16 wrap, not the {scale}\n\
         -{scale_label} fold; a 1k-{scale_label} witness is expected to stay the same order of\n\
         magnitude (~{mean:.0}s). This is an extrapolation — confirm with `bench` on a 1k witness.",
    );
    println!();
    println!(
        "on-chain verify (Sprint-0 capture): ~{}M CPU insns to verify one proof on Soroban,\n\
         ~{:.1}x headroom under the ~{}M/tx budget — verify cost is flat in {scale_label} count.",
        ONCHAIN_VERIFY_INSNS / 1_000_000,
        SOROBAN_TX_INSN_BUDGET as f64 / ONCHAIN_VERIFY_INSNS as f64,
        SOROBAN_TX_INSN_BUDGET / 1_000_000,
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
