// Interactive explorer for egg-stitch WASM module.
// Manages a JS-side tree + priority queue. Manual clicks and automated search
// both use the same expandNode() path, so the tree builds up identically.

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
let originalSize = 0;

// ── Tree data model ──────────────────────────────────────────────────────────
// Mirrors the SearchTreeLog node format used by tree.html/tree-render.js.

let nodes = [];          // flat array, same shape as tree log nodes
let stateIdMap = [];     // nodeId -> wasm state_id
let heap = [];           // [{nodeId, priority}] sorted by priority ascending
let seen = new Set();    // state patterns we've already added (dedup)
let bestNode = null;     // id of the best (lowest-cost) node
let bestCost = Infinity;
let expansionOrder = []; // ordered list of expanded node ids
let expandedOnly = false;
let openNodes = new Set();
let selectedId = null;

// ── WASM loading ─────────────────────────────────────────────────────────────

async function loadWasm() {
  try {
    wasm = await import('../pkg/egg_stitch.js');
    await wasm.default();
    overlay.classList.add('hidden');
    $('btnLoad').disabled = false;
    // Auto-load from URL params (e.g. linked from results page).
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

  // Set dropdowns to match.
  const rules = params.get('rules') || '';
  $('selDomain').value = domain;
  $('selRules').value = rules;

  // Trigger load.
  $('btnLoad').click();

  if (replayFile) {
    // Wait for engine to be ready.
    await new Promise(resolve => {
      const check = () => engine ? resolve() : setTimeout(check, 50);
      check();
    });

    // Load the replay log directly.
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

    // Fetch expected cost from the corresponding run result.
    const runPath = `results/${replayFile.replace('_replay.json', '.json')}`;
    try {
      const run = await fetch(runPath).then(r => r.ok ? r.json() : null);
      console.log('run result fetch:', runPath, run ? `final_cost=${run.final_cost}` : 'null');
      if (run && run.final_cost != null) replayExpectedCost = run.final_cost;
    } catch (e) { console.warn('failed to fetch run result:', e); }

    // Also select it in the dropdown if available.
    const sel = $('selReplay');
    const opt = [...sel.options].find(o => o.value === replayFile);
    if (opt) sel.value = replayFile;

    // Replay all steps.
    console.log(`replaying ${replaySteps.length} steps...`);
    while (replayIdx < replaySteps.length) {
      if (!replayOneStep()) break;
    }
    console.log(`replay done: ${replayIdx} steps, bestCost=${bestCost}, expected=${replayExpectedCost}`);
    renderAll();
    updateReplayButtons();
    // Set status AFTER renderAll (which calls updateStatus) so it's not overwritten.
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
    originalSize = engine.original_size();
    initTree();
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

let replaySteps = [];    // loaded replay steps
let replayIdx = 0;       // next step to replay
let replayExpectedCost = null; // final_cost from the original run, for validation

/// Scan viz/results/ for runs matching the current domain that have replay logs.
async function scanReplays() {
  const sel = $('selReplay');
  const domain = $('selDomain').value;
  sel.innerHTML = '<option value="">none</option>';
  try {
    const listing = await fetch('results/').then(r => r.text());
    const { files: topFiles, dirs } = parseDirectoryListing(listing);

    const candidates = [];

    // Scan top-level files.
    for (const f of topFiles) {
      if (f.endsWith('_debug.json') || f.endsWith('_replay.json')) continue;
      try {
        const r = await fetch(`results/${f}`).then(r => r.json());
        if (r.replay_log_file && r.input_file && r.input_file.includes(`/${domain}.json`)) {
          candidates.push({ label: f.replace(/\.json$/, ''), path: r.replay_log_file, finalCost: r.final_cost });
        }
      } catch { /* skip bad files */ }
    }

    // Scan subfolders.
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

/// Parse http.server's directory listing HTML into {files, dirs}.
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

/// Apply replay config to UI controls.
function applyReplayConfig(config) {
  if (!config) return;
  if (config.priority) $('selPriority').value = config.priority;
  if (config.budget) $('numBudget').value = config.budget;
  if (config.max_arity) $('numArity').value = config.max_arity;
}

/// Load the selected replay log.
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

/// Replay one step: find the fringe node matching the logged pattern and expand it.
/// Returns true if a node was expanded, false if not found (already expanded or missing).
function replayOneStep() {
  if (replayIdx >= replaySteps.length) return false;
  const step = replaySteps[replayIdx];
  replayIdx++;

  // Find a fringe node whose pattern matches.
  const target = nodes.findIndex(n => !n.expanded && n.pattern === step.pattern);
  if (target < 0) {
    // Check if it's already expanded (ok, no-op) vs missing entirely (error).
    const exists = nodes.some(n => n.pattern === step.pattern);
    if (exists) return true; // already expanded, skip
    // Pattern not in tree — halt with error.
    const msg = `replay error at step ${replayIdx}: pattern not found in tree: ${step.pattern}`;
    statusBar.innerHTML = `<b class="bad">${msg}</b>`;
    console.error(msg);
    return false; // halt
  }
  expandNode(target);
  openNodes.add(target);
  return true;
}

$('btnReplay').addEventListener('click', () => {
  if (replayOneStep()) {
    selectedId = expansionOrder[expansionOrder.length - 1] ?? selectedId;
    renderAll();
    updateReplayButtons();
  }
});

$('btnReplayAll').addEventListener('click', () => {
  const costBefore = bestCost;
  while (replayIdx < replaySteps.length) {
    if (!replayOneStep()) break;
  }
  renderAll();
  updateReplayButtons();

  // Validate: did we reach at least the expected cost?
  if (replayExpectedCost != null && bestCost > replayExpectedCost && bestCost > costBefore) {
    const msg = `replay mismatch: expected best cost ${replayExpectedCost.toLocaleString()} but got ${bestCost.toLocaleString()}`;
    statusBar.innerHTML = `<b class="bad">${msg}</b>`;
    console.warn(msg);
  } else if (replayExpectedCost != null && bestCost <= replayExpectedCost) {
    statusBar.innerHTML = `replayed ${replayIdx} steps · best cost <b class="good">${bestCost.toLocaleString()}</b> (expected ${replayExpectedCost.toLocaleString()})`;
  }
});

// ── Tree init ────────────────────────────────────────────────────────────────

function initTree() {
  nodes = [];
  stateIdMap = [];
  heap = [];
  seen.clear();
  bestNode = null;
  bestCost = Infinity;
  expansionOrder = [];
  openNodes.clear();
  selectedId = null;

  const sid = engine.create_state();
  const info = engine.state_info(sid);
  addNode(null, sid, info, null);
  heapInsert(0); // root starts on the heap as an expandable fringe node
  openNodes.add(0);
  selectedId = 0;
  renderAll();
}

/// Add a tree node. Returns the new node id.
/// Only updates best-so-far if arity <= maxArity (matching Rust best_first behavior).
function addNode(parentId, stateId, info, action) {
  const id = nodes.length;
  const depth = parentId != null ? (nodes[parentId].depth || 0) + 1 : 0;
  nodes.push({
    id,
    parent: parentId,
    expanded: false,
    cost: info.cost,
    pattern: info.pattern,
    num_matches: info.num_matches,
    arity: info.arity,
    pattern_size: info.arity, // approximate; real pattern_size not exposed via WASM
    action: action,
    priority: null,
    depth,
  });
  stateIdMap.push(stateId);

  const maxArity = parseInt($('numArity').value) || 2;
  if (info.arity <= maxArity && info.cost < bestCost) {
    bestCost = info.cost;
    bestNode = id;
  }
  return id;
}

// ── Priority ─────────────────────────────────────────────────────────────────

function computePriority(node) {
  const key = $('selPriority').value;
  switch (key) {
    case 'cost':          return node.cost;
    case 'depth-first':   return -(node.depth || 0);
    case 'breadth-first': return node.depth || 0;
    case 'most-matches':  return -node.num_matches;
    default:              return node.cost;
  }
}

/// Insert into the sorted heap (binary search).
function heapInsert(nodeId) {
  const p = computePriority(nodes[nodeId]);
  nodes[nodeId].priority = p;
  let lo = 0, hi = heap.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (heap[mid].priority <= p) lo = mid + 1;
    else hi = mid;
  }
  heap.splice(lo, 0, { nodeId, priority: p });
}

// ── Expand ───────────────────────────────────────────────────────────────────

/// Expand a single fringe node: fetch successors from WASM, add children to
/// tree and heap. Used by both manual clicks and automated search.
function expandNode(nodeId) {
  const node = nodes[nodeId];
  if (node.expanded) return; // already expanded

  const stateId = stateIdMap[nodeId];
  const succs = engine.successors(stateId);

  node.expanded = true;
  expansionOrder.push(nodeId);

  // Remove this node from the heap.
  const hi = heap.findIndex(h => h.nodeId === nodeId);
  if (hi >= 0) heap.splice(hi, 1);

  for (const s of succs) {
    // Dedup by pattern string (matches Rust's seen.insert() check).
    const key = s.pattern;
    if (seen.has(key)) continue;
    seen.add(key);

    const childId = addNode(nodeId, s.state_id, s, s.action);
    heapInsert(childId);
  }
}

// ── Automated search ─────────────────────────────────────────────────────────

function runSteps(n) {
  for (let i = 0; i < n && heap.length > 0; i++) {
    const top = heap.shift();
    expandNode(top.nodeId);
    // Auto-open expanded nodes so the tree shows growth.
    openNodes.add(top.nodeId);
  }
}

$('btnStep').addEventListener('click', () => {
  if (heap.length === 0) return;
  runSteps(1);
  // Select the just-expanded node.
  selectedId = expansionOrder[expansionOrder.length - 1];
  renderAll();
});

$('btnRun').addEventListener('click', () => {
  const budget = parseInt($('numBudget').value) || 100;
  runSteps(budget);
  renderAll();
});

// ── UI controls ──────────────────────────────────────────────────────────────

$('btnExpandBest').addEventListener('click', () => {
  if (bestNode == null) return;
  let cur = bestNode;
  while (cur != null) { openNodes.add(cur); cur = nodes[cur].parent; }
  selectedId = bestNode;
  renderAll();
  const el = treepane.querySelector(`.row[data-id="${bestNode}"]`);
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
  renderDetail(ctx);
  renderHeap();
  updateStatus();
}

function handleRowClick(id, e) {
  const node = nodes[id];
  if (!node.expanded && stateIdMap[id] != null) {
    // Fringe node: expand it (manual interactive expansion).
    expandNode(id);
    openNodes.add(id);
  } else {
    // Expanded node: toggle disclosure.
    if (openNodes.has(id)) openNodes.delete(id);
    else openNodes.add(id);
  }
  selectedId = id;
  renderAll();
}

function renderDetail(ctx) {
  renderSidePane(detail, selectedId, ctx, null);
  wireNavLinks(detail, nodes, openNodes, id => {
    selectedId = id;
    renderAll();
    const el = treepane.querySelector(`.row[data-id="${id}"]`);
    if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
  });
}

function renderHeap() {
  heapTitle.textContent = `heap (${heap.length})`;
  heapList.innerHTML = '';
  const show = heap.slice(0, 200); // cap rendering for performance
  for (let i = 0; i < show.length; i++) {
    const h = show[i];
    const n = nodes[h.nodeId];
    const li = document.createElement('li');
    if (h.nodeId === selectedId) li.classList.add('selected');
    li.innerHTML = `
      <span class="rank">${i}</span>
      <span class="prio">${h.priority}</span>
      <span class="cost">${n.cost.toLocaleString()}</span>
      <span class="stats">m${n.num_matches} a${n.arity}</span>
      <span class="pattern">${esc(n.pattern)}</span>
    `;
    li.addEventListener('click', () => {
      selectedId = h.nodeId;
      // Ensure ancestors are open so it's visible in tree.
      let cur = h.nodeId;
      while (cur != null) { openNodes.add(nodes[cur].parent ?? 0); cur = nodes[cur].parent; }
      renderAll();
      const el = treepane.querySelector(`.row[data-id="${h.nodeId}"]`);
      if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
    });
    heapList.appendChild(li);
  }
  if (heap.length > 200) {
    const li = document.createElement('li');
    li.innerHTML = `<span class="rank">…</span><span class="pattern" style="color:var(--muted)">${heap.length - 200} more</span>`;
    heapList.appendChild(li);
  }
}

function updateStatus() {
  const nExp = expansionOrder.length;
  const ratio = bestCost < Infinity ? (originalSize / bestCost).toFixed(3) : '—';
  statusBar.innerHTML = `<b>${nodes.length}</b> nodes · <b>${nExp}</b> expanded · <b>${heap.length}</b> in heap · best cost: <b>${bestCost < Infinity ? bestCost.toLocaleString() : '—'}</b> · ratio: <b>${ratio}×</b> · original: <b>${originalSize.toLocaleString()}</b>`;
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
