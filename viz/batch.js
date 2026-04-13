// Batch runner for the index/results page.
// Loads WASM on demand and runs preset experiment batches.

import { ALL_DOMAINS, RULES_FOR, fetchDomainData, saveSearchResults, getSessionFolder, resetSessionFolder } from './shared.js';

const $ = id => document.getElementById(id);
const status = $('batchStatus');
let wasm = null;

function buildBatches() {
  const key = $('batchPreset').value;

  const bf = (domain, priority = 'cost', name) => ({
    name: name || `${domain}_bf_${priority}`, domain, rules: RULES_FOR[domain],
    search: 'best-first', priority, budget: 500, max_arity: 2,
  });
  const smc = (domain, temp = 1000, name) => ({
    name: name || `${domain}_smc`, domain, rules: RULES_FOR[domain],
    search: 'smc', particles: 1000, steps: 100, temperature: temp, dead_runs: 50, max_arity: 2,
  });

  switch (key) {
    case 'all-bf-cost':    return ALL_DOMAINS.map(d => bf(d));
    case 'all-smc':        return ALL_DOMAINS.map(d => smc(d));
    case 'temp-sweep':     return [1, 10, 100, 1000, 10000].map(t => smc('dials', t, `T${t}`));
    case 'priority-sweep': return ['cost', 'depth-first', 'breadth-first', 'most-matches'].map(p => bf('dials', p, `bf_${p}`));
    default: return [];
  }
}

async function ensureWasm() {
  if (wasm) return;
  status.textContent = 'loading WASM…';
  wasm = await import('../pkg/egg_stitch.js');
  await wasm.default();
}

$('btnBatchRun').addEventListener('click', async () => {
  const btn = $('btnBatchRun');
  btn.disabled = true;
  btn.textContent = 'running…';
  resetSessionFolder();

  try {
    await ensureWasm();
    const items = buildBatches();

    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      status.textContent = `[${i + 1}/${items.length}] ${item.name}…`;

      const { programsText, rulesText } = await fetchDomainData(item.domain, item.rules);
      const configJson = JSON.stringify({
        follow: null, weight_by_usage: false, p_reuse: 0.5,
        max_arity: item.max_arity, priority: item.priority || 'cost',
      });
      const eng = new wasm.Engine(programsText, rulesText, configJson);

      const t0 = performance.now();
      if (item.search === 'smc') {
        eng.run_smc(item.particles, item.steps, item.temperature, item.dead_runs);
      } else {
        eng.step_n(item.budget);
      }
      const elapsed = (performance.now() - t0) / 1000;

      await saveSearchResults(eng, item.domain, item.rules, item.search, elapsed, item.name, item.budget || 0);
    }

    status.textContent = `done — ${items.length} runs saved to ${getSessionFolder()}/`;
    // Reload the results table (load() is a global from analysis.js).
    if (typeof load === 'function') load();
  } catch (e) {
    status.textContent = `error: ${e}`;
    console.error(e);
  } finally {
    btn.disabled = false;
    btn.textContent = 'run';
  }
});
