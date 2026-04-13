// Batch runner for the index/results page.
// Loads WASM on demand and runs preset experiment batches.

import { ALL_DOMAINS, RULES_FOR, fetchDomainData, saveSearchResults, getSessionFolder, resetSessionFolder } from './shared.js';
import { loadWasm, createEngine, runSearch } from './wasm-api.js';

const $ = id => document.getElementById(id);
const status = $('batchStatus');

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

$('btnBatchRun').addEventListener('click', async () => {
  const btn = $('btnBatchRun');
  btn.disabled = true;
  btn.textContent = 'running\u2026';
  resetSessionFolder();

  try {
    status.textContent = 'loading WASM\u2026';
    await loadWasm();
    const items = buildBatches();

    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      status.textContent = `[${i + 1}/${items.length}] ${item.name}\u2026`;

      const { programsText, rulesText } = await fetchDomainData(item.domain, item.rules);
      const configJson = JSON.stringify({
        follow: null, weight_by_usage: false, p_reuse: 0.5,
        max_arity: item.max_arity, priority: item.priority || 'cost',
      });
      const eng = createEngine(programsText, rulesText, configJson);

      const searchParams = item.search === 'smc'
        ? { particles: item.particles, steps: item.steps, temperature: item.temperature, deadRuns: item.dead_runs }
        : { budget: item.budget };
      const { elapsed } = runSearch(eng, item.search, searchParams);

      await saveSearchResults(eng, item.domain, item.rules, item.search, elapsed, item.name, item.budget || 0);
    }

    status.textContent = `done \u2014 ${items.length} runs saved to ${getSessionFolder()}/`;
    // Reload the results table (exposed on window by analysis.js).
    if (typeof window.load === 'function') window.load();
  } catch (e) {
    status.textContent = `error: ${e}`;
    console.error(e);
  } finally {
    btn.disabled = false;
    btn.textContent = 'run';
  }
});
