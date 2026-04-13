// Interactive explorer for egg-stitch WASM module.
// All search state (heap, dedup, best tracking) lives in Rust.
// JS only manages UI state (open/selected nodes) and rendering.

import { buildTreeMeta, buildExpOrder, buildBestPath, renderTree, renderSidePane, wireNavLinks, escapeHtml } from './tree-render.js';
import { fetchDomainData, saveSearchResults, getSessionFolder, paint } from './shared.js';
import { loadWasm, createEngine, runSearch } from './wasm-api.js';
import {
  scanReplays, applyReplayConfig, updateReplayButtons, replayOneStep,
  runReplayFromJson, runReplayFromUrl, wireReplaySelect, getReplayJsonText,
  getReplayExpectedCost, setReplayExpectedCost,
} from './replay.js';

const $ = id => document.getElementById(id);
const overlay = $('loading-overlay');
const treepane = $('treepane');
const detail = $('detail');
const heapList = $('heapList');
const heapTitle = $('heapTitle');
const statusBar = $('status-bar');

let engine = null;

// ── UI-only state (not search state) ────────────────────────────────────────

let openNodes = new Set();
let selectedId = null;
let expandedOnly = false;

// Track engine-level config that requires a full reload to change
// (baked into SharedSearchData at construction time).
let lastLoadedEngineKey = null;
function engineKey() {
  return JSON.stringify({
    domain: $('selDomain').value,
    rules: $('selRules').value,
    follow: $('cfgFollow').value.trim() || null,
    weight_by_usage: $('cfgWeightByUsage').checked,
    p_reuse: parseFloat($('cfgPReuse').value) || 0.5,
  });
}
function trackLoadedConfig() { lastLoadedEngineKey = engineKey(); }

// ── WASM loading ─────────────────────────────────────────────────────────────

async function initWasm() {
  try {
    await loadWasm();
    overlay.classList.add('hidden');
    $('btnLoad').disabled = false;
    $('btnRun').disabled = false;
    await autoLoadFromParams();
  } catch (e) {
    overlay.textContent = `failed to load WASM: ${e}. Run "make wasm" first.`;
    console.error(e);
  }
}

/// If URL has ?domain=...&replay=... or ?domain=...&config=... params, auto-load.
async function autoLoadFromParams() {
  const params = new URLSearchParams(location.search);
  const domain = params.get('domain');
  const replayFile = params.get('replay');
  const configFile = params.get('config');
  if (!domain) return;

  const rules = params.get('rules') || '';
  $('selDomain').value = domain;
  $('selRules').value = rules;

  try {
    const { programsText, rulesText } = await fetchDomainData($('selDomain').value, $('selRules').value);
    engine = createEngine(programsText, rulesText, buildEngineConfig());
    trackLoadedConfig();
    openNodes.clear();
    openNodes.add(0);
    selectedId = 0;
    enableControls(true);
  } catch (e) {
    console.warn('auto-load failed:', e);
    return;
  }

  if (configFile) {
    try {
      const text = await fetch(`results/${configFile}`).then(r => {
        if (!r.ok) throw new Error(r.status);
        return r.text();
      });
      const log = JSON.parse(text);
      applyReplayConfig(log.config, engine);
      renderAll();
      statusBar.textContent = `loaded config: priority=${log.config?.priority}, max_arity=${log.config?.max_arity}`;
    } catch (e) { console.warn('failed to load config:', e); }
  } else if (replayFile) {
    const runPath = `results/${replayFile.replace('_replay.json', '.json')}`;
    try {
      const run = await fetch(runPath).then(r => r.ok ? r.json() : null);
      if (run && run.final_cost != null) setReplayExpectedCost(run.final_cost);
    } catch (e) { console.warn('failed to fetch run result:', e); }

    const sel = $('selReplay');
    const opt = [...sel.options].find(o => o.value === replayFile);
    if (opt) sel.value = replayFile;

    await doRunReplayFromUrl(`results/${replayFile}`);
  }
}

initWasm();

// ── Domain / rules ───────────────────────────────────────────────────────────

/// Build config JSON from the config panel for Engine constructor.
function buildEngineConfig() {
  const follow = $('cfgFollow').value.trim() || undefined;
  return JSON.stringify({
    follow: follow || null,
    weight_by_usage: $('cfgWeightByUsage').checked,
    p_reuse: parseFloat($('cfgPReuse').value) || 0.5,
    max_arity: parseInt($('cfgArity').value) || 2,
    priority: $('selSearch').value === 'best-first' ? ($('cfgPriority').value || 'cost') : 'cost',
  });
}

$('selDomain').addEventListener('change', () => {
  const domain = $('selDomain').value;
  const rulesSelect = $('selRules');
  const match = `drawings.${domain}.rewrites`;
  const opt = [...rulesSelect.options].find(o => o.value === match);
  rulesSelect.value = opt ? match : '';
});

// ── Config panel ────────────────────────────────────────────────────────────

const PRESETS = {
  'dev':            { search: 'smc', particles: 1000, steps: 100, temperature: 1000, max_arity: 2, dead_runs: 50 },
  'dials-compress': { search: 'smc', particles: 100, steps: 10, temperature: 100, max_arity: 2, dead_runs: 50 },
  'dials-follow':   { search: 'smc', particles: 100, steps: 10, temperature: 100, max_arity: 2, dead_runs: 50,
                      follow: '(T (T (T l (M 1 0 -0.5 0)) (M #0 (/ pi 4) 0 0)) (M 1 0 (* #0 (* 0.5 (cos (/ pi 4)))) (* #0 (* 0.5 (sin (/ pi 4))))))' },
  'best-first':     { search: 'best-first', priority: 'cost', budget: 500, max_arity: 2 },
  'bf-dfs':         { search: 'best-first', priority: 'depth-first', budget: 500, max_arity: 2 },
  'bf-bfs':         { search: 'best-first', priority: 'breadth-first', budget: 500, max_arity: 2 },
  'bf-matches':     { search: 'best-first', priority: 'most-matches', budget: 500, max_arity: 2 },
};

$('configToggle').addEventListener('click', () => {
  const body = $('configBody');
  const toggle = $('configToggle');
  const open = body.classList.toggle('open');
  toggle.innerHTML = (open ? '&#9652;' : '&#9662;') + ' search config';
});

$('selSearch').addEventListener('change', () => {
  const isSmc = $('selSearch').value === 'smc';
  $('rowSmc').style.display = isSmc ? '' : 'none';
  $('rowBf').style.display = isSmc ? 'none' : '';
});

$('cfgPriority').addEventListener('change', () => {
  if (engine) {
    engine.set_priority($('cfgPriority').value);
    renderAll();
  }
});

$('selPreset').addEventListener('change', () => {
  const p = PRESETS[$('selPreset').value];
  if (!p) return;
  if (p.search) { $('selSearch').value = p.search; $('selSearch').dispatchEvent(new Event('change')); }
  if (p.particles != null) $('cfgParticles').value = p.particles;
  if (p.steps != null) $('cfgSteps').value = p.steps;
  if (p.temperature != null) $('cfgTemp').value = p.temperature;
  if (p.dead_runs != null) $('cfgDeadRuns').value = p.dead_runs;
  if (p.budget != null) $('cfgBudget').value = p.budget;
  if (p.priority != null) $('cfgPriority').value = p.priority;
  if (p.max_arity != null) $('cfgArity').value = p.max_arity;
  $('cfgFollow').value = p.follow || '';
  $('cfgPReuse').value = p.p_reuse ?? 0.5;
  $('cfgWeightByUsage').checked = p.weight_by_usage ?? false;
});

// ── Load / Run ──────────────────────────────────────────────────────────────

/// Load: create a fresh engine with config, no search.
$('btnLoad').addEventListener('click', async () => {
  const btn = $('btnLoad');
  btn.disabled = true;
  btn.textContent = 'loading\u2026';
  statusBar.textContent = 'loading domain\u2026';
  await paint();

  try {
    const { programsText, rulesText } = await fetchDomainData($('selDomain').value, $('selRules').value);
    engine = createEngine(programsText, rulesText, buildEngineConfig());
    trackLoadedConfig();
    openNodes.clear();
    openNodes.add(0);
    selectedId = 0;
    enableControls(true);
    renderAll();
  } catch (e) {
    alert('load failed: ' + e);
    console.error(e);
    statusBar.innerHTML = `<b class="bad">error: ${e}</b>`;
  } finally {
    btn.disabled = false;
    btn.textContent = 'load';
  }
});

/// Run: run search on the current engine (or load first if none).
$('btnRun').addEventListener('click', async () => {
  const btn = $('btnRun');
  btn.disabled = true;
  btn.textContent = 'running\u2026';
  const resultsBar = $('results-bar');
  resultsBar.className = '';
  resultsBar.innerHTML = '';

  try {
    // (Re)load engine if not yet loaded or if engine-level config changed.
    if (!engine || engineKey() !== lastLoadedEngineKey) {
      statusBar.textContent = 'loading domain\u2026';
      await paint();
      const { programsText, rulesText } = await fetchDomainData($('selDomain').value, $('selRules').value);
      engine = createEngine(programsText, rulesText, buildEngineConfig());
      openNodes.clear();
      openNodes.add(0);
      selectedId = 0;
      trackLoadedConfig();
    }

    // Sync live-changeable settings to the engine.
    engine.set_priority($('cfgPriority').value || 'cost');
    engine.set_max_arity(parseInt($('cfgArity').value) || 2);

    const searchType = $('selSearch').value;
    statusBar.textContent = `running ${searchType}\u2026`;
    await paint();

    const searchParams = searchType === 'smc'
      ? { particles: parseInt($('cfgParticles').value) || 1000, steps: parseInt($('cfgSteps').value) || 100, temperature: parseFloat($('cfgTemp').value) || 100, deadRuns: parseInt($('cfgDeadRuns').value) || 50 }
      : { budget: parseInt($('cfgBudget').value) || 500 };
    const { results, elapsed } = runSearch(engine, searchType, searchParams);

    // Save results.
    const domain = $('selDomain').value;
    const rulesFile = $('selRules').value;
    statusBar.textContent = 'saving\u2026';
    await paint();
    const outputName = `${domain}_${searchType.replace('-', '_')}`;
    const budget = searchType === 'best-first' ? (searchParams.budget || 0) : 0;
    const saved = await saveSearchResults(engine, domain, rulesFile, searchType, elapsed, outputName, budget);

    showResults(results, elapsed.toFixed(2), searchType, saved.folder);
    enableControls(true);

    statusBar.textContent = `rendering\u2026`;
    await paint();
    renderAll();
    showBest();
    statusBar.innerHTML += ` \u00b7 ${elapsed.toFixed(2)}s \u00b7 saved to ${saved.folder}/`;
  } catch (e) {
    alert('search failed: ' + e);
    console.error(e);
    statusBar.innerHTML = `<b class="bad">error: ${e}</b>`;
  } finally {
    btn.disabled = false;
    btn.textContent = 'run';
  }
});

/// Display search results in the results bar.
function showResults(r, elapsed, searchType, folder) {
  const bar = $('results-bar');
  const fmt = v => v != null ? Number(v).toLocaleString() : '\u2014';
  const savedTag = folder ? `<span class="result-label" style="color:var(--accent)">saved: ${folder}/</span>` : '';
  if (r.best_cost == null) {
    bar.className = 'visible';
    bar.style.background = '#fef3c7';
    bar.style.borderColor = '#fcd34d';
    bar.innerHTML = `<span class="result-label">no solution found</span>
      <span class="result-label">expansions:</span><span class="result-value">${fmt(r.num_expansions)}</span>
      <span class="result-label">time:</span><span class="result-value">${elapsed}s</span>${savedTag}`;
    return;
  }
  bar.className = 'visible';
  bar.style.background = '';
  bar.style.borderColor = '';
  const ratio = r.compression_ratio != null ? r.compression_ratio.toFixed(2) + 'x' : '\u2014';
  bar.innerHTML = `
    <span class="result-label">${searchType}</span>
    <span class="result-label">cost:</span><span class="result-value" style="color:var(--good)">${fmt(r.best_cost)}</span>
    <span class="result-label">ratio:</span><span class="result-value" style="color:var(--good)">${ratio}</span>
    <span class="result-label">arity:</span><span class="result-value">${r.arity ?? '\u2014'}</span>
    <span class="result-label">matches:</span><span class="result-value">${fmt(r.num_matches)}</span>
    <span class="result-label">expansions:</span><span class="result-value">${fmt(r.num_expansions)}</span>
    <span class="result-label">nodes:</span><span class="result-value">${fmt(r.num_nodes)}</span>
    <span class="result-label">time:</span><span class="result-value">${elapsed}s</span>
    ${savedTag}
    <span class="result-pattern" title="${escapeHtml(r.pattern || '')}">${escapeHtml(r.pattern || '')}</span>
  `;
}

function enableControls(on) {
  $('btnStep').disabled = !on;
  $('btnExpandBest').disabled = !on;
  $('btnCollapseAll').disabled = !on;
  $('selReplay').disabled = !on;
  if (on) scanReplays(engine);
  updateReplayButtons(engine);
}

// ── Replay event handlers ───────────────────────────────────────────────────

wireReplaySelect(engine, statusBar);

$('btnReplay').addEventListener('click', () => {
  if (replayOneStep(engine, openNodes, statusBar)) {
    selectedId = engine.expansion_order_json().at(-1) ?? selectedId;
    renderAll();
    updateReplayButtons(engine);
  }
});

$('btnReplayAll').addEventListener('click', () => {
  const json = getReplayJsonText();
  if (json) doRunReplayFromJson(json);
});

/// Run a full replay from JSON and update the UI.
async function doRunReplayFromJson(json) {
  const formatCost = (cost) => cost >= 0 ? cost.toLocaleString() : '\u2014';

  $('btnReplayAll').disabled = true;
  $('btnReplay').disabled = true;

  const { error, replayMs, nExpanded, bestCost } = await runReplayFromJson(engine, json, statusBar);

  if (error) {
    statusBar.innerHTML = `<b class="bad">${error}</b> (${replayMs}ms)`;
  } else {
    const costStr = formatCost(bestCost);
    statusBar.textContent = `replayed ${nExpanded} steps in ${replayMs}ms \u00b7 best cost ${costStr} \u00b7 rendering\u2026`;
  }
  await paint();

  const t1 = performance.now();
  renderAll();
  updateReplayButtons(engine);
  const renderMs = (performance.now() - t1).toFixed(0);
  if (!error) {
    const costStr = formatCost(bestCost);
    const expected = getReplayExpectedCost();
    statusBar.innerHTML = `replayed ${nExpanded} steps in <b>${replayMs}ms</b> \u00b7 render <b>${renderMs}ms</b> \u00b7 best cost <b>${costStr}</b>` + (expected != null ? ` (expected ${expected.toLocaleString()})` : '');
  }
}

/// Run a full replay from a URL.
async function doRunReplayFromUrl(url) {
  const text = await fetch(url).then(r => {
    if (!r.ok) throw new Error(`${r.status} loading ${url}`);
    return r.text();
  });
  await doRunReplayFromJson(text);
}

// ── Commands ────────────────────────────────────────────────────────────────

$('btnStep').addEventListener('click', () => {
  const nodeId = engine.step();
  if (nodeId < 0) return;
  openNodes.add(nodeId);
  selectedId = nodeId;
  renderAll();
});

// ── UI controls ──────────────────────────────────────────────────────────────

/// Expand the tree path to the best node and scroll to it.
function showBest() {
  if (!engine) return;
  const bestId = engine.best_node_id();
  if (bestId < 0) return;
  const nodes = engine.nodes_json();
  let cur = bestId;
  while (cur != null) { openNodes.add(cur); cur = nodes[cur]?.parent; }
  selectedId = bestId;
  renderAll();
  const el = treepane.querySelector(`.row[data-id="${bestId}"]`);
  if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
}

$('btnExpandBest').addEventListener('click', showBest);

$('btnCollapseAll').addEventListener('click', () => {
  openNodes.clear();
  openNodes.add(0);
  renderAll();
});

$('chkExpandedOnly').addEventListener('change', e => {
  expandedOnly = e.target.checked;
  renderAll();
});

// ── Rendering ────────────────────────────────────────────────────────────────

function renderAll() {
  const nodes = engine.nodes_json();
  const expansionOrder = engine.expansion_order_json();
  const bestNodeId = engine.best_node_id();
  const bestNode = bestNodeId >= 0 ? bestNodeId : null;
  const originalSize = engine.original_size();

  const meta = buildTreeMeta(nodes);
  const expOrder = buildExpOrder(expansionOrder, nodes.length);
  const bestPath = buildBestPath(nodes, bestNode);

  const ctx = {
    nodes, meta, bestNode, bestPath, expOrder, originalSize,
    openNodes, selectedId, expandedOnly,
    onRowClick: handleRowClick,
  };

  renderTree(treepane, ctx);
  renderDetail(ctx, nodes);
  renderHeap(nodes);
  updateStatus();
}

function handleRowClick(id) {
  const nodes = engine.nodes_json();
  const node = nodes[id];
  if (!node.expanded) {
    engine.expand_node(id);
    openNodes.add(id);
  } else {
    openNodes.has(id) ? openNodes.delete(id) : openNodes.add(id);
  }
  selectedId = id;
  renderAll();
}

function buildCostSparkline(nodes, id) {
  const chain = [];
  let cur = id;
  while (cur != null) { chain.push(nodes[cur]); cur = nodes[cur].parent; }
  chain.reverse();
  if (chain.length < 2) return '';

  const costs = chain.map(n => n.cost);
  const minC = Math.min(...costs), maxC = Math.max(...costs);
  const range = maxC - minC || 1;
  const W = 240, H = 48, px = 6, py = 6;
  const iW = W - 2 * px, iH = H - 2 * py;
  const x = (i) => px + (i / (chain.length - 1)) * iW;
  const y = (c) => py + ((c - minC) / range) * iH;

  const pts = chain.map((n, i) => `${x(i).toFixed(1)},${y(n.cost).toFixed(1)}`).join(' ');
  const lx = x(chain.length - 1).toFixed(1);
  const ly = y(costs.at(-1)).toFixed(1);

  return `
    <h2>cost along path</h2>
    <svg width="${W}" height="${H}" style="display:block;overflow:visible;margin:.25rem 0">
      <polyline points="${pts}" fill="none" stroke="var(--accent)" stroke-width="1.5" stroke-linejoin="round"/>
      <circle cx="${lx}" cy="${ly}" r="3" fill="var(--accent)"/>
      <text x="${px}" y="${H - 1}" font-size="9" fill="var(--muted)">${minC.toLocaleString()}</text>
      <text x="${px}" y="${py + 8}" font-size="9" fill="var(--muted)">${maxC.toLocaleString()}</text>
      <text x="${lx}" y="${+ly - 5}" font-size="9" fill="var(--fg)" text-anchor="middle">${costs.at(-1).toLocaleString()}</text>
    </svg>`;
}

function renderDetail(ctx, nodes) {
  const sparkline = selectedId != null ? buildCostSparkline(nodes, selectedId) : '';
  renderSidePane(detail, selectedId, ctx, sparkline);
  wireNavLinks(detail, nodes, openNodes, id => {
    selectedId = id;
    renderAll();
    const el = treepane.querySelector(`.row[data-id="${id}"]`);
    if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
  });
}

function renderHeap(nodes) {
  const heapEntries = engine.heap_top_json(200);
  const heapTotal = engine.heap_size();
  heapTitle.textContent = `heap (${heapTotal})`;
  heapList.innerHTML = '';
  for (let i = 0; i < heapEntries.length; i++) {
    const h = heapEntries[i];
    const n = nodes[h.node_id];
    const li = document.createElement('li');
    if (h.node_id === selectedId) li.classList.add('selected');
    li.innerHTML = `
      <span class="rank">${i}</span>
      <span class="prio">${h.priority}</span>
      <span class="cost">${n.cost.toLocaleString()}</span>
      <span class="stats">m${n.num_matches} a${n.arity}</span>
      <span class="pattern">${escapeHtml(n.pattern)}</span>
    `;
    li.addEventListener('click', () => {
      selectedId = h.node_id;
      let cur = h.node_id;
      while (cur != null) { openNodes.add(nodes[cur]?.parent ?? 0); cur = nodes[cur]?.parent; }
      renderAll();
      const el = treepane.querySelector(`.row[data-id="${h.node_id}"]`);
      if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
    });
    heapList.appendChild(li);
  }
  if (heapTotal > 200) {
    const li = document.createElement('li');
    li.innerHTML = `<span class="rank">\u2026</span><span class="pattern" style="color:var(--muted)">${heapTotal - 200} more</span>`;
    heapList.appendChild(li);
  }
}

function updateStatus() {
  const nNodes = engine.num_nodes();
  const nExp = engine.num_expansions();
  const heapSz = engine.heap_size();
  const bestCost = engine.best_cost();
  const originalSize = engine.original_size();
  const hasBest = bestCost >= 0;
  const ratio = hasBest ? (originalSize / bestCost).toFixed(3) : '\u2014';
  statusBar.innerHTML = `<b>${nNodes}</b> nodes \u00b7 <b>${nExp}</b> expanded \u00b7 <b>${heapSz}</b> in heap \u00b7 best cost: <b>${hasBest ? bestCost.toLocaleString() : '\u2014'}</b> \u00b7 ratio: <b>${ratio}\u00d7</b> \u00b7 original: <b>${originalSize.toLocaleString()}</b>`;
}
