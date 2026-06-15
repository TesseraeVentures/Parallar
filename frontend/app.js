// Parallar frontend: shared on-chain data loader.
// Reads the committed deployment record, then best-effort confirms state live
// from testnet RPC. Every write is element-guarded, so the same script drives
// every page; a page simply omits the ids it does not use.
const EXPLORER = "https://stellar.expert/explorer/testnet";
const DEPLOY_URL = "../deployments/testnet.json";

const $   = (id) => document.getElementById(id);
const set = (id, v) => { const e = $(id); if (e) e.textContent = v; };
const href = (id, h) => { const e = $(id); if (e) e.href = h; };
const short = (s, n = 4) => (s ? `${s.slice(0, n)}…${s.slice(-4)}` : "");
const el = (h) => { const t = document.createElement('template'); t.innerHTML = h.trim(); return t.content.firstChild; };

// compact instrument table (home + testnet): #dep-rows
function renderDeployed(d, live) {
  const host = $('dep-rows'); if (!host) return;
  host.innerHTML = '';
  d.instruments.forEach((ins, i) => {
    const settled = live?.[i] ? live[i].settled : !!ins.settled;
    const status = settled
      ? `<span class="tag settled"><span class="dot"></span> Settled</span>`
      : `<span class="tag deployed"><span class="dot"></span> Deployed</span>`;
    host.appendChild(el(`<div class="dep-row">
      <div><div class="name">credit_v1 #${i + 1}</div><div class="sm">vault ${short(ins.vault)}</div></div>
      <div class="sm">XLM</div>
      <div><a href="${EXPLORER}/contract/${ins.settlement}" target="_blank" rel="noopener">${short(ins.settlement, 6)}</a></div>
      <div>${status}</div>
    </div>`));
  });
}

// detailed instrument table (testnet page): #inst-rows
function renderInstrumentsDetailed(d, live) {
  const host = $('inst-rows'); if (!host) return;
  host.innerHTML = '';
  d.instruments.forEach((ins, i) => {
    const settled = live?.[i] ? live[i].settled : !!ins.settled;
    const reserve = live?.[i]?.reserve ?? ins.reserve;
    const status = settled
      ? `<span class="tag settled"><span class="dot"></span> Settled</span>`
      : `<span class="tag deployed"><span class="dot"></span> Deployed</span>`;
    host.appendChild(el(`<div class="dep-row">
      <div class="name">#${i + 1}</div>
      <div><a href="${EXPLORER}/contract/${ins.vault}" target="_blank" rel="noopener">${short(ins.vault, 6)}</a></div>
      <div class="sm">${reserve} XLM</div>
      <div class="sm">${ins.cover} cover</div>
      <div><a href="${EXPLORER}/contract/${ins.settlement}" target="_blank" rel="noopener">${short(ins.settlement, 6)}</a></div>
      <div>${status}</div>
    </div>`));
  });
}

// contract address rows (testnet page)
function fillContracts(d) {
  href('c-factory', `${EXPLORER}/contract/${d.contracts.factory}`);
  set('c-factory', short(d.contracts.factory, 8));
  href('c-verifier', `${EXPLORER}/contract/${d.contracts.groth16_verifier}`);
  set('c-verifier', short(d.contracts.groth16_verifier, 8));
  href('c-sac', `${EXPLORER}/contract/${d.contracts.collateral_xlm_sac}`);
  set('c-sac', short(d.contracts.collateral_xlm_sac, 8));
  set('c-imageid', d.type ? short(d.type.image_id, 8) : '');
  set('c-typeid', d.type ? d.type.type_id : '');
}

async function load() {
  let d;
  try { d = await (await fetch(DEPLOY_URL, { cache: 'no-store' })).json(); }
  catch (e) {
    const n = $('dep-note'); if (n) n.innerHTML = '<span class="err">Serve from the repo root (make frontend) to load on-chain state.</span>';
    return;
  }

  href('nav-explorer', `${EXPLORER}/contract/${d.contracts.factory}`);
  href('f-factory', `${EXPLORER}/contract/${d.contracts.factory}`);
  href('f-verifier', `${EXPLORER}/contract/${d.contracts.groth16_verifier}`);
  set('s-mkts', d.instruments.length);
  set('s-settled', d.instruments.filter(x => x.settled).length);

  const sj = d.instruments.find(x => x.settled);
  if (sj) { set('r-reserve', sj.reserve); set('r-payout', sj.cover); }
  if (d.live_settlement) href('r-tx', `${EXPLORER}/tx/${d.live_settlement.settle_tx}`);

  renderDeployed(d, null);
  renderInstrumentsDetailed(d, null);
  fillContracts(d);

  // best-effort: confirm state live on testnet RPC (CDN-dependent; fails soft)
  try {
    const sdk = (await import('https://esm.sh/@stellar/stellar-sdk@13')).default;
    const { rpc, Contract, TransactionBuilder, Account, Networks, scValToNative, nativeToScVal } = sdk;
    const server = new rpc.Server('https://soroban-testnet.stellar.org');
    const src = new Account(d.accounts.admin, '0');
    const read = async (id, m, a = []) => {
      const tx = new TransactionBuilder(src, { fee: '100', networkPassphrase: Networks.TESTNET })
        .addOperation(new Contract(id).call(m, ...a)).setTimeout(30).build();
      const sim = await server.simulateTransaction(tx);
      if (rpc.Api.isSimulationSuccess(sim) && sim.result) return scValToNative(sim.result.retval);
      throw new Error('sim');
    };
    const live = await Promise.all(d.instruments.map(async ins => ({
      settled: await read(ins.settlement, 'is_settled', [nativeToScVal(1, { type: 'u32' })]),
      reserve: String(await read(ins.vault, 'total_collateral')),
    })));
    set('net-state', 'live ✓');
    set('s-verifier', 'live ✓');
    set('s-settled', live.filter(x => x.settled).length);
    const si = d.instruments.findIndex(x => x.settled);
    if (si >= 0) set('r-reserve', live[si].reserve);
    renderDeployed(d, live);
    renderInstrumentsDetailed(d, live);
    const n = $('dep-note'); if (n) n.textContent = 'Live from testnet RPC. Factory-deployed credit_v1 instruments; the third has settled, the others demonstrate the one-transaction replication.';
  } catch (e) {
    set('net-state', 'records');
    console.warn('live reads unavailable; showing recorded deployment', e);
  }
}
load();
