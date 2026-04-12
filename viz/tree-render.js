// Shared tree rendering for egg-stitch viewers.
// Used by both tree.js (static viewer) and interactive.js (live explorer).

export function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
}

/// Build derived tree metadata from a flat nodes array.
/// Returns { children: Map<id, id[]>, subtreeMin: number[], subtreeExp: number[] }.
export function buildTreeMeta(nodes) {
  const children = new Map();
  for (const n of nodes) {
    if (n.parent != null) {
      if (!children.has(n.parent)) children.set(n.parent, []);
      children.get(n.parent).push(n.id);
    }
  }
  const subtreeMin = nodes.map(n => n.cost);
  const subtreeExp = nodes.map(n => n.expanded ? 1 : 0);
  for (let i = nodes.length - 1; i > 0; i--) {
    const p = nodes[i].parent;
    if (p != null) {
      if (subtreeMin[i] < subtreeMin[p]) subtreeMin[p] = subtreeMin[i];
      subtreeExp[p] += subtreeExp[i];
    }
  }
  for (const arr of children.values()) arr.sort((a, b) => subtreeMin[a] - subtreeMin[b]);
  return { children, subtreeMin, subtreeExp };
}

/// Build expansion-order lookup: nodeId → pop index (or -1).
export function buildExpOrder(expansionOrder, numNodes) {
  const eo = new Array(numNodes).fill(-1);
  if (Array.isArray(expansionOrder)) {
    expansionOrder.forEach((id, i) => { eo[id] = i; });
  }
  return eo;
}

/// Build the set of node ids on the path from root to bestNode.
export function buildBestPath(nodes, bestNode) {
  const bp = new Set();
  if (bestNode != null) {
    let cur = bestNode;
    while (cur != null) { bp.add(cur); cur = nodes[cur].parent; }
  }
  return bp;
}

/// Render the full tree into `treepane`.
/// `ctx` is the rendering context (see below).
export function renderTree(treepane, ctx) {
  const root = document.createElement('ul');
  root.className = 'tree-list';
  if (ctx.nodes.length > 0) root.appendChild(renderNode(0, ctx));
  treepane.innerHTML = '';
  treepane.appendChild(root);
}

/*
 * Rendering context (ctx):
 *   nodes         — flat array of node objects
 *   meta          — { children, subtreeMin, subtreeExp } from buildTreeMeta
 *   bestNode      — id of the best node, or null
 *   bestPath      — Set of ids on the root→best path
 *   expOrder      — array from buildExpOrder
 *   originalSize  — corpus size before compression
 *   openNodes     — Set of ids whose children are UI-visible
 *   selectedId    — currently selected id, or null
 *   expandedOnly  — if true, hide non-expanded children
 *   onRowClick(id, event) — callback when a row is clicked
 */

function visibleChildren(id, ctx) {
  const kids = ctx.meta.children.get(id) || [];
  return ctx.expandedOnly ? kids.filter(k => ctx.nodes[k].expanded) : kids;
}

function renderNode(id, ctx) {
  const n = ctx.nodes[id];
  const kids = visibleChildren(id, ctx);
  const isOpen = ctx.openNodes.has(id);
  const isBest = id === ctx.bestNode;
  const onBest = ctx.bestPath.has(id);

  const li = document.createElement('li');
  const row = document.createElement('div');
  row.className = 'row' + (onBest ? ' on-best' : '') + (id === ctx.selectedId ? ' selected' : '');
  row.dataset.id = id;

  const caret = document.createElement('span');
  caret.className = 'caret' + (kids.length === 0 ? ' leaf' : '');
  caret.textContent = kids.length === 0 ? '·' : (isOpen ? '▼' : '▶');
  row.appendChild(caret);

  const ei = ctx.expOrder[id] ?? -1;
  const expBadge = document.createElement('span');
  if (isBest) {
    expBadge.className = 'exp-badge best';
    expBadge.textContent = ei >= 0 ? ei : '';
    expBadge.title = `best node · #${id}` + (ei >= 0 ? ` · pop #${ei}` : '');
  } else if (n.expanded) {
    expBadge.className = 'exp-badge expanded';
    expBadge.textContent = ei >= 0 ? ei : '';
    expBadge.title = `expanded · #${id}` + (ei >= 0 ? ` · pop #${ei}` : '');
  } else {
    expBadge.className = 'exp-badge fringe';
    expBadge.textContent = '';
    expBadge.title = `fringe · #${id}`;
  }
  row.appendChild(expBadge);

  const prio = document.createElement('span');
  prio.className = 'prio';
  prio.textContent = n.priority != null ? n.priority : '';
  row.appendChild(prio);

  const cost = document.createElement('span');
  cost.className = 'cost';
  cost.textContent = n.cost.toLocaleString();
  row.appendChild(cost);

  if (n.parent != null) {
    const diff = n.cost - ctx.nodes[n.parent].cost;
    const cdiff = document.createElement('span');
    cdiff.className = 'cost-diff' + (diff < 0 ? ' good' : diff > 0 ? ' bad' : '');
    cdiff.textContent = (diff > 0 ? '+' : '') + diff.toLocaleString();
    row.appendChild(cdiff);
  }

  const sm = ctx.meta.subtreeMin[id];
  const submin = document.createElement('span');
  submin.className = 'submin' + (sm === n.cost ? ' same' : '');
  submin.textContent = `\u2193 ${sm.toLocaleString()}`;
  row.appendChild(submin);

  const stats = document.createElement('span');
  stats.className = 'stats';
  stats.textContent = `sz${n.pattern_size ?? n.arity}\u00b7m${n.num_matches}`;
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
    badge.textContent = `${kids.length} ch`;
    row.appendChild(badge);
  }

  const se = ctx.meta.subtreeExp[id];
  if (se > 0) {
    const seBadge = document.createElement('span');
    seBadge.className = 'badge subtree-exp';
    seBadge.textContent = `${se} exp`;
    row.appendChild(seBadge);
  }

  if (isBest) {
    const badge = document.createElement('span');
    badge.className = 'badge best';
    badge.textContent = 'best';
    row.appendChild(badge);
  }

  row.addEventListener('click', e => ctx.onRowClick(id, e));
  li.appendChild(row);

  if (isOpen && kids.length > 0) {
    const ul = document.createElement('ul');
    for (const k of kids) ul.appendChild(renderNode(k, ctx));
    li.appendChild(ul);
  }
  return li;
}

/// Render the side/detail pane for the selected node.
export function renderSidePane(pane, id, ctx, extraHtml) {
  if (id == null) {
    pane.innerHTML = '<div class="empty">click a node to inspect</div>';
    return;
  }
  const n = ctx.nodes[id];
  const kids = ctx.meta.children.get(id) || [];
  const ratio = ctx.originalSize ? (ctx.originalSize / n.cost) : null;
  const isBest = id === ctx.bestNode;
  const ei = ctx.expOrder[id] ?? -1;

  pane.innerHTML = `
    <h2>node ${n.id}${isBest ? ' \u00b7 best' : ''}</h2>
    <dl>
      <dt>cost</dt><dd${isBest ? ' class="good"' : ''}>${n.cost.toLocaleString()}</dd>
      <dt>ratio</dt><dd>${ratio ? ratio.toFixed(3) + '\u00d7' : '\u2014'}</dd>
      <dt>arity</dt><dd>${n.arity}</dd>
      <dt>matches</dt><dd>${n.num_matches}</dd>
      <dt>expanded</dt><dd>${n.expanded ? (ei >= 0 ? `yes (#${ei})` : 'yes') : 'no (fringe)'}</dd>
      <dt>parent</dt><dd>${n.parent != null ? `<a class="nav" data-id="${n.parent}">#${n.parent}</a>` : '\u2014'}</dd>
      <dt>children</dt><dd>${kids.length}${kids.length ? ' \u00b7 ' + kids.slice(0, 20).map(k => `<a class="nav" data-id="${k}">#${k}</a>`).join(' ') + (kids.length > 20 ? ' \u2026' : '') : ''}</dd>
    </dl>
    <h2>action</h2>
    ${n.action ? `<div class="action">${escapeHtml(n.action)}</div>` : '<div class="empty">root</div>'}
    <h2>pattern</h2>
    <div class="pattern">${escapeHtml(n.pattern)}</div>
    ${extraHtml || ''}
  `;
}

/// Wire up .nav links inside a pane to call `navigate(id)`.
export function wireNavLinks(pane, nodes, openNodes, navigate) {
  pane.querySelectorAll('a.nav').forEach(a => {
    a.addEventListener('click', e => {
      e.preventDefault();
      const id = +a.dataset.id;
      let cur = id;
      while (cur != null) { openNodes.add(nodes[cur].parent ?? 0); cur = nodes[cur].parent; }
      navigate(id);
    });
  });
}
