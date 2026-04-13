// Interactive explorer for egg-stitch WASM module.
// All search state (heap, dedup, best tracking) lives in Rust.
// JS only manages UI state (open/selected nodes) and rendering.

import { buildTreeMeta, buildExpOrder, buildBestPath, renderTree, renderSidePane, wireNavLinks } from './tree-render.js';

const $ = id => document.getElementById(id);
const overlay = $('loading-overlay');
const treepane = $('treepane');
const detail = $('detail');
const heapList = $('heapList');
const heapTitle = $('heapTitle');
const statusBar = $('status-bar');

let wasm = null;
let engine = null;

/// Await this to guarantee the browser paints the current DOM state.
const paint = () => new Promise(r => requestAnimationFrame(() => setTimeout(r, 0)));

// ── UI-only state (not search state) ────────────────────────────────────────

let openNodes = new Set();
let selectedId = null;
let expandedOnly = false;

// ── WASM loading ─────────────────────────────────────────────────────────────

async function loadWasm() {
  try {
    wasm = await import('../pkg/egg_stitch.js');
    await wasm.default();
    overlay.classList.add('hidden');
    $('btnLoadRun').disabled = false;
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

  // Load engine with current config for auto-load.
  try {
    const { programsText, rulesText } = await fetchDomainData();
    engine = new wasm.Engine(programsText, rulesText, buildEngineConfig());
    openNodes.clear();
    openNodes.add(0);
    selectedId = 0;
    enableControls(true);
  } catch (e) {
    console.warn('auto-load failed:', e);
    return;
  }

  if (configFile) {
    // Apply config from replay file without replaying steps.
    try {
      const text = await fetch(`results/${configFile}`).then(r => {
        if (!r.ok) throw new Error(r.status);
        return r.text();
      });
      const log = JSON.parse(text);
      applyReplayConfig(log.config);
      renderAll();
      statusBar.textContent = `loaded config: priority=${log.config?.priority}, max_arity=${log.config?.max_arity}`;
    } catch (e) { console.warn('failed to load config:', e); }
    return;
  }

  if (replayFile) {
    // Fetch expected cost from the run result.
    const runPath = `results/${replayFile.replace('_replay.json', '.json')}`;
    try {
      const run = await fetch(runPath).then(r => r.ok ? r.json() : null);
      if (run && run.final_cost != null) replayExpectedCost = run.final_cost;
    } catch (e) { console.warn('failed to fetch run result:', e); }

    const sel = $('selReplay');
    const opt = [...sel.options].find(o => o.value === replayFile);
    if (opt) sel.value = replayFile;

    await runReplayFromUrl(`results/${replayFile}`);
  }
}

loadWasm();

// ── Domain / rules ───────────────────────────────────────────────────────────

const DOMAIN_DIR = '/data/domains/cogsci';
const RULES_DIR = '/babble/harness/data/benchmark-dsrs';

/// Fetch programs + rules texts for the currently selected domain.
async function fetchDomainData() {
  const domain = $('selDomain').value;
  const rulesFile = $('selRules').value;
  const programsText = await fetch(`${DOMAIN_DIR}/${domain}.json`).then(r => {
    if (!r.ok) throw new Error(`${r.status} loading ${domain}.json`);
    return r.text();
  });
  let rulesText = undefined;
  if (rulesFile) {
    rulesText = await fetch(`${RULES_DIR}/${rulesFile}`).then(r => {
      if (!r.ok) throw new Error(`${r.status} loading ${rulesFile}`);
      return r.text();
    });
  }
  return { programsText, rulesText };
}

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

$('selPreset').addEventListener('change', () => {
  const p = PRESETS[$('selPreset').value];
  if (!p) return;
  $('selSearch').value = p.search;
  $('selSearch').dispatchEvent(new Event('change'));
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

/// Load & Run: create a fresh engine with config, run the selected search, show results.
$('btnLoadRun').addEventListener('click', async () => {
  const btn = $('btnLoadRun');
  btn.disabled = true;
  btn.textContent = 'running…';
  const resultsBar = $('results-bar');
  resultsBar.className = '';
  resultsBar.innerHTML = '';
  statusBar.textContent = 'loading domain…';
  await paint();

  try {
    const { programsText, rulesText } = await fetchDomainData();
    statusBar.textContent = 'initializing engine…';
    await paint();

    engine = new wasm.Engine(programsText, rulesText, buildEngineConfig());
    openNodes.clear();
    openNodes.add(0);
    selectedId = 0;

    const searchType = $('selSearch').value;
    statusBar.textContent = `running ${searchType}…`;
    await paint();

    const t0 = performance.now();
    if (searchType === 'smc') {
      const particles = parseInt($('cfgParticles').value) || 1000;
      const steps = parseInt($('cfgSteps').value) || 100;
      const temp = parseFloat($('cfgTemp').value) || 100;
      const deadRuns = parseInt($('cfgDeadRuns').value) || 50;
      engine.run_smc(particles, steps, temp, deadRuns);
    } else {
      const budget = parseInt($('cfgBudget').value) || 500;
      engine.step_n(budget);
    }
    const elapsed = ((performance.now() - t0) / 1000).toFixed(2);

    // Show results.
    const results = engine.results_json();
    showResults(results, elapsed, searchType);
    enableControls(true);

    statusBar.textContent = `rendering…`;
    await paint();
    renderAll();
    statusBar.textContent = `${searchType} complete in ${elapsed}s`;
  } catch (e) {
    alert('search failed: ' + e);
    console.error(e);
    statusBar.innerHTML = `<b class="bad">error: ${e}</b>`;
  } finally {
    btn.disabled = false;
    btn.textContent = 'load & run';
  }
});

/// Display search results in the results bar.
function showResults(r, elapsed, searchType) {
  const bar = $('results-bar');
  const fmt = v => v != null ? Number(v).toLocaleString() : '—';
  if (r.best_cost == null) {
    bar.className = 'visible';
    bar.style.background = '#fef3c7';
    bar.style.borderColor = '#fcd34d';
    bar.innerHTML = `<span class="result-label">no solution found</span>
      <span class="result-label">expansions:</span><span class="result-value">${fmt(r.num_expansions)}</span>
      <span class="result-label">time:</span><span class="result-value">${elapsed}s</span>`;
    return;
  }
  bar.className = 'visible';
  bar.style.background = '';
  bar.style.borderColor = '';
  const ratio = r.compression_ratio != null ? r.compression_ratio.toFixed(2) + 'x' : '—';
  bar.innerHTML = `
    <span class="result-label">${searchType}</span>
    <span class="result-label">cost:</span><span class="result-value" style="color:var(--good)">${fmt(r.best_cost)}</span>
    <span class="result-label">ratio:</span><span class="result-value" style="color:var(--good)">${ratio}</span>
    <span class="result-label">arity:</span><span class="result-value">${r.arity ?? '—'}</span>
    <span class="result-label">matches:</span><span class="result-value">${fmt(r.num_matches)}</span>
    <span class="result-label">expansions:</span><span class="result-value">${fmt(r.num_expansions)}</span>
    <span class="result-label">nodes:</span><span class="result-value">${fmt(r.num_nodes)}</span>
    <span class="result-label">time:</span><span class="result-value">${elapsed}s</span>
    <span class="result-pattern" title="${esc(r.pattern || '')}">${esc(r.pattern || '')}</span>
  `;
}

function enableControls(on) {
  $('btnStep').disabled = !on;
  $('btnExpandBest').disabled = !on;
  $('btnCollapseAll').disabled = !on;
  $('selReplay').disabled = !on;
  if (on) scanReplays();
  updateReplayButtons();
}

// ── Replay log ───────────────────────────────────────────────────────────────

let replayJsonText = null;  // raw JSON string, sent to Rust for bulk replay
let replaySteps = [];       // parsed steps, used for single-step replay
let replayIdx = 0;
let replayExpectedCost = null;

async function scanReplays() {
  const sel = $('selReplay');
  const domain = $('selDomain').value;
  sel.innerHTML = '<option value="">none</option>';
  try {
    const listing = await fetch('results/').then(r => r.text());
    const { files: topFiles, dirs } = parseDirectoryListing(listing);

    const candidates = [];

    for (const f of topFiles) {
      if (f.endsWith('_debug.json') || f.endsWith('_replay.json')) continue;
      try {
        const r = await fetch(`results/${f}`).then(r => r.json());
        if (r.replay_log_file && r.input_file && r.input_file.includes(`/${domain}.json`)) {
          candidates.push({ label: f.replace(/\.json$/, ''), path: r.replay_log_file, finalCost: r.final_cost });
        }
      } catch { /* skip bad files */ }
    }

    for (const d of dirs) {
      try {
        const sub = await fetch(`results/${d}/`).then(r => r.text());
        const { files } = parseDirectoryListing(sub);
        for (const f of files) {
          if (f.endsWith('_debug.json') || f.endsWith('_replay.json')) continue;
          try {
            const r = await fetch(`results/${d}/${f}`).then(r => r.json());
            if (r.replay_log_file && r.input_file && r.input_file.includes(`/${domain}.json`)) {
              candidates.push({ label: `${d}/${f.replace(/\.json$/, '')}`, path: `${d}/${r.replay_log_file}`, finalCost: r.final_cost });
            }
          } catch { /* skip */ }
        }
      } catch { /* skip */ }
    }

    for (const c of candidates) {
      const opt = document.createElement('option');
      opt.value = c.path;
      opt.dataset.finalCost = c.finalCost ?? '';
      opt.textContent = c.label + (c.finalCost != null ? ` (cost ${c.finalCost})` : '');
      sel.appendChild(opt);
    }
  } catch (e) {
    console.warn('could not scan replays:', e);
  }
}

function parseDirectoryListing(html) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const files = [], dirs = [];
  for (const a of doc.querySelectorAll('a')) {
    const h = a.getAttribute('href');
    if (!h || h.startsWith('?') || h === '../' || h === '/') continue;
    if (h.endsWith('/')) dirs.push(h.replace(/\/$/, ''));
    else if (h.endsWith('.json')) files.push(h);
  }
  return { files, dirs };
}

function applyReplayConfig(config) {
  if (!config) return;
  if (config.priority) $('cfgPriority').value = config.priority;
  if (config.budget) $('cfgBudget').value = config.budget;
  if (config.max_arity) $('cfgArity').value = config.max_arity;
  // Sync to engine.
  if (engine) {
    engine.set_priority(config.priority || 'cost');
    engine.set_max_arity(config.max_arity || 2);
  }
}

$('selReplay').addEventListener('change', async () => {
  const sel = $('selReplay');
  const path = sel.value;
  if (!path) { replayJsonText = null; replaySteps = []; replayIdx = 0; replayExpectedCost = null; updateReplayButtons(); return; }
  const opt = sel.options[sel.selectedIndex];
  replayExpectedCost = opt.dataset.finalCost ? parseInt(opt.dataset.finalCost) : null;
  try {
    replayJsonText = await fetch(`results/${path}`).then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.text();
    });
    const log = JSON.parse(replayJsonText);
    applyReplayConfig(log.config);
    replaySteps = log.steps || [];
    replayIdx = 0;
    updateReplayButtons();
    statusBar.textContent = `loaded replay: ${replaySteps.length} steps` + (replayExpectedCost != null ? ` (expected cost: ${replayExpectedCost})` : '');
  } catch (err) {
    alert('failed to load replay: ' + err);
  }
});

function updateReplayButtons() {
  const hasSteps = replaySteps.length > 0 && replayIdx < replaySteps.length && engine;
  $('btnReplay').disabled = !hasSteps;
  $('btnReplayAll').disabled = !hasSteps;
}

/// Replay one step using Rust engine. Returns true if a node was expanded.
function replayOneStep() {
  if (replayIdx >= replaySteps.length) return false;
  const step = replaySteps[replayIdx];
  replayIdx++;

  const nodeId = engine.find_unexpanded_by_pattern(step.pattern);
  if (nodeId < 0) {
    if (engine.has_pattern(step.pattern)) return true; // already expanded, skip
    const msg = `replay error at step ${replayIdx}: pattern not found in tree: ${step.pattern}`;
    statusBar.innerHTML = `<b class="bad">${msg}</b>`;
    console.error(msg);
    return false;
  }

  // Validate matches and cost against expected values.
  const info = engine.node_info_json(nodeId);
  const matchesOk = step.num_matches == null || info.num_matches === step.num_matches;
  const costOk = step.cost == null || info.cost === step.cost;
  if (!matchesOk || !costOk) {
    const parts = [`replay mismatch at step ${replayIdx}: ${step.pattern}`];
    parts.push(`  matches: ${info.num_matches} (expected ${step.num_matches})`);
    parts.push(`  cost: ${info.cost} (expected ${step.cost})`);
    parts.push(`  pattern_size: ${info.pattern_size}`);
    const msg = parts.join('\n');
    statusBar.innerHTML = `<b class="bad">${parts.join('<br>')}</b>`;
    console.error(msg);
    return false;
  }

  engine.expand_node(nodeId);
  openNodes.add(nodeId);
  return true;
}

$('btnReplay').addEventListener('click', () => {
  if (replayOneStep()) {
    selectedId = engine.expansion_order_json().at(-1) ?? selectedId;
    renderAll();
    updateReplayButtons();
  }
});

$('btnReplayAll').addEventListener('click', () => {
  if (replayJsonText) runReplayFromJson(replayJsonText);
});

/// Fetch a replay log by URL and run it entirely in Rust.
async function runReplayFromUrl(url) {
  const text = await fetch(url).then(r => {
    if (!r.ok) throw new Error(`${r.status} loading ${url}`);
    return r.text();
  });
  await runReplayFromJson(text);
}

/// Run a replay from raw JSON text. Parsing + execution happen in Rust.
async function runReplayFromJson(json) {
  $('btnReplayAll').disabled = true;
  $('btnReplay').disabled = true;
  const stepCount = replaySteps.length || '?';
  statusBar.textContent = `replaying ${stepCount} steps…`;
  await paint();

  const t0 = performance.now();
  let error = null;
  try {
    const config = engine.replay_from_json(json);
    // Sync config panel with the config Rust applied.
    if (config.priority) $('cfgPriority').value = config.priority;
    if (config.max_arity) $('cfgArity').value = config.max_arity;
    replayIdx = replaySteps.length;
  } catch (e) {
    error = e.message;
  }
  const replayMs = (performance.now() - t0).toFixed(0);
  const nExpanded = engine.num_expansions();

  const bestCost = engine.best_cost();
  if (error) {
    statusBar.innerHTML = `<b class="bad">${error}</b> (${replayMs}ms)`;
  } else {
    const costStr = bestCost >= 0 ? bestCost.toLocaleString() : '—';
    statusBar.textContent = `replayed ${nExpanded} steps in ${replayMs}ms · best cost ${costStr} · rendering…`;
  }
  await paint();

  const t1 = performance.now();
  renderAll();
  updateReplayButtons();
  const renderMs = (performance.now() - t1).toFixed(0);
  if (!error) {
    const costStr = bestCost >= 0 ? bestCost.toLocaleString() : '—';
    statusBar.innerHTML = `replayed ${nExpanded} steps in <b>${replayMs}ms</b> · render <b>${renderMs}ms</b> · best cost <b>${costStr}</b>` + (replayExpectedCost != null ? ` (expected ${replayExpectedCost.toLocaleString()})` : '');
  }
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

$('btnExpandBest').addEventListener('click', () => {
  const bestId = engine.best_node_id();
  if (bestId < 0) return;
  const nodes = engine.nodes_json();
  let cur = bestId;
  while (cur != null) { openNodes.add(cur); cur = nodes[cur]?.parent; }
  selectedId = bestId;
  renderAll();
  const el = treepane.querySelector(`.row[data-id="${bestId}"]`);
  if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
});

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
  // Fetch all state from Rust.
  const nodes = engine.nodes_json();
  const expansionOrder = engine.expansion_order_json();
  const bestNodeId = engine.best_node_id();
  const bestNode = bestNodeId >= 0 ? bestNodeId : null;
  const originalSize = engine.original_size();

  const meta = buildTreeMeta(nodes);
  const expOrder = buildExpOrder(expansionOrder, nodes.length);
  const bestPath = buildBestPath(nodes, bestNode);

  const ctx = {
    nodes,
    meta,
    bestNode,
    bestPath,
    expOrder,
    originalSize,
    openNodes,
    selectedId,
    expandedOnly,
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
    if (openNodes.has(id)) openNodes.delete(id);
    else openNodes.add(id);
  }
  selectedId = id;
  renderAll();
}

function buildCostSparkline(nodes, id) {
  // Walk parent chain from selected node to root.
  const chain = [];
  let cur = id;
  while (cur != null) { chain.push(nodes[cur]); cur = nodes[cur].parent; }
  chain.reverse(); // root first
  if (chain.length < 2) return '';

  const costs = chain.map(n => n.cost);
  const minC = Math.min(...costs), maxC = Math.max(...costs);
  const range = maxC - minC || 1;
  const W = 240, H = 48, px = 6, py = 6;
  const iW = W - 2 * px, iH = H - 2 * py;
  const x = (i) => px + (i / (chain.length - 1)) * iW;
  const y = (c) => py + ((c - minC) / range) * iH; // lower cost → higher y (bottom)

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
      <span class="pattern">${esc(n.pattern)}</span>
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
    li.innerHTML = `<span class="rank">…</span><span class="pattern" style="color:var(--muted)">${heapTotal - 200} more</span>`;
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
  const ratio = hasBest ? (originalSize / bestCost).toFixed(3) : '—';
  statusBar.innerHTML = `<b>${nNodes}</b> nodes · <b>${nExp}</b> expanded · <b>${heapSz}</b> in heap · best cost: <b>${hasBest ? bestCost.toLocaleString() : '—'}</b> · ratio: <b>${ratio}×</b> · original: <b>${originalSize.toLocaleString()}</b>`;
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
