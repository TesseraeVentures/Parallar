// Parallar interactive testnet dApp.
//
// Connect Freighter, deposit collateral (underwriter), buy cover (bondholder): real signed
// transactions on Stellar testnet against the deployed contracts. READ + COMMIT ONLY: the only
// writes are vault.deposit and vault.buy_protection, neither of which can move the reserve to a
// payee. Payouts happen solely through a settlement that verifies a proof on-chain (Law #1).
//
// The cover commitment is computed in-browser by commit.wasm, the guest's actual Poseidon
// function compiled to wasm, so a bought position is byte-identically settleable (privacy
// honored, not faked).

const EXPLORER = "https://stellar.expert/explorer/testnet";
const RPC = "https://soroban-testnet.stellar.org";
const DEPLOY_URL = "../deployments/testnet.json";

const $ = (id) => document.getElementById(id);
const short = (s, n = 5) => (s ? `${s.slice(0, n)}…${s.slice(-4)}` : "");
const toHex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, "0")).join("");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function out(id, html, kind) {
  const e = $(id);
  e.className = "out show";
  e.innerHTML = kind === "err" ? `<span class="b">${html}</span>` : html;
}

let sdk, fapi, wasm, deploy, account = null, selected = 0;

// ---- commit.wasm bridge (parity-verified: identical to the guest commitment) ----
async function loadWasm() {
  const bytes = await (await fetch("commit.wasm")).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return instance.exports;
}
function commit(buyerXdr, coverBig, salt32) {
  const w = wasm;
  const bptr = w.parallar_alloc(buyerXdr.length);
  const sptr = w.parallar_alloc(32);
  const optr = w.parallar_alloc(32);
  new Uint8Array(w.memory.buffer).set(buyerXdr, bptr);
  new Uint8Array(w.memory.buffer).set(salt32, sptr);
  const mask = 0xffffffffffffffffn;
  w.parallar_commit(bptr, buyerXdr.length, coverBig & mask, (coverBig >> 64n) & mask, sptr, optr);
  return new Uint8Array(w.memory.buffer).slice(optr, optr + 32);
}

// ---- deployment + instrument picker ----
function renderPicker(live) {
  const host = $("picker");
  host.innerHTML = "";
  deploy.instruments.forEach((ins, i) => {
    const reserve = live?.[i]?.reserve ?? ins.reserve;
    const settled = live?.[i] ? live[i].settled : !!ins.settled;
    const row = document.createElement("div");
    row.className = "pick" + (i === selected ? " sel" : "");
    row.innerHTML = `<span class="nm">credit_v1 #${i + 1} ${settled ? "· settled" : ""}</span>
      <span class="meta">reserve ${reserve} · cover ${ins.cover} · vault ${short(ins.vault, 4)}</span>`;
    row.onclick = () => { selected = i; renderPicker(live); };
    host.appendChild(row);
  });
}

// ---- Soroban invoke: build → prepare → Freighter sign → submit → poll ----
async function invoke(contractId, method, args) {
  const { rpc, Contract, TransactionBuilder, Networks, BASE_FEE } = sdk;
  const server = new rpc.Server(RPC);
  const source = await server.getAccount(account);
  let tx = new TransactionBuilder(source, { fee: String(Number(BASE_FEE) * 100), networkPassphrase: Networks.TESTNET })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(120)
    .build();
  tx = await server.prepareTransaction(tx);
  const res = await fapi.signTransaction(tx.toXDR(), { networkPassphrase: Networks.TESTNET, address: account });
  if (res?.error) throw new Error(typeof res.error === "string" ? res.error : JSON.stringify(res.error));
  const signedXdr = res.signedTxXdr ?? res;
  const signed = TransactionBuilder.fromXDR(signedXdr, Networks.TESTNET);
  const sent = await server.sendTransaction(signed);
  if (sent.status === "ERROR") throw new Error("submit failed: " + JSON.stringify(sent.errorResult ?? sent));
  let g;
  for (let i = 0; i < 30; i++) {
    await sleep(1500);
    g = await server.getTransaction(sent.hash);
    if (g.status !== "NOT_FOUND") break;
  }
  if (g.status !== "SUCCESS") throw new Error("transaction " + (g?.status ?? "timed out"));
  return sent.hash;
}

// ---- actions ----
async function doDeposit() {
  const { nativeToScVal, Address } = sdk;
  try {
    const amt = BigInt($("dep-amt").value || "0");
    if (amt <= 0n) return out("deposit-out", "enter a positive amount", "err");
    out("deposit-out", "building + signing deposit…");
    const vault = deploy.instruments[selected].vault;
    const args = [new Address(account).toScVal(), nativeToScVal(amt, { type: "i128" })];
    const hash = await invoke(vault, "deposit", args);
    out("deposit-out", `<span class="g">deposited ${amt} to credit_v1 #${selected + 1}.</span>\n<a href="${EXPLORER}/tx/${hash}" target="_blank" rel="noopener">view tx ↗</a>`);
    refreshLive();
  } catch (e) {
    out("deposit-out", "deposit failed: " + (e?.message ?? e), "err");
  }
}

async function doBuy() {
  const { nativeToScVal, Address } = sdk;
  try {
    const cover = BigInt($("cov-amt").value || "0");
    if (cover <= 0n) return out("buy-out", "enter a positive cover", "err");
    out("buy-out", "computing commitment + signing…");
    const vault = deploy.instruments[selected].vault;
    const buyerXdr = new Uint8Array(new Address(account).toScVal().toXDR());
    const salt = crypto.getRandomValues(new Uint8Array(32));
    const c = commit(buyerXdr, cover, salt);
    const args = [
      new Address(account).toScVal(),
      nativeToScVal(c, { type: "bytes" }),
      nativeToScVal(cover, { type: "i128" }),
    ];
    const hash = await invoke(vault, "buy_protection", args);
    out(
      "buy-out",
      `<span class="g">bought ${cover} of cover. Your size is private (only the commitment is on-chain).</span>\n` +
        `commitment ${toHex(c)}\n` +
        `SAVE YOUR OPENING (needed to settle):\n  cover = ${cover}\n  salt  = ${toHex(salt)}\n` +
        `<a href="${EXPLORER}/tx/${hash}" target="_blank" rel="noopener">view tx ↗</a>`
    );
    refreshLive();
  } catch (e) {
    const m = e?.message ?? String(e);
    const hint = /insolvent|exceed/i.test(m) ? "  (cover must fit under the reserve; deposit collateral first)" : "";
    out("buy-out", "buy failed: " + m + hint, "err");
  }
}

// ---- wallet ----
async function connect() {
  if (!fapi) return out("wallet-out", "Freighter not detected. Install it from freighter.app and reload.", "err");
  try {
    const conn = await fapi.isConnected();
    if (!(conn?.isConnected ?? conn)) return out("wallet-out", "Freighter not detected. Install it and reload.", "err");
    const access = await fapi.requestAccess();
    if (access?.error) return out("wallet-out", "access denied: " + access.error, "err");
    account = access?.address ?? access;
    if (!account) { const a = await fapi.getAddress(); account = a?.address ?? a; }
    const net = await fapi.getNetwork().catch(() => null);
    const passphrase = net?.networkPassphrase ?? "";
    const onTestnet = !net || /Test SDF Network/i.test(passphrase) || /TESTNET/i.test(net?.network ?? "");
    $("wallet").innerHTML = `<span class="ok">connected</span> ${short(account, 6)}`;
    $("deposit").disabled = false;
    $("buy").disabled = false;
    out("wallet-out", onTestnet
      ? `<span class="g">ready on testnet.</span> Actions below sign with your wallet.`
      : `<span class="b">warning: your wallet is not on Test SDF Network. Switch networks, or transactions will fail.</span>`);
  } catch (e) {
    out("wallet-out", "connect failed: " + (e?.message ?? e), "err");
  }
}

async function refreshLive() {
  try {
    const { rpc, Contract, TransactionBuilder, Account, Networks, scValToNative, nativeToScVal } = sdk;
    const server = new rpc.Server(RPC);
    const src = new Account(deploy.accounts.admin, "0");
    const read = async (id, m, a = []) => {
      const tx = new TransactionBuilder(src, { fee: "100", networkPassphrase: Networks.TESTNET })
        .addOperation(new Contract(id).call(m, ...a)).setTimeout(30).build();
      const sim = await server.simulateTransaction(tx);
      if (rpc.Api.isSimulationSuccess(sim) && sim.result) return scValToNative(sim.result.retval);
      throw new Error("sim");
    };
    const live = await Promise.all(deploy.instruments.map(async (ins) => ({
      reserve: String(await read(ins.vault, "total_collateral")),
      settled: await read(ins.settlement, "is_settled", [nativeToScVal(1, { type: "u32" })]),
    })));
    $("net-state").textContent = "live ✓";
    renderPicker(live);
  } catch (e) {
    $("net-state").textContent = "records";
    console.warn("live reads unavailable; showing recorded deployment", e);
  }
}

async function boot() {
  deploy = await (await fetch(DEPLOY_URL, { cache: "no-store" })).json();
  $("nav-explorer").href = `${EXPLORER}/contract/${deploy.contracts.factory}`;
  if ($("f-factory")) $("f-factory").href = `${EXPLORER}/contract/${deploy.contracts.factory}`;
  if ($("f-verifier")) $("f-verifier").href = `${EXPLORER}/contract/${deploy.contracts.groth16_verifier}`;
  renderPicker(null);

  const loads = await Promise.allSettled([
    import("https://esm.sh/@stellar/stellar-sdk@13").then((m) => m.default),
    import("https://esm.sh/@stellar/freighter-api@4").then((m) => m.default ?? m),
    loadWasm(),
  ]);
  sdk = loads[0].status === "fulfilled" ? loads[0].value : null;
  fapi = loads[1].status === "fulfilled" ? loads[1].value : null;
  wasm = loads[2].status === "fulfilled" ? loads[2].value : null;

  $("connect").onclick = connect;
  $("deposit").onclick = doDeposit;
  $("buy").onclick = doBuy;
  if (!sdk) out("wallet-out", "could not load the Stellar SDK (CDN). Reload to retry.", "err");

  // expose the parity-verified commitment for inspection/testing
  window.__parallar = {
    ready: !!(sdk && wasm),
    commit: (hexBuyer, cover, hexSalt) => {
      const b = Uint8Array.from(hexBuyer.match(/../g).map((h) => parseInt(h, 16)));
      const s = Uint8Array.from(hexSalt.match(/../g).map((h) => parseInt(h, 16)));
      return toHex(commit(b, BigInt(cover), s));
    },
  };

  refreshLive();
}
boot();
