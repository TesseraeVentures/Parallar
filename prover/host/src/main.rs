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
use parallar_prover_host::{
    prove_claim_credit_v1, prove_credit_v2_settlement, prove_credit_v3_settlement, prove_settlement,
    prove_solvency_buy, prove_solvency_withdraw, prove_weather_settlement, ProofArtifact,
};
use parallar_prover_host::keeper::{plan_buy, plan_withdraw, KeeperState};
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
    /// settle_credit_v2 — attested credit (G1): the issuer signs the payment snapshot.
    #[value(name = "credit-v2")]
    Credit2,
    /// settle_credit_v3 — attested + record-date (G4): the issuer signs the per-epoch holder set.
    #[value(name = "credit-v3")]
    Credit3,
    /// claim_credit_v1 — the single-buyer escape-hatch claim (G2); consumed by claim_direct.
    Claim,
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
    /// Run the solvency_v1 guest (G3 confidential cover) under the real Groth16 prover. The witness
    /// is a `SolvencyRequest` (Buy or Withdraw) — `confidential_vault` consumes the resulting
    /// SolvencyProofArtifact via buy_protection_proven / withdraw_proven. Distinct artifact shape
    /// (no allocations) from the settlement guests, hence its own subcommand.
    ProveSolvency {
        /// Witness JSON — a `solvency_v1::SolvencyRequest` (Buy{..} or Withdraw{..}).
        #[arg(long)]
        inputs: PathBuf,
        /// Where to write the solvency proof artifact JSON.
        #[arg(long, default_value = "solvency_proof.json")]
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
    /// Initialise the confidential-cover keeper state for ONE instrument: writes the aggregate
    /// opening (total=0, salt0) and prints `initial_cover_commitment` for the vault's `init`.
    KeeperInit {
        /// salt0 (32-byte hex) — the genesis opening salt; the vault is init'd with commit_total(0, salt0).
        #[arg(long)]
        salt0: String,
        /// Premium rate in basis points (cover × bps / 10_000 = the buyer's upfront premium).
        #[arg(long)]
        premium_bps: u32,
        /// Keeper state file to create.
        #[arg(long, default_value = "keeper_state.json")]
        state: PathBuf,
    },
    /// Confidential BUY: advance the hidden aggregate by `cover`, prove solvency under the real
    /// Groth16 prover, and emit the SolvencyProofArtifact for `buy_protection_proven`. Advances the
    /// keeper state on success. (Single-writer sequencer — see the keeper module.)
    KeeperBuy {
        /// Keeper state file (from keeper-init; advanced in place on success).
        #[arg(long, default_value = "keeper_state.json")]
        state: PathBuf,
        /// Buyer address as the Address ScVal XDR, hex (what the dApp's commit.wasm uses).
        #[arg(long)]
        buyer: String,
        /// Cover amount (hidden on-chain; the keeper learns it to maintain the aggregate).
        #[arg(long)]
        cover: i128,
        /// The vault's current total_collateral (read from chain) — the public solvency bound.
        #[arg(long)]
        collateral: i128,
        /// Where to write the solvency proof artifact.
        #[arg(long, default_value = "buy_proof.json")]
        out: PathBuf,
    },
    /// Confidential WITHDRAW: prove the (unchanged) hidden aggregate still fits under the
    /// post-withdrawal collateral. State is NOT advanced (a withdrawal shrinks collateral, not cover).
    KeeperWithdraw {
        #[arg(long, default_value = "keeper_state.json")]
        state: PathBuf,
        /// total_collateral − the withdrawal amount.
        #[arg(long)]
        collateral_after: i128,
        #[arg(long, default_value = "withdraw_proof.json")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Prove { inputs, out, guest } => cmd_prove(inputs, out, guest),
        Cmd::ProveSolvency { inputs, out } => cmd_prove_solvency(inputs, out),
        Cmd::Submit { artifact, settlement, source, network, dry_run } => {
            cmd_submit(artifact, settlement, source, network, dry_run)
        }
        Cmd::Bench { inputs, n, guest } => cmd_bench(inputs, n, guest),
        Cmd::HistoryBuilder { scan, params, out } => cmd_history_builder(scan, params, out),
        Cmd::KeeperInit { salt0, premium_bps, state } => cmd_keeper_init(salt0, premium_bps, state),
        Cmd::KeeperBuy { state, buyer, cover, collateral, out } => {
            cmd_keeper_buy(state, buyer, cover, collateral, out)
        }
        Cmd::KeeperWithdraw { state, collateral_after, out } => {
            cmd_keeper_withdraw(state, collateral_after, out)
        }
    }
}

// ─────────────────────────── confidential-cover keeper CLI (G3) ───────────────────────────
// Wraps the keeper sequencer (src/keeper.rs) over a JSON state file. keeper-buy/withdraw run the
// REAL Groth16 prover (Docker/x86) on the constructed solvency inputs. Single-writer model: run one
// keeper process per instrument. PRODUCTION note: keeper-buy advances the persisted state on
// successful proof generation; if the on-chain buy_protection_proven submission then fails, re-sync
// the state to the vault's on-chain CoverCommitment before the next buy (advance-on-confirmation).

fn parse_salt32(s: &str) -> Result<[u8; 32]> {
    let v = hex::decode(s).context("salt must be hex")?;
    anyhow::ensure!(v.len() == 32, "salt must be 32 bytes (64 hex chars), got {}", v.len());
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    Ok(a)
}

/// 32 fresh bytes from the OS CSPRNG (the keeper runs on Unix/x86; no extra crate needed).
fn random_salt32() -> Result<[u8; 32]> {
    use std::io::Read;
    let mut a = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .context("opening /dev/urandom")?
        .read_exact(&mut a)
        .context("reading /dev/urandom")?;
    Ok(a)
}

fn load_keeper_state(p: &PathBuf) -> Result<KeeperState> {
    let raw = std::fs::read_to_string(p)
        .with_context(|| format!("reading keeper state {} (run keeper-init first)", p.display()))?;
    serde_json::from_str(&raw).context("parsing keeper state JSON")
}

fn save_keeper_state(p: &PathBuf, s: &KeeperState) -> Result<()> {
    std::fs::write(p, serde_json::to_string_pretty(s)?)
        .with_context(|| format!("writing keeper state {}", p.display()))
}

fn cmd_keeper_init(salt0: String, premium_bps: u32, state: PathBuf) -> Result<()> {
    let st = KeeperState::genesis(parse_salt32(&salt0)?, premium_bps);
    save_keeper_state(&state, &st)?;
    eprintln!(
        "✓ keeper state → {} | premium_bps={premium_bps}\n  initial_cover_commitment = {}  (pass this to confidential_vault.init)",
        state.display(),
        hex::encode(st.commitment()),
    );
    Ok(())
}

fn cmd_keeper_buy(
    state: PathBuf,
    buyer: String,
    cover: i128,
    collateral: i128,
    out: PathBuf,
) -> Result<()> {
    let st = load_keeper_state(&state)?;
    let buyer_xdr = hex::decode(&buyer).context("--buyer must be hex (the Address ScVal XDR)")?;
    let plan = plan_buy(&st, buyer_xdr, cover, collateral, random_salt32()?, random_salt32()?)?;

    eprintln!("proving confidential buy — real Groth16 STARK→SNARK (needs Docker/x86; slow)…");
    let artifact = prove_solvency_buy(&plan.inputs)?;
    std::fs::write(&out, serde_json::to_string_pretty(&artifact)?)
        .with_context(|| format!("writing artifact {}", out.display()))?;
    save_keeper_state(&state, &plan.next)?; // advance-on-proof; re-sync if the submit fails (see note)

    eprintln!(
        "✓ buy proof → {} | premium={} new cover commitment={}",
        out.display(),
        plan.premium,
        hex::encode(plan.next.commitment()),
    );
    eprintln!(
        "  BUYER MUST SAVE THIS OPENING to settle later:  cover={cover}  position_salt={}",
        hex::encode(plan.position_salt),
    );
    eprintln!(
        "  then submit: parallar-prover ... (or stellar invoke buy_protection_proven --seal <artifact.seal> --journal <artifact.journal> --premium {})",
        plan.premium,
    );
    Ok(())
}

fn cmd_keeper_withdraw(state: PathBuf, collateral_after: i128, out: PathBuf) -> Result<()> {
    let st = load_keeper_state(&state)?;
    let inputs = plan_withdraw(&st, collateral_after)?;
    eprintln!("proving confidential withdraw — real Groth16 (needs Docker/x86; slow)…");
    let artifact = prove_solvency_withdraw(&inputs)?;
    std::fs::write(&out, serde_json::to_string_pretty(&artifact)?)
        .with_context(|| format!("writing artifact {}", out.display()))?;
    eprintln!(
        "✓ withdraw proof → {} | aggregate unchanged (commitment {})",
        out.display(),
        hex::encode(st.commitment()),
    );
    Ok(())
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
        GuestKind::Credit2 => {
            let inputs: settle_credit_v2::Inputs =
                serde_json::from_str(&raw).context("parsing credit_v2 witness JSON")?;
            prove_credit_v2_settlement(&inputs)?
        }
        GuestKind::Credit3 => {
            let inputs: settle_credit_v3::Inputs =
                serde_json::from_str(&raw).context("parsing credit_v3 witness JSON")?;
            prove_credit_v3_settlement(&inputs)?
        }
        GuestKind::Claim => {
            let inputs: claim_credit_v1::ClaimInputs =
                serde_json::from_str(&raw).context("parsing claim witness JSON")?;
            prove_claim_credit_v1(&inputs)?
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

fn cmd_prove_solvency(inputs_path: PathBuf, out: PathBuf) -> Result<()> {
    use solvency_v1::SolvencyRequest;
    let raw = std::fs::read_to_string(&inputs_path)
        .with_context(|| format!("reading witness {}", inputs_path.display()))?;
    let req: SolvencyRequest = serde_json::from_str(&raw).context("parsing solvency witness JSON")?;
    let artifact = match req {
        SolvencyRequest::Buy(i) => prove_solvency_buy(&i)?,
        SolvencyRequest::Withdraw(i) => prove_solvency_withdraw(&i)?,
    };
    std::fs::write(&out, serde_json::to_string_pretty(&artifact)?)
        .with_context(|| format!("writing artifact {}", out.display()))?;
    eprintln!(
        "✓ solvency proof → {} | seal={}B image_id={} journal={}B (cover hidden)",
        out.display(),
        hex::decode(&artifact.seal).map(|s| s.len()).unwrap_or(0),
        artifact.image_id,
        hex::decode(&artifact.journal).map(|s| s.len()).unwrap_or(0),
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
            GuestKind::Credit2 => {
                let inputs: settle_credit_v2::Inputs =
                    serde_json::from_str(&raw).context("parsing credit_v2 witness JSON")?;
                let scale = inputs.snapshot.len();
                ("holders", scale, Box::new(move || prove_credit_v2_settlement(&inputs)))
            }
            GuestKind::Credit3 => {
                let inputs: settle_credit_v3::Inputs =
                    serde_json::from_str(&raw).context("parsing credit_v3 witness JSON")?;
                let scale = inputs.snapshot.len();
                ("holders", scale, Box::new(move || prove_credit_v3_settlement(&inputs)))
            }
            GuestKind::Claim => {
                let inputs: claim_credit_v1::ClaimInputs =
                    serde_json::from_str(&raw).context("parsing claim witness JSON")?;
                let scale = inputs.snapshot.len();
                ("holders", scale, Box::new(move || prove_claim_credit_v1(&inputs)))
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
