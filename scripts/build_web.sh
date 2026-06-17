#!/usr/bin/env bash
#
# Assemble the SINGLE-SURFACE web root served by Cloudflare (the parallar-www Worker → ./dist):
# the flat frontend/ site + dApp (relative links, one integrated nav) PLUS the canonical
# deployments/testnet.json. app.js reads it at /deployments/testnet.json to populate the live
# explorer links + on-chain reads — and deployments/ lives OUTSIDE frontend/, so it must be copied
# into the served root here (this is why serving frontend/ raw left every explorer link dead).
#
# Local dev (`make frontend`) serves the repo root, where both frontend/ and deployments/ already
# sit, so no build is needed locally. Cloudflare parallar-www → Build command: bash scripts/build_web.sh
# (the Worker serves ./dist per wrangler.www.toml). Re-run after editing frontend/ or testnet.json.
set -euo pipefail
cd "$(dirname "$0")/.."

rm -rf dist
mkdir -p dist/deployments
cp frontend/*.html frontend/*.js frontend/*.css frontend/*.wasm dist/
cp deployments/testnet.json dist/deployments/testnet.json

echo "✓ dist/ assembled: $(ls dist/*.html | wc -l | tr -d ' ') pages + js/css/wasm + deployments/testnet.json (Worker serves ./dist)"
