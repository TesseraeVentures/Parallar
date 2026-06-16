# Parallar — server + domain deployment (Hetzner x86)

The Hetzner box does **two jobs**: it's the **proving box** (x86, so real Groth16 proving runs
natively) *and* the **web host** (nginx serving the static site + dApp). There is **no backend** —
the dApp talks to Stellar testnet RPC + Freighter entirely client-side, so hosting is just static
files. (Companion docs: [RUNBOOK.md](RUNBOOK.md) = proving/benchmark/deploy; [VERIFY.md](../VERIFY.md)
= how anyone checks the claims.)

## 1 · Transfer + build out

```bash
# from your machine — copy the repo (the gitignored .env with testnet keys comes along)
rsync -av --exclude target --exclude prover/target --exclude dist ./parallar/  user@<hetzner-ip>:/srv/parallar/

# on the box — one script does the whole toolchain + build + test
cd /srv/parallar
bash scripts/provision_server.sh         # rust + risc0 + stellar-cli + build + full suite (~30 GB, ≥8 GB RAM)
```

`provision_server.sh` installs the toolchain, builds the contracts (wasm) + the 6-guest zkVM
methods (mints the image_ids), runs `check_image_ids.sh`, and runs the full test suite. After it
passes you can generate proofs, benchmark, and deploy to testnet per [RUNBOOK.md](RUNBOOK.md).

## 2 · Domain + subdomains

Buy **parallar.com** at any registrar (you do this — it's a purchase). Then point DNS at the box.

| Host | Purpose | Needed? |
|---|---|---|
| `parallar.com` (apex) | 301-redirect to `www` | **yes** |
| `www.parallar.com` | the marketing / overview site (`frontend/index.html` + bondholders / underwriters / how-it-works / testnet) | **yes** |
| `app.parallar.com` | the interactive dApp (`frontend/app.html` — Freighter connect, deposit, buy cover) | **yes** |
| `docs.parallar.com` | optional — and if used, render ONLY public-confidence docs (the how-it-works deep-dive + the `VERIFY.md` "check it yourself" guide). **Do NOT publish `PRODUCTION_GAP.md` or `STATUS.md`** — those are the internal workplan + build log (the SDF grant conversation, shared selectively, not a public page that advertises what's unfinished). | optional |
| `api.parallar.com` | — | **not needed** (no backend; the dApp is client-side RPC + Freighter) |

`www` and `app` are **cleanly separated** — each subdomain has its own docroot. `scripts/build_web.sh`
generates two directories from the flat `frontend/` source: `dist/www` (the marketing pages + their
live-state loader) and `dist/app` (the dApp only — `app.html` served as `index.html` + `dapp.js` +
`commit.wasm`), and rewires the shared nav's cross-links to absolute cross-subdomain URLs. So
`app.parallar.com` carries **no** marketing pages and `www.parallar.com` **no** dApp. (Local dev
keeps the integrated nav: `make frontend` serves the flat `frontend/` with relative links; only the
built `dist/` uses the absolute cross-links — re-run `build_web.sh` after editing `frontend/`.)
Email (e.g. `hello@parallar.com`) is MX records at an email provider, not a host you run here.

**DNS records** (at the registrar or Hetzner DNS), with `<ip>` = the box's public IPv4:

```
A     parallar.com        <ip>
A     www.parallar.com    <ip>
A     app.parallar.com    <ip>
A     docs.parallar.com   <ip>     # optional
# add matching AAAA records if the box has IPv6
```

## 3 · Web host (nginx + TLS)

```bash
bash scripts/build_web.sh                      # → dist/www (site) + dist/app (dApp), cross-links rewired
sudo apt-get install -y nginx certbot python3-certbot-nginx
sudo cp deploy/nginx/parallar.conf /etc/nginx/sites-available/parallar.conf
sudo ln -s /etc/nginx/sites-available/parallar.conf /etc/nginx/sites-enabled/
# edit the two `root` lines → /srv/parallar/dist/www and /srv/parallar/dist/app
sudo nginx -t && sudo systemctl reload nginx
# TLS for all names (certbot rewrites the vhosts to add 443 + auto-renew):
sudo certbot --nginx -d parallar.com -d www.parallar.com -d app.parallar.com   # add -d docs.parallar.com if used
```

`commit.wasm` (in `dist/app`) is served as `application/wasm` (the vhost sets it) so the in-browser
Poseidon commitment loads. After reload, `https://www.parallar.com` is the site and
`https://app.parallar.com` is the dApp. **Re-run `build_web.sh` whenever you edit `frontend/`.**

## 4 · Harden the live dApp (recommended before judging)

The dApp currently imports the Stellar SDK + Freighter from the **esm.sh CDN** at runtime
(`frontend/app.js`, `frontend/dapp.js`) and reads testnet RPC live. For a robust judge demo:

- **Vendor the SDK locally** — download `@stellar/stellar-sdk@13` + `@stellar/freighter-api@4`
  into `frontend/vendor/` and change the `import('https://esm.sh/…')` calls in `frontend/dapp.js`
  (+ `app.js`) to the local paths, so a slow/blocked CDN can't degrade `app.parallar.com` mid-demo.
  Then re-run `scripts/build_web.sh` so `dist/app` picks it up.
- **Confirm the live `settle_tx` TTL** hasn't lapsed (`scripts/ttl_monitor.sh`) so the on-chain
  story on the `testnet.html` page / stellar.expert link stays valid.
- The site reads `deployments/testnet.json` for live ids — keep it current as you deploy the new
  families (`scripts/deploy_*.sh`).

## 5 · Going-live checklist

- [ ] `provision_server.sh` green (build + tests + image_ids)
- [ ] DNS A records resolve to the box (`dig www.parallar.com`)
- [ ] `build_web.sh` run → `dist/www` + `dist/app` in place (clean split, links rewired)
- [ ] nginx serving + `certbot` TLS on www / app (+ docs)
- [ ] `.env` populated with funded testnet keys (gitignored; never commit)
- [ ] (recommended) SDK vendored locally; `ttl_monitor.sh` cron’d
- [ ] real proofs generated + (optionally) the new families deployed live → `deployments/testnet.json`

> **Note:** `parallar.com` is a purchase and DNS/registrar changes are yours to make — this repo
> ships the build-out script + nginx config + this runbook, not the account actions.
