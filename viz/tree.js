// Tree viewer for egg-stitch best-first search runs.
// Renders the SearchTreeLog as a nested disclosure list.

import { buildTreeMeta, buildExpOrder, buildBestPath, renderTree, renderSidePane, wireNavLinks } from './tree-render.js';

const $ = id => document.getElementById(id);
const loading = $('loading');
const treepane = $('treepane');
const side = $('sidepane');

const params = new URLSearchParams(location.search);
const file = params.get('file');

let data = null;
let cachedMeta = null;
let cachedExpOrder = null;
let cachedBestPath = null;
let expandedOnly = false;
let openNodes = new Set();
let selectedId = null;

if (!file) {
  loading.innerHTML = '<span class="err">no ?file= parameter</span>';
} else {
  fetch('results/' + file)
    .then(r => { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(d => { data = d; init(); })
    .catch(e => { loading.innerHTML = `<span class="err">failed to load ${file}: ${e}</span>`; });
}

function init() {
  if (loading.parentNode) loading.remove();
  if (!Array.isArray(data.nodes)) {
    treepane.innerHTML = '<div class="loading err">this debug file is not a search tree log (expected data.nodes)</div>';
    return;
  }
  $('title').textContent = `tree: ${file.replace(/_debug\.json$/, '')}`;
  const nExpanded = data.nodes.filter(n => n.expanded).length;
  $('meta').textContent = `${data.nodes.length} nodes · ${nExpanded} expanded · original=${data.original_size} · best=${data.best_node ?? '—'}`;

  cachedMeta = buildTreeMeta(data.nodes);
  cachedExpOrder = buildExpOrder(data.expansion_order, data.nodes.length);
  cachedBestPath = buildBestPath(data.nodes, data.best_node);

  // Open the root and the full best path by default.
  openNodes.add(0);
  for (const id of cachedBestPath) openNodes.add(id);

  $('btnExpandBest').addEventListener('click', () => {
    for (const id of cachedBestPath) openNodes.add(id);
    render();
  });
  $('btnCollapseAll').addEventListener('click', () => {
    openNodes.clear();
    openNodes.add(0);
    render();
  });
  $('chkExpandedOnly').addEventListener('change', e => { expandedOnly = e.target.checked; render(); });

  selectedId = data.best_node ?? 0;
  render();
  renderSide();
}

function buildCtx() {
  return {
    nodes: data.nodes,
    meta: cachedMeta,
    bestNode: data.best_node,
    bestPath: cachedBestPath,
    expOrder: cachedExpOrder,
    originalSize: data.original_size,
    openNodes,
    selectedId,
    expandedOnly,
    onRowClick: handleRowClick,
  };
}

function handleRowClick(id, _e) {
  const kids = cachedMeta.children.get(id) || [];
  const visible = expandedOnly ? kids.filter(k => data.nodes[k].expanded) : kids;
  if (visible.length > 0) {
    if (openNodes.has(id)) openNodes.delete(id);
    else openNodes.add(id);
  }
  selectedId = id;
  render();
  renderSide();
}

function render() {
  renderTree(treepane, buildCtx());
}

function renderSide() {
  const ctx = buildCtx();
  renderSidePane(side, selectedId, ctx, null);
  wireNavLinks(side, data.nodes, openNodes, id => {
    selectedId = id;
    render();
    renderSide();
    const el = treepane.querySelector(`.row[data-id="${id}"]`);
    if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
  });
}
