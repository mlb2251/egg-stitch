// Tree viewer for egg-stitch best-first search runs.
// Loads a SearchTreeLog JSON file and renders the explored search tree.

const $ = id => document.getElementById(id);
const loading = $('loading');
const treepane = $('treepane');
const side = $('sidepane');

const params = new URLSearchParams(location.search);
const file = params.get('file');

let data = null;
let children = new Map();   // parent id -> sorted child id list
let pos = null;             // node id -> { y, depth } in logical coords
let bestPathEdges = new Set();
let bestPathNodes = new Set();
let selectedId = null;
let layoutMode = 'h';
let showEdgeLabels = false;
let showBestPath = true;

if (!file) {
  loading.innerHTML = '<span class="err">no ?file= parameter</span>';
} else {
  fetch('results/' + file)
    .then(r => { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(d => { data = d; init(); })
    .catch(e => { loading.innerHTML = `<span class="err">failed to load ${file}: ${e}</span>`; });
}

// --- Init ---

function init() {
  if (loading.parentNode) loading.remove();
  if (!Array.isArray(data.nodes)) {
    treepane.innerHTML = '<div class="loading err">this debug file is not a search tree log (expected data.nodes)</div>';
    return;
  }
  $('title').textContent = `tree: ${file.replace(/_debug\.json$/, '')}`;
  const nExpanded = data.nodes.filter(n => n.expanded).length;
  $('meta').textContent = `${data.nodes.length} nodes · ${nExpanded} expanded · original=${data.original_size} · best=${data.best_node ?? '—'}`;

  children = new Map();
  for (const n of data.nodes) {
    if (n.parent != null) {
      if (!children.has(n.parent)) children.set(n.parent, []);
      children.get(n.parent).push(n.id);
    }
  }
  // Sort siblings by cost so the lowest-cost branch sits on top.
  for (const arr of children.values()) arr.sort((a, b) => data.nodes[a].cost - data.nodes[b].cost);

  computeLayout();

  // Best path: walk parent pointers from best_node.
  bestPathEdges.clear();
  bestPathNodes.clear();
  if (data.best_node != null) {
    let cur = data.best_node;
    while (cur != null) {
      bestPathNodes.add(cur);
      const parent = data.nodes[cur].parent;
      if (parent != null) bestPathEdges.add(`${parent}-${cur}`);
      cur = parent;
    }
  }

  $('chkBestPath').addEventListener('change', (e) => { showBestPath = e.target.checked; render(); });
  $('chkEdgeLabels').addEventListener('change', (e) => { showEdgeLabels = e.target.checked; render(); });
  $('layout').addEventListener('change', (e) => { layoutMode = e.target.value; computeLayout(); render(); });

  selectedId = data.best_node ?? 0;
  render();
  renderSide();
  setTimeout(() => scrollToNode(selectedId), 0);
}

// --- Layout ---

// Two-pass layered tree: post-order compute subtree leaf counts, top-down assign
// center position of each parent as the midpoint of its children.
function computeLayout() {
  const n = data.nodes.length;
  const leafCount = new Array(n).fill(1);
  function post(id) {
    const kids = children.get(id) || [];
    if (kids.length === 0) return (leafCount[id] = 1);
    let total = 0;
    for (const k of kids) total += post(k);
    return (leafCount[id] = total);
  }
  post(0);

  pos = new Array(n);
  function assign(id, yStart, depth) {
    const kids = children.get(id) || [];
    if (kids.length === 0) {
      pos[id] = { y: yStart + 0.5, depth };
      return 1;
    }
    let y = yStart;
    for (const k of kids) y += assign(k, y, depth + 1);
    const first = pos[kids[0]].y;
    const last = pos[kids[kids.length - 1]].y;
    pos[id] = { y: (first + last) / 2, depth };
    return leafCount[id];
  }
  assign(0, 0, 0);
}

// --- Render ---

function render() {
  const xStep = layoutMode === 'h' ? 110 : 18;
  const yStep = layoutMode === 'h' ? 18 : 110;
  const padX = 50, padY = 50;

  let maxDepth = 0, maxY = 0;
  for (const p of pos) {
    if (!p) continue;
    if (p.depth > maxDepth) maxDepth = p.depth;
    if (p.y > maxY) maxY = p.y;
  }

  const W = (layoutMode === 'h' ? maxDepth * xStep : maxY * xStep) + padX * 2 + 220;
  const H = (layoutMode === 'h' ? maxY * yStep : maxDepth * yStep) + padY * 2 + 40;

  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('class', 'tree');
  svg.setAttribute('width', W);
  svg.setAttribute('height', H);

  function coord(id) {
    const p = pos[id];
    if (layoutMode === 'h') return { x: padX + p.depth * xStep, y: padY + p.y * yStep };
    return { x: padX + p.y * xStep, y: padY + p.depth * yStep };
  }

  // Edges
  const gEdges = document.createElementNS(NS, 'g');
  svg.appendChild(gEdges);
  for (const n of data.nodes) {
    if (n.parent == null) continue;
    const a = coord(n.parent), b = coord(n.id);
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
    const d = layoutMode === 'h'
      ? `M${a.x},${a.y} C${mx},${a.y} ${mx},${b.y} ${b.x},${b.y}`
      : `M${a.x},${a.y} C${a.x},${my} ${b.x},${my} ${b.x},${b.y}`;
    const path = document.createElementNS(NS, 'path');
    path.setAttribute('d', d);
    const onBest = showBestPath && bestPathEdges.has(`${n.parent}-${n.id}`);
    path.setAttribute('class', 'edge' + (onBest ? ' on-best' : ''));
    gEdges.appendChild(path);

    if (showEdgeLabels && n.action) {
      const t = document.createElementNS(NS, 'text');
      t.setAttribute('class', 'edge-label');
      t.setAttribute('x', mx + 4);
      t.setAttribute('y', my - 3);
      t.textContent = n.action;
      gEdges.appendChild(t);
    }
  }

  // Nodes
  const gNodes = document.createElementNS(NS, 'g');
  svg.appendChild(gNodes);
  for (const n of data.nodes) {
    const p = coord(n.id);
    const g = document.createElementNS(NS, 'g');
    const state = n.id === data.best_node ? 'best' : (n.expanded ? 'expanded' : 'fringe');
    g.setAttribute('class', 'node ' + state + (n.id === selectedId ? ' selected' : ''));
    g.setAttribute('transform', `translate(${p.x},${p.y})`);
    g.dataset.id = n.id;
    const c = document.createElementNS(NS, 'circle');
    c.setAttribute('r', n.id === data.best_node ? 7 : 5);
    g.appendChild(c);
    if (n.id === data.best_node) {
      const t = document.createElementNS(NS, 'text');
      t.setAttribute('x', layoutMode === 'h' ? 12 : 0);
      t.setAttribute('y', layoutMode === 'h' ? 3 : -11);
      t.setAttribute('text-anchor', layoutMode === 'h' ? 'start' : 'middle');
      t.textContent = `best: ${n.cost}`;
      g.appendChild(t);
    }
    g.addEventListener('click', () => {
      selectedId = n.id;
      render();
      renderSide();
    });
    gNodes.appendChild(g);
  }

  treepane.innerHTML = '';
  treepane.appendChild(svg);
}

// --- Side pane ---

function renderSide() {
  if (selectedId == null) {
    side.innerHTML = '<div class="empty">click a node to inspect</div>';
    return;
  }
  const n = data.nodes[selectedId];
  const kids = children.get(n.id) || [];
  const ratio = data.original_size ? (data.original_size / n.cost) : null;
  const isBest = n.id === data.best_node;
  // Expansion order index, if this node was popped from the heap.
  let popIdx = -1;
  if (Array.isArray(data.expansion_order)) popIdx = data.expansion_order.indexOf(n.id);

  side.innerHTML = `
    <h2>node ${n.id}${isBest ? ' · best' : ''}</h2>
    <dl>
      <dt>cost</dt><dd${isBest ? ' class="good"' : ''}>${n.cost}</dd>
      <dt>ratio</dt><dd>${ratio ? ratio.toFixed(3) + '×' : '—'}</dd>
      <dt>arity</dt><dd>${n.arity}</dd>
      <dt>matches</dt><dd>${n.num_matches}</dd>
      <dt>expanded</dt><dd>${n.expanded ? (popIdx >= 0 ? `yes (#${popIdx} in pop order)` : 'yes') : 'no'}</dd>
      <dt>parent</dt><dd>${n.parent != null ? `<a class="nav" data-id="${n.parent}">#${n.parent}</a>` : '—'}</dd>
      <dt>children</dt><dd>${kids.length}${kids.length ? ' · ' + kids.map(k => `<a class="nav" data-id="${k}">#${k}</a>`).join(' ') : ''}</dd>
    </dl>
    <h2>action</h2>
    ${n.action ? `<div class="action">${escapeHtml(n.action)}</div>` : '<div class="empty">root</div>'}
    <h2>pattern</h2>
    <div class="pattern">${escapeHtml(n.pattern)}</div>
  `;
  side.querySelectorAll('a.nav').forEach(a => {
    a.addEventListener('click', (e) => {
      e.preventDefault();
      selectedId = +a.dataset.id;
      render();
      renderSide();
      scrollToNode(selectedId);
    });
  });
}

function scrollToNode(id) {
  const el = treepane.querySelector(`.node[data-id="${id}"]`);
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const pRect = treepane.getBoundingClientRect();
  const dx = rect.left - pRect.left - pRect.width / 2 + rect.width / 2;
  const dy = rect.top - pRect.top - pRect.height / 2 + rect.height / 2;
  treepane.scrollBy({ left: dx, top: dy, behavior: 'instant' });
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
