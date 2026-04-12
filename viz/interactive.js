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
    $('btnLoad').disabled = false;
    await autoLoadFromParams();
  } catch (e) {
    overlay.textContent = `failed to load WASM: ${e}. Run "make wasm" first.`;
    console.error(e);
  }
}

/// If URL has ?domain=...&replay=... params, auto-load the domain and replay.
async function autoLoadFromParams() {
  const params = new URLSearchParams(location.search);
  const domain = params.get('domain');
  const replayFile = params.get('replay');
  if (!domain) return;

  const rules = params.get('rules') || '';
  $('selDomain').value = domain;
  $('selRules').value = rules;

  $('btnLoad').click();

  if (replayFile) {
    await new Promise(resolve => {
      const check = () => engine ? resolve() : setTimeout(check, 50);
      check();
    });

    try {
      const log = await fetch(`results/${replayFile}`).then(r => {
        if (!r.ok) throw new Error(r.status);
        return r.json();
      });
      applyReplayConfig(log.config);
      replaySteps = log.steps || [];
      replayIdx = 0;
    } catch (e) {
      console.error('failed to load replay:', e);
      return;
    }

    const runPath = `results/${replayFile.replace('_replay.json', '.json')}`;
    try {
      const run = await fetch(runPath).then(r => r.ok ? r.json() : null);
      console.log('run result fetch:', runPath, run ? `final_cost=${run.final_cost}` : 'null');
      if (run && run.final_cost != null) replayExpectedCost = run.final_cost;
    } catch (e) { console.warn('failed to fetch run result:', e); }

    const sel = $('selReplay');
    const opt = [...sel.options].find(o => o.value === replayFile);
    if (opt) sel.value = replayFile;

    console.log(`replaying ${replaySteps.length} steps...`);
    while (replayIdx < replaySteps.length) {
      if (!replayOneStep()) break;
    }
    const bestCost = engine.best_cost();
    console.log(`replay done: ${replayIdx} steps, bestCost=${bestCost}, expected=${replayExpectedCost}`);
    renderAll();
    updateReplayButtons();
    if (replayExpectedCost != null && bestCost > replayExpectedCost) {
      statusBar.innerHTML = `<b class="bad">REPLAY MISMATCH: expected cost ${replayExpectedCost.toLocaleString()} but got ${bestCost.toLocaleString()}</b>`;
    } else {
      statusBar.textContent = `replayed ${replayIdx} / ${replaySteps.length} steps`;
    }
  }
}

loadWasm();

// ── Domain / rules ───────────────────────────────────────────────────────────

const DOMAIN_DIR = '/data/domains/cogsci';
const RULES_DIR = '/babble/harness/data/benchmark-dsrs';

$('btnLoad').addEventListener('click', async () => {
  const domain = $('selDomain').value;
  const rulesFile = $('selRules').value;
  const btn = $('btnLoad');
  btn.disabled = true;
  btn.textContent = 'loading…';
  try {
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
    engine = new wasm.Engine(programsText, rulesText);
    // Apply current UI settings.
    engine.set_priority($('selPriority').value);
    engine.set_max_arity(parseInt($('numArity').value) || 2);
    openNodes.clear();
    openNodes.add(0);
    selectedId = 0;
    renderAll();
    enableControls(true);
  } catch (e) {
    alert('failed to load: ' + e);
    console.error(e);
  } finally {
    btn.disabled = false;
    btn.textContent = 'load';
  }
});

$('selDomain').addEventListener('change', () => {
  const domain = $('selDomain').value;
  const rulesSelect = $('selRules');
  const match = `drawings.${domain}.rewrites`;
  const opt = [...rulesSelect.options].find(o => o.value === match);
  rulesSelect.value = opt ? match : '';
});

// Sync settings to Rust when changed.
$('selPriority').addEventListener('change', () => {
  if (engine) engine.set_priority($('selPriority').value);
});
$('numArity').addEventListener('change', () => {
  if (engine) {
    engine.set_max_arity(parseInt($('numArity').value) || 2);
    renderAll();
  }
});

function enableControls(on) {
  $('btnStep').disabled = !on;
  $('btnRun').disabled = !on;
  $('btnExpandBest').disabled = !on;
  $('btnCollapseAll').disabled = !on;
  $('selReplay').disabled = !on;
  if (on) scanReplays();
  updateReplayButtons();
}

// ── Replay log ───────────────────────────────────────────────────────────────

let replaySteps = [];
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
  if (config.priority) $('selPriority').value = config.priority;
  if (config.budget) $('numBudget').value = config.budget;
  if (config.max_arity) $('numArity').value = config.max_arity;
  // Sync to engine.
  if (engine) {
    engine.set_priority(config.priority || 'cost');
    engine.set_max_arity(config.max_arity || 2);
  }
}

$('selReplay').addEventListener('change', async () => {
  const sel = $('selReplay');
  const path = sel.value;
  if (!path) { replaySteps = []; replayIdx = 0; replayExpectedCost = null; updateReplayButtons(); return; }
  const opt = sel.options[sel.selectedIndex];
  replayExpectedCost = opt.dataset.finalCost ? parseInt(opt.dataset.finalCost) : null;
  try {
    const log = await fetch(`results/${path}`).then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    });
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
  const costBefore = engine.best_cost();
  while (replayIdx < replaySteps.length) {
    if (!replayOneStep()) break;
  }
  renderAll();
  updateReplayButtons();

  const bestCost = engine.best_cost();
  if (replayExpectedCost != null && bestCost > replayExpectedCost && bestCost > costBefore) {
    const msg = `replay mismatch: expected best cost ${replayExpectedCost.toLocaleString()} but got ${bestCost.toLocaleString()}`;
    statusBar.innerHTML = `<b class="bad">${msg}</b>`;
    console.warn(msg);
  } else if (replayExpectedCost != null && bestCost >= 0 && bestCost <= replayExpectedCost) {
    statusBar.innerHTML = `replayed ${replayIdx} steps · best cost <b class="good">${bestCost.toLocaleString()}</b> (expected ${replayExpectedCost.toLocaleString()})`;
  }
});

// ── Commands ────────────────────────────────────────────────────────────────

$('btnStep').addEventListener('click', () => {
  const nodeId = engine.step();
  if (nodeId < 0) return;
  openNodes.add(nodeId);
  selectedId = nodeId;
  renderAll();
});

$('btnRun').addEventListener('click', () => {
  const budget = parseInt($('numBudget').value) || 100;
  engine.step_n(budget);
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

function handleRowClick(id, e) {
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

function renderDetail(ctx, nodes) {
  renderSidePane(detail, selectedId, ctx, null);
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
