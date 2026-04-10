// Tree viewer for egg-stitch best-first search runs.
// Renders the SearchTreeLog as a nested disclosure list — each node is a row
// the user can click to expand/collapse its children. Designed for high-
// branching search trees where an SVG layout would be unreadable.

const $ = id => document.getElementById(id);
const loading = $('loading');
const treepane = $('treepane');
const side = $('sidepane');

const params = new URLSearchParams(location.search);
const file = params.get('file');

let data = null;
let children = new Map();    // parent id -> child ids (sorted by subtree min cost)
let subtreeMin = [];         // node id -> min cost anywhere in its subtree
let expOrder = [];           // node id -> expansion pop index, or -1
let bestPathNodes = new Set();
let expandedOnly = false;
let openNodes = new Set();   // ids whose children are visible
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

  children = new Map();
  for (const n of data.nodes) {
    if (n.parent != null) {
      if (!children.has(n.parent)) children.set(n.parent, []);
      children.get(n.parent).push(n.id);
    }
  }
  // Subtree min cost: the best (lowest) cost reachable anywhere at or below
  // each node. Computed by processing ids in reverse order — children always
  // have higher ids than their parent since nodes are appended on discovery.
  subtreeMin = data.nodes.map(n => n.cost);
  for (let i = data.nodes.length - 1; i > 0; i--) {
    const p = data.nodes[i].parent;
    if (p != null && subtreeMin[i] < subtreeMin[p]) subtreeMin[p] = subtreeMin[i];
  }
  for (const arr of children.values()) arr.sort((a, b) => subtreeMin[a] - subtreeMin[b]);

  expOrder = new Array(data.nodes.length).fill(-1);
  if (Array.isArray(data.expansion_order)) {
    data.expansion_order.forEach((id, i) => { expOrder[id] = i; });
  }

  bestPathNodes.clear();
  if (data.best_node != null) {
    let cur = data.best_node;
    while (cur != null) {
      bestPathNodes.add(cur);
      cur = data.nodes[cur].parent;
    }
  }

  // Open the root and the full best path by default.
  openNodes.add(0);
  for (const id of bestPathNodes) openNodes.add(id);

  $('btnExpandBest').addEventListener('click', () => {
    for (const id of bestPathNodes) openNodes.add(id);
    render();
  });
  $('btnCollapseAll').addEventListener('click', () => {
    openNodes.clear();
    openNodes.add(0);
    render();
  });
  $('chkExpandedOnly').addEventListener('change', (e) => { expandedOnly = e.target.checked; render(); });

  selectedId = data.best_node ?? 0;
  render();
  renderSide();
}

// --- Render ---

function visibleChildren(id) {
  const kids = children.get(id) || [];
  return expandedOnly ? kids.filter(k => data.nodes[k].expanded) : kids;
}

function render() {
  const root = document.createElement('ul');
  root.className = 'tree-list';
  root.appendChild(renderNode(0));
  treepane.innerHTML = '';
  treepane.appendChild(root);
}

function renderNode(id) {
  const n = data.nodes[id];
  const kids = visibleChildren(id);
  const isOpen = openNodes.has(id);
  const isBest = id === data.best_node;
  const onBest = bestPathNodes.has(id);

  const li = document.createElement('li');

  const row = document.createElement('div');
  row.className = 'row' + (onBest ? ' on-best' : '') + (id === selectedId ? ' selected' : '');
  row.dataset.id = id;

  const caret = document.createElement('span');
  caret.className = 'caret' + (kids.length === 0 ? ' leaf' : '');
  caret.textContent = kids.length === 0 ? '·' : (isOpen ? '▼' : '▶');
  row.appendChild(caret);

  const dot = document.createElement('span');
  const state = isBest ? 'best' : (n.expanded ? 'expanded' : 'fringe');
  dot.className = 'dot ' + state;
  row.appendChild(dot);

  const idSpan = document.createElement('span');
  idSpan.className = 'id';
  idSpan.textContent = `#${id}`;
  idSpan.title = `node id #${id}`;
  row.appendChild(idSpan);

  const eord = document.createElement('span');
  const ei = expOrder[id];
  eord.className = 'eord' + (ei < 0 ? ' none' : '');
  eord.textContent = ei < 0 ? '—' : `e${ei}`;
  eord.title = ei < 0 ? 'not expanded' : `expansion order #${ei}`;
  row.appendChild(eord);

  const cost = document.createElement('span');
  cost.className = 'cost';
  cost.textContent = n.cost.toLocaleString();
  cost.title = 'cost at this node';
  row.appendChild(cost);

  const sm = subtreeMin[id];
  const submin = document.createElement('span');
  submin.className = 'submin' + (sm === n.cost ? ' same' : '');
  submin.textContent = `↓ ${sm.toLocaleString()}`;
  submin.title = 'min cost reachable in this subtree';
  row.appendChild(submin);

  const stats = document.createElement('span');
  stats.className = 'stats';
  stats.textContent = `sz${n.pattern_size ?? n.arity}·m${n.num_matches}`;
  stats.title = `pattern size ${n.pattern_size ?? '—'} · ${n.num_matches} matches`;
  row.appendChild(stats);

  const pat = document.createElement('span');
  pat.className = 'pattern';
  pat.textContent = n.pattern;
  row.appendChild(pat);

  if (n.action) {
    const act = document.createElement('span');
    act.className = 'action';
    act.textContent = n.action;
    row.appendChild(act);
  }

  if (kids.length > 0) {
    const badge = document.createElement('span');
    badge.className = 'badge';
    badge.textContent = `${kids.length}`;
    row.appendChild(badge);
  }
  if (isBest) {
    const badge = document.createElement('span');
    badge.className = 'badge best';
    badge.textContent = 'best';
    row.appendChild(badge);
  }

  row.addEventListener('click', (e) => {
    if (kids.length > 0) {
      if (openNodes.has(id)) openNodes.delete(id);
      else openNodes.add(id);
    }
    selectedId = id;
    render();
    renderSide();
  });

  li.appendChild(row);

  if (isOpen && kids.length > 0) {
    const ul = document.createElement('ul');
    for (const k of kids) ul.appendChild(renderNode(k));
    li.appendChild(ul);
  }

  return li;
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
      const id = +a.dataset.id;
      // Open ancestors so the target is visible.
      let cur = id;
      while (cur != null) {
        openNodes.add(data.nodes[cur].parent ?? 0);
        cur = data.nodes[cur].parent;
      }
      selectedId = id;
      render();
      renderSide();
      const el = treepane.querySelector(`.row[data-id="${id}"]`);
      if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' });
    });
  });
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
