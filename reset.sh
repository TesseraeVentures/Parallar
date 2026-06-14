#!/usr/bin/env bash
#
# Parallar reset — return a clone to a clean pre-demo state.
#
# The counterpart to demo.sh: `cargo clean` for BOTH workspaces (the root contracts
# workspace and the prover workspace) and remove the throwaway artifacts the "Going live"
# path can leave behind (a root proof.json / witness.json, /tmp leftovers). After this,
# the next `./demo.sh` (or `make demo`) builds from scratch.
#
# Safe + idempotent: it NEVER touches tracked source or the committed real-proof fixtures
# (prover/host/tests/fixtures/real_proof.json · witness.json · groth16_verifier.wasm — the
# data demo.sh verifies against). Run it as often as you like.
set -euo pipefail

cd "$(dirname "$0")"

bold=$(tput bold 2>/dev/null || true); dim=$(tput dim 2>/dev/null || true)
grn=$(tput setaf 2 2>/dev/null || true); cyn=$(tput setaf 6 2>/dev/null || true)
ylw=$(tput setaf 3 2>/dev/null || true); rst=$(tput sgr0 2>/dev/null || true)

step=0
beat() { step=$((step+1)); printf "\n${bold}${cyn}━━ %d. %s${rst}\n" "$step" "$1"; [ $# -gt 1 ] && printf "${dim}   %s${rst}\n" "$2" || true; }
note() { printf "${dim}   %s${rst}\n" "$1"; }
ok()   { printf "${grn}   ✓ %s${rst}\n" "$1"; }

# du -sh that never fails (missing dir → 0)
size() { du -sh "$1" 2>/dev/null | awk '{print $1}' || echo 0; }

printf "${bold}${cyn}\n"
printf "  ╔══════════════════════════════════════════════════════════════╗\n"
printf "  ║   PARALLAR — reset to a clean pre-demo state                  ║\n"
printf "  ╚══════════════════════════════════════════════════════════════╝${rst}\n"
note "cargo clean (both workspaces) + drop throwaway artifacts. Source & fixtures untouched."

# ── 0. before ─────────────────────────────────────────────────────────────────
beat "Before" "build output the next demo will regenerate"
note "contracts target/        $(size target)"
note "prover    prover/target/  $(size prover/target)"

# ── 1. cargo clean — contracts workspace ──────────────────────────────────────
beat "cargo clean — root contracts workspace (factory · bond · vault · settlement)"
cargo clean
ok "target/ cleaned"

# ── 2. cargo clean — prover workspace ─────────────────────────────────────────
beat "cargo clean — prover workspace (host · guests · methods)"
cargo clean --manifest-path prover/Cargo.toml
ok "prover/target/ cleaned"

# ── 3. drop throwaway demo artifacts (NOT the committed fixtures) ──────────────
beat "Remove throwaway artifacts" \
     "the 'Going live' CLI writes proof.json / witness.json to the repo root; clear /tmp leftovers"
removed=0
for f in proof.json witness.json; do
  if [ -f "$f" ]; then rm -f "$f"; ok "removed ./$f"; removed=$((removed+1)); fi
done
for d in /tmp/parallar /tmp/parallar-*; do
  if [ -e "$d" ]; then rm -rf "$d"; ok "removed $d"; removed=$((removed+1)); fi
done
[ "$removed" -eq 0 ] && note "none present (already clean)"

# committed fixtures are never deleted — confirm they survived
beat "Committed fixtures preserved" "real-proof data demo.sh verifies against (never deleted)"
fixtures=prover/host/tests/fixtures
for f in real_proof.json witness.json groth16_verifier.wasm; do
  if [ -f "$fixtures/$f" ]; then ok "$fixtures/$f"
  else printf "${ylw}   ! missing $fixtures/$f — reset did NOT remove it; check git status${rst}\n"; fi
done

# ── 4. after ──────────────────────────────────────────────────────────────────
beat "After"
note "contracts target/        $(size target)"
note "prover    prover/target/  $(size prover/target)"

printf "\n${bold}${grn}  ✓ Reset complete — clone is in a clean pre-demo state.${rst}\n"
printf "${dim}  Rebuild + replay the full scenario:  ./demo.sh   (or  make demo)${rst}\n\n"
