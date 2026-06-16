#!/usr/bin/env bash
#
# Provision a fresh x86 Linux box (e.g. Hetzner, Ubuntu 22.04/24.04) to BUILD, TEST, and PROVE
# Parallar — the one machine that does everything a near-full laptop can't (real Groth16 proving
# runs natively on x86, no Rosetta). Run from the repo root after transferring the folder:
#
#   bash scripts/provision_server.sh            # toolchain + build + full test suite
#   bash scripts/provision_server.sh --no-test  # toolchain + build only (faster)
#
# This does NOT touch DNS, nginx, or any secret — see docs/DEPLOYMENT.md for the web/domain step.
# Idempotent: safe to re-run. Needs ~30 GB free disk + ≥ 8 GB RAM for the RISC Zero guest build.
set -euo pipefail
cd "$(dirname "$0")/.."
RUN_TESTS=1; [ "${1:-}" = "--no-test" ] && RUN_TESTS=0

say() { printf "\n\033[1;36m== %s ==\033[0m\n" "$1"; }

say "1/6 · system packages (build-essential, ssl, clang, git)"
if command -v apt-get >/dev/null; then
  sudo apt-get update -y
  sudo apt-get install -y build-essential pkg-config libssl-dev clang curl git
else
  echo "non-apt distro: install build-essential/pkg-config/libssl-dev/clang/curl/git manually"
fi

say "2/6 · Rust (rustup) + the Soroban wasm target"
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
. "$HOME/.cargo/env"
rustup target add wasm32v1-none 2>/dev/null || true

say "3/6 · RISC Zero toolchain (rzup) — the zkVM prover (x86-native here)"
if ! command -v rzup >/dev/null && [ ! -x "$HOME/.risc0/bin/rzup" ]; then
  curl -L https://risczero.com/install | bash
fi
export PATH="$HOME/.risc0/bin:$PATH"
rzup install

say "4/6 · stellar-cli (deploy + submit)"
if ! command -v stellar >/dev/null; then
  cargo install --locked stellar-cli   # prebuilt binaries: github.com/stellar/stellar-cli/releases
fi

say "5/6 · build — contracts (wasm) + guests + the zkVM methods (ELFs/image_ids)"
stellar contract build
( cd prover && cargo build -p parallar-methods )   # cross-compiles the 6 guests; mints the image_ids
./scripts/check_image_ids.sh                        # assert all 6 reproduce (credit_v1 == deployed 705ddac4…)

if [ "$RUN_TESTS" = "1" ]; then
  say "6/6 · test — full suite (contracts + guests + proptests + on-chain verify)"
  cargo test                                                   # 12 contracts (wasm built above)
  ( cd prover && cargo test -p settle-credit-v1 -p settle-weather-v1 -p settle-credit-v2 \
      -p settle-credit-v3 -p claim-credit-v1 -p solvency-v1 -p parallar-proptests )
  ( cd prover && cargo test -p parallar-prover-host --test onchain_verify )   # real Groth16 verify
else
  say "6/6 · tests skipped (--no-test)"
fi

cat <<'EOF'

✓ Parallar is built and the toolchain is ready.

Next (founder, on this x86 box):
  • Generate real proofs + benchmark        → docs/RUNBOOK.md §1–§2
  • Deploy the families to testnet          → scripts/deploy_*.sh  (docs/RUNBOOK.md §3)
  • Serve the site + dApp (nginx + TLS)      → docs/DEPLOYMENT.md
  • Put your funded testnet keys in .env     (gitignored; copy .env.example)
EOF
