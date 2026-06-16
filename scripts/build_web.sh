#!/usr/bin/env bash
#
# Build the two CLEANLY-SEPARATED deploy docroots from the flat frontend/ source:
#   dist/www → the marketing site ONLY (index + bondholders/underwriters/how-it-works/testnet)
#   dist/app → the dApp ONLY        (app.html served as index.html + dapp.js + commit.wasm)
#
# The shared top-nav links between the site and the dApp are rewired to ABSOLUTE cross-subdomain
# URLs here, so app.parallar.com carries no marketing pages and www.parallar.com carries no dApp.
# The SOURCE frontend/ stays flat with relative links, so local dev (`make frontend`) keeps the
# seamless integrated nav. Run on the server before pointing nginx at dist/ (see docs/DEPLOYMENT.md).
set -euo pipefail
cd "$(dirname "$0")/.."

WWW_HOST="${WWW_HOST:-https://www.parallar.com}"
APP_HOST="${APP_HOST:-https://app.parallar.com}"

rm -rf dist/www dist/app
mkdir -p dist/www dist/app

# ── www: the marketing site (every page EXCEPT the dApp) + its live-state loader ──
cp frontend/index.html frontend/bondholders.html frontend/underwriters.html \
   frontend/how-it-works.html frontend/testnet.html frontend/styles.css frontend/app.js dist/www/
# the "Try it" nav link → the dApp subdomain
sed -i.bak -E "s#href=\"app\.html\"#href=\"$APP_HOST/\"#g" dist/www/*.html
rm -f dist/www/*.bak

# ── app: ONLY the dApp (app.html becomes the index) + its bridge + the Poseidon wasm ──
cp frontend/app.html dist/app/index.html
cp frontend/dapp.js frontend/commit.wasm frontend/styles.css dist/app/
# the dApp's nav back to the marketing pages → the www subdomain; "Try it"/wordmark → app root
sed -i.bak -E \
  -e "s#href=\"index\.html\"#href=\"$WWW_HOST/\"#g" \
  -e "s#href=\"bondholders\.html\"#href=\"$WWW_HOST/bondholders.html\"#g" \
  -e "s#href=\"underwriters\.html\"#href=\"$WWW_HOST/underwriters.html\"#g" \
  -e "s#href=\"how-it-works\.html\"#href=\"$WWW_HOST/how-it-works.html\"#g" \
  -e "s#href=\"testnet\.html\"#href=\"$WWW_HOST/testnet.html\"#g" \
  -e "s#href=\"app\.html\"#href=\"$APP_HOST/\"#g" \
  dist/app/index.html
rm -f dist/app/*.bak

echo "✓ built:"
echo "   dist/www  → www.parallar.com  ($(ls dist/www | wc -l | tr -d ' ') files; marketing site)"
echo "   dist/app  → app.parallar.com  ($(ls dist/app | wc -l | tr -d ' ') files; dApp only)"
echo "  point nginx 'root' at these (docs/DEPLOYMENT.md). Re-run after editing frontend/."
