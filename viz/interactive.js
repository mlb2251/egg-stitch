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
  } catch (e) {
    overlay.textContent = `failed to load WASM: ${e}. Run "make wasm" first.`;
    console.error(e);
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
}

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

  if (info.cost < bestCost) {
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

  const maxArity = parseInt($('numArity').value) || 3;
  const stateId = stateIdMap[nodeId];
  const succs = engine.successors(stateId);

  node.expanded = true;
  expansionOrder.push(nodeId);

  // Remove this node from the heap.
  const hi = heap.findIndex(h => h.nodeId === nodeId);
  if (hi >= 0) heap.splice(hi, 1);

  for (const s of succs) {
    if (s.arity > maxArity) continue;
    // Dedup by pattern string.
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
