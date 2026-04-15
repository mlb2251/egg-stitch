/* viz/stackpath viewer
 *
 * Three-panel UI over viz/stackpath/:
 *   - left top: directory tree (expand/collapse, select, rename, delete)
 *   - left bottom: saved selections (switch, rename, save, delete)
 *   - right: list of runs under current selection
 *   - top:   Plot of elapsed_secs (log) vs compression_ratio
 *
 * State is intentionally simple: a single in-memory tree, a Set of selected
 * paths, and a Map of cached run JSON. Mutations re-render the affected
 * panels rather than diffing.
 */

const state = {
  tree: null,                // root tree node (children-of-stackpath wrapped)
  expanded: new Set(),       // paths of expanded directories
  selected: new Set(),       // paths of currently-selected directories
  selections: [],            // names of saved selections
  activeSelection: null,     // currently-loaded selection name (or null)
  runs: new Map(),           // path -> {config, result, type}
};

// ---------- HTTP helpers ----------

async function api(path, opts = {}) {
  const res = await fetch(path, opts);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}: ${await res.text().catch(() => '')}`);
  return res;
}

async function getJSON(path) {
  return (await api(path)).json();
}

async function putJSON(path, body) {
  return api(path, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body, null, 2),
  });
}

async function del(path) {
  return api(path, { method: 'DELETE' });
}

async function rename(from, to) {
  return api('/api/rename', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ from, to }),
  });
}

// ---------- toast ----------

let toastTimer = null;
function toast(msg, isErr = false) {
  const t = document.getElementById('toast');
  t.textContent = msg;
  t.classList.toggle('err', isErr);
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), 2200);
}

// ---------- tree helpers ----------

function findNode(node, path) {
  if (node.path === path) return node;
  for (const c of node.children) {
    const found = findNode(c, path);
    if (found) return found;
  }
  return null;
}

function isDescendant(child, ancestor) {
  return child === ancestor || child.startsWith(ancestor + '/');
}

/** Walk node and yield every run-directory descendant (paths). */
function* runsBelow(node) {
  if (node.is_run) yield node.path;
  for (const c of node.children) yield* runsBelow(c);
}

/** Compute the set of run paths covered by the current selection.
 * Selecting both a parent and a descendant collapses to the parent's runs. */
function selectedRunPaths() {
  if (!state.tree) return [];
  const sel = [...state.selected];
  const pruned = sel.filter(p => !sel.some(q => q !== p && isDescendant(p, q)));
  // Preserve tree order by walking from root.
  const order = [];
  const walk = (node) => {
    if (pruned.includes(node.path)) for (const r of runsBelow(node)) order.push(r);
    else for (const c of node.children) walk(c);
  };
  walk(state.tree);
  return [...new Set(order)];
}

// ---------- rendering: tree ----------

function nodeClass(node) {
  if (state.selected.has(node.path)) return 'sel-direct';
  for (const s of state.selected) {
    if (node.path !== s && isDescendant(node.path, s)) return 'sel-descendant';
  }
  return '';
}

function renderTree() {
  const ul = document.getElementById('tree');
  ul.innerHTML = '';
  if (!state.tree) return;
  for (const child of state.tree.children) ul.appendChild(renderNode(child));
}

function renderNode(node) {
  const li = document.createElement('li');
  const row = document.createElement('div');
  row.className = 'node ' + nodeClass(node);
  row.dataset.path = node.path;

  const hasChildren = node.children.length > 0;
  const expanded = state.expanded.has(node.path);

  const twist = document.createElement('span');
  twist.className = 'twist';
  twist.textContent = hasChildren ? (expanded ? '▾' : '▸') : ' ';
  twist.onclick = (e) => {
    e.stopPropagation();
    if (!hasChildren) return;
    if (expanded) state.expanded.delete(node.path);
    else state.expanded.add(node.path);
    renderTree();
  };
  row.appendChild(twist);

  const name = document.createElement('span');
  name.className = 'name' + (node.is_run ? ' run' : '');
  name.textContent = node.name;
  name.title = node.path;
  row.appendChild(name);

  const del = document.createElement('button');
  del.className = 'del';
  del.textContent = '×';
  del.title = 'delete';
  del.onclick = (e) => {
    e.stopPropagation();
    deleteNode(node);
  };
  row.appendChild(del);

  // Click selects/unselects (toggle).
  row.onclick = () => toggleSelect(node.path);

  // Double-click on the name renames.
  name.ondblclick = (e) => {
    e.stopPropagation();
    startRenameNode(node, name);
  };

  li.appendChild(row);
  if (hasChildren && expanded) {
    const sub = document.createElement('ul');
    for (const c of node.children) sub.appendChild(renderNode(c));
    li.appendChild(sub);
  }
  return li;
}

function toggleSelect(path) {
  if (state.selected.has(path)) state.selected.delete(path);
  else state.selected.add(path);
  state.activeSelection = null;  // editing the live selection forgets the loaded name
  renderTree();
  renderSelectionsList();
  renderRuns();
}

async function deleteNode(node) {
  const unsafe = document.getElementById('unsafe-del').checked;
  if (!unsafe && !confirm(`Delete ${node.path}?`)) return;
  try {
    await del('/' + node.path);
    toast(`deleted ${node.path}`);
    // Drop any selections/expansions that referred to anything under this path.
    for (const s of [...state.selected]) if (isDescendant(s, node.path)) state.selected.delete(s);
    for (const e of [...state.expanded]) if (isDescendant(e, node.path)) state.expanded.delete(e);
    await refreshTree();
  } catch (err) {
    toast(`delete failed: ${err.message}`, true);
  }
}

function startRenameNode(node, nameEl) {
  const original = node.name;
  nameEl.contentEditable = 'true';
  nameEl.focus();
  // Select all text.
  const range = document.createRange();
  range.selectNodeContents(nameEl);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);

  const finish = async (commit) => {
    nameEl.removeEventListener('blur', onBlur);
    nameEl.removeEventListener('keydown', onKey);
    nameEl.contentEditable = 'false';
    const newName = nameEl.textContent.trim();
    if (!commit || !newName || newName === original) {
      nameEl.textContent = original;
      return;
    }
    if (newName.includes('/')) {
      toast('name cannot contain "/"', true);
      nameEl.textContent = original;
      return;
    }
    const parent = node.path.includes('/') ? node.path.slice(0, node.path.lastIndexOf('/')) : '';
    const newPath = parent ? `${parent}/${newName}` : newName;
    try {
      await rename('/' + node.path, '/' + newPath);
      // Re-key any selection/expansion entries that point at this subtree.
      remapPaths(node.path, newPath);
      await refreshTree();
      toast(`renamed to ${newName}`);
    } catch (err) {
      toast(`rename failed: ${err.message}`, true);
      nameEl.textContent = original;
    }
  };
  const onBlur = () => finish(true);
  const onKey = (e) => {
    if (e.key === 'Enter') { e.preventDefault(); nameEl.blur(); }
    else if (e.key === 'Escape') { e.preventDefault(); finish(false); nameEl.blur(); }
  };
  nameEl.addEventListener('blur', onBlur);
  nameEl.addEventListener('keydown', onKey);
}

function remapPaths(oldPrefix, newPrefix) {
  const remap = (set) => {
    const out = new Set();
    for (const p of set) {
      if (p === oldPrefix) out.add(newPrefix);
      else if (p.startsWith(oldPrefix + '/')) out.add(newPrefix + p.slice(oldPrefix.length));
      else out.add(p);
    }
    return out;
  };
  state.selected = remap(state.selected);
  state.expanded = remap(state.expanded);
}

// ---------- rendering: selections ----------

function renderSelectionsList() {
  const div = document.getElementById('selections');
  div.innerHTML = '';
  for (const name of state.selections) {
    const item = document.createElement('div');
    item.className = 'sel-item' + (state.activeSelection === name ? ' active' : '');
    const n = document.createElement('span');
    n.className = 'name';
    n.textContent = name;
    item.appendChild(n);
    const del = document.createElement('button');
    del.className = 'del'; del.textContent = '×'; del.title = 'delete selection';
    del.onclick = (e) => { e.stopPropagation(); deleteSelection(name); };
    item.appendChild(del);
    item.onclick = () => loadSelection(name);
    n.ondblclick = (e) => { e.stopPropagation(); startRenameSelection(name, n); };
    div.appendChild(item);
  }
}

async function loadSelection(name) {
  try {
    const sel = await getJSON(`/viz/selections/${encodeURIComponent(name)}.json`);
    state.selected = new Set(sel.dirs || []);
    state.activeSelection = name;
    renderTree();
    renderSelectionsList();
    renderRuns();
  } catch (err) {
    toast(`load failed: ${err.message}`, true);
  }
}

async function saveSelection() {
  let name = state.activeSelection;
  if (!name) {
    name = prompt('selection name?');
    if (!name) return;
    name = name.trim();
    if (!name) return;
    if (state.selections.includes(name)) {
      toast(`selection "${name}" already exists`, true);
      return;
    }
  }
  try {
    await putJSON(`/viz/selections/${encodeURIComponent(name)}.json`, {
      dirs: [...state.selected],
    });
    state.activeSelection = name;
    await refreshSelections();
    toast(`saved selection "${name}"`);
  } catch (err) {
    toast(`save failed: ${err.message}`, true);
  }
}

async function deleteSelection(name) {
  if (!document.getElementById('unsafe-del').checked &&
      !confirm(`Delete selection "${name}"?`)) return;
  try {
    await del(`/viz/selections/${encodeURIComponent(name)}.json`);
    if (state.activeSelection === name) state.activeSelection = null;
    await refreshSelections();
    toast(`deleted selection "${name}"`);
  } catch (err) {
    toast(`delete failed: ${err.message}`, true);
  }
}

function startRenameSelection(name, nameEl) {
  nameEl.contentEditable = 'true';
  nameEl.focus();
  const range = document.createRange();
  range.selectNodeContents(nameEl);
  const sel = window.getSelection();
  sel.removeAllRanges(); sel.addRange(range);
  const finish = async (commit) => {
    nameEl.removeEventListener('blur', onBlur);
    nameEl.removeEventListener('keydown', onKey);
    nameEl.contentEditable = 'false';
    const newName = nameEl.textContent.trim();
    if (!commit || !newName || newName === name) { nameEl.textContent = name; return; }
    if (newName.includes('/')) { toast('name cannot contain "/"', true); nameEl.textContent = name; return; }
    if (state.selections.includes(newName)) {
      toast(`selection "${newName}" already exists`, true);
      nameEl.textContent = name;
      return;
    }
    try {
      await rename(
        `/viz/selections/${name}.json`,
        `/viz/selections/${newName}.json`,
      );
      if (state.activeSelection === name) state.activeSelection = newName;
      await refreshSelections();
      toast(`renamed to "${newName}"`);
    } catch (err) {
      toast(`rename failed: ${err.message}`, true);
      nameEl.textContent = name;
    }
  };
  const onBlur = () => finish(true);
  const onKey = (e) => {
    if (e.key === 'Enter') { e.preventDefault(); nameEl.blur(); }
    else if (e.key === 'Escape') { e.preventDefault(); finish(false); nameEl.blur(); }
  };
  nameEl.addEventListener('blur', onBlur);
  nameEl.addEventListener('keydown', onKey);
}

// ---------- rendering: runs + plot ----------

async function loadRun(path) {
  if (state.runs.has(path)) return state.runs.get(path);
  const [config, result, type] = await Promise.all([
    fetch(`/${path}/config.json`).then(r => r.ok ? r.json() : {}),
    fetch(`/${path}/result.json`).then(r => r.ok ? r.json() : {}),
    fetch(`/${path}/type.txt`).then(r => r.ok ? r.text() : '').then(t => t.trim()),
  ]);
  const run = { config, result, type, path };
  state.runs.set(path, run);
  return run;
}

function fmtNum(v) {
  if (typeof v !== 'number') return String(v);
  if (Number.isInteger(v)) return v.toString();
  if (Math.abs(v) >= 100) return v.toFixed(1);
  if (Math.abs(v) >= 1) return v.toFixed(3);
  return v.toFixed(4);
}

function fmtVal(v) {
  if (v === null || v === undefined) return '∅';
  if (typeof v === 'number') return fmtNum(v);
  if (typeof v === 'string') return v;
  if (Array.isArray(v)) return `[${v.length}]`;
  if (typeof v === 'object') return '{…}';
  return String(v);
}

function kvGroup(label, obj) {
  const g = document.createElement('span');
  g.className = 'group';
  const lab = document.createElement('span');
  lab.className = 'label';
  lab.textContent = label;
  g.appendChild(lab);
  for (const [k, v] of Object.entries(obj || {})) {
    const kv = document.createElement('span');
    kv.className = 'kv';
    const kk = document.createElement('span'); kk.className = 'k'; kk.textContent = k + ':';
    const vv = document.createElement('span'); vv.className = 'v'; vv.textContent = fmtVal(v);
    if (typeof v === 'object' && v !== null) vv.title = JSON.stringify(v, null, 2);
    kv.appendChild(kk); kv.appendChild(vv);
    g.appendChild(kv);
  }
  return g;
}

async function renderRuns() {
  const paths = selectedRunPaths();
  const summary = document.getElementById('runs-summary');
  const container = document.getElementById('runs');
  container.innerHTML = '';
  summary.textContent = paths.length === 0 ? 'no runs in current selection' : `${paths.length} run(s)`;

  const runs = await Promise.all(paths.map(loadRun));

  for (const run of runs) {
    const card = document.createElement('div');
    card.className = 'run-card';
    const path = document.createElement('div');
    path.className = 'path';
    const pill = document.createElement('span');
    pill.className = 'type-pill'; pill.textContent = run.type || '?';
    path.appendChild(pill);
    path.appendChild(document.createTextNode(' ' + run.path));
    card.appendChild(path);

    const row = document.createElement('div'); row.className = 'row';
    row.appendChild(kvGroup('config', run.config));
    row.appendChild(kvGroup('result', flattenResult(run.result)));
    card.appendChild(row);
    container.appendChild(card);
  }
  renderPlot(runs);
}

/** Drop bulky/array fields from result for the inline display. */
function flattenResult(result) {
  const out = {};
  for (const [k, v] of Object.entries(result || {})) {
    if (k === 'library' || k === 'extra') continue;
    out[k] = v;
  }
  return out;
}

// Downward-pointing equilateral triangle centered at origin with area `size`.
const TRIANGLE_DOWN = {
  draw(context, size) {
    const s = Math.sqrt(4 * size / Math.sqrt(3));
    const h = s * Math.sqrt(3) / 2;
    context.moveTo(0, h * 2 / 3);
    context.lineTo(s / 2, -h / 3);
    context.lineTo(-s / 2, -h / 3);
    context.closePath();
  },
};
// Diamond with equal width and height (Plot's default "diamond" is narrower).
const DIAMOND_SQUARE = {
  draw(context, size) {
    const r = Math.sqrt(size / 2);
    context.moveTo(0, -r);
    context.lineTo(r, 0);
    context.lineTo(0, r);
    context.lineTo(-r, 0);
    context.closePath();
  },
};
// Fixed domain → symbol map. We render one <dot> mark per domain with the
// symbol set to a constant, because Plot's symbol scale doesn't reliably
// honour mixed built-in + custom ranges on the dot mark itself (the scale
// renders fine in the legend though). Domain names match the values in
// config.json.
const DOMAIN_SYMBOLS = {
  'nuts-bolts': d3.symbolSquare,
  'dials': d3.symbolTriangle,
  'wheels': DIAMOND_SQUARE,
  'furniture': TRIANGLE_DOWN,
};

function renderPlot(runs) {
  const div = document.getElementById('graph-inner');
  div.innerHTML = '';

  const points = runs
    .map(r => ({
      path: r.path, type: r.type, domain: r.config?.domain ?? '?',
      algo: r.path.split('/').pop(),
      time: r.result?.elapsed_secs, ratio: r.result?.compression_ratio,
      config: r.config || {}, result: r.result || {},
    }))
    .filter(p => Number.isFinite(p.time) && p.time > 0 && Number.isFinite(p.ratio));

  // Geomean over reps for each (algo, domain) pair, drawn as larger overlaid
  // points. Skipped for groups with a single rep (mean would equal the point).
  const groups = new Map();
  for (const p of points) {
    const k = `${p.algo}\u0001${p.domain}`;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(p);
  }
  const means = [];
  for (const pts of groups.values()) {
    if (pts.length < 2) continue;
    const gmean = (xs) => Math.exp(xs.reduce((a, b) => a + Math.log(b), 0) / xs.length);
    const times = pts.map(p => p.time);
    const ratios = pts.map(p => p.ratio);
    means.push({
      algo: pts[0].algo, domain: pts[0].domain,
      time: gmean(times), ratio: gmean(ratios),
      timeMin: Math.min(...times), timeMax: Math.max(...times),
      ratioMin: Math.min(...ratios), ratioMax: Math.max(...ratios),
      n: pts.length,
      isMean: true,
    });
  }
  // Per-algo geomean across domains (geomean of the per-(algo, domain)
  // means). Drawn as circles with no error bars. Skipped for algos that
  // only appear in a single domain.
  const algoGroups = new Map();
  for (const m of means) {
    if (!algoGroups.has(m.algo)) algoGroups.set(m.algo, []);
    algoGroups.get(m.algo).push(m);
  }
  const algoMeans = [];
  for (const ms of algoGroups.values()) {
    if (ms.length < 2) continue;
    const gmean = (xs) => Math.exp(xs.reduce((a, b) => a + Math.log(b), 0) / xs.length);
    algoMeans.push({
      algo: ms[0].algo, domain: '*all*',  // falls through to `unknown` (circle)
      time: gmean(ms.map(m => m.time)),
      ratio: gmean(ms.map(m => m.ratio)),
      n: ms.length,
      isAlgoMean: true,
    });
  }

  // Means rendered last so they sit above the rep points; "geomean only"
  // checkbox suppresses the rep-level points entirely. Algo-means always
  // show on top.
  const hideReps = document.getElementById('hide-reps')?.checked;
  const allPoints = hideReps
    ? [...means, ...algoMeans]
    : [...points, ...means, ...algoMeans];

  if (points.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'empty';
    empty.textContent = 'no plottable runs (need elapsed_secs > 0 and compression_ratio)';
    div.appendChild(empty);
    return;
  }

  const size = Math.max(150, Math.min(div.clientWidth || 400, div.clientHeight || 400));
  const ratios = allPoints.map(p => p.ratio);
  const xMin = Math.min(1, Math.min(...ratios));
  const xMax = Math.max(...ratios);

  // Build one dot mark per domain with a constant symbol. Each mark also
  // tracks its point array (in render order) so the tooltip can find the
  // right datum for each rendered circle/path.
  const dotMarks = [];
  const dotData = [];
  const byDomain = new Map();
  for (const p of allPoints) {
    if (p.isAlgoMean) continue;
    if (!byDomain.has(p.domain)) byDomain.set(p.domain, []);
    byDomain.get(p.domain).push(p);
  }
  for (const [dom, pts] of byDomain) {
    const sym = DOMAIN_SYMBOLS[dom] ?? d3.symbolCircle;
    dotMarks.push(Plot.dot(pts, {
      x: 'ratio', y: 'time',
      stroke: 'algo', fill: 'algo',
      fillOpacity: d => d.isMean ? 0.95 : 0.4,
      r: d => d.isMean ? 9 : 4,
      strokeWidth: d => d.isMean ? 2 : 1,
      symbol: sym,
    }));
    dotData.push(pts);
  }
  if (algoMeans.length > 0) {
    dotMarks.push(Plot.dot(algoMeans, {
      x: 'ratio', y: 'time',
      stroke: 'algo', fill: 'algo',
      fillOpacity: 0.95, r: 11, strokeWidth: 2,
      symbol: d3.symbolCircle,
    }));
    dotData.push(algoMeans);
  }

  const plot = Plot.plot({
    width: size, height: size,
    marginLeft: 75, marginBottom: 60, marginTop: 12, marginRight: 12,
    x: { domain: [xMin, xMax], label: 'compression ratio', labelAnchor: 'center', labelArrow: false, grid: true },
    y: { type: 'log', label: 'time (s)', labelAnchor: 'center', labelArrow: false, grid: true },
    color: {
      legend: true,
      domain: ['enum', 'best-first', 'smc', 'babble'],
      range: ['#3b82f6', '#3b82f6', '#f59e0b', '#10b981'],
      unknown: '#6b7280',
    },
    marks: [
      // Vertical bar: time min↔max at the geomean ratio.
      Plot.ruleX(means, { x: 'ratio', y1: 'timeMin', y2: 'timeMax', stroke: 'algo', strokeOpacity: 0.6, strokeWidth: 1.5 }),
      // Horizontal bar: ratio min↔max at the geomean time.
      Plot.ruleY(means, { y: 'time', x1: 'ratioMin', x2: 'ratioMax', stroke: 'algo', strokeOpacity: 0.6, strokeWidth: 1.5 }),
      ...dotMarks,
    ],
  });

  // Render a matching symbol legend ourselves, since the dot marks use
  // constant symbols (no symbol scale). Renders reliably in every version.
  const symLegend = buildSymbolLegend([...byDomain.keys()]);
  if (symLegend) plot.prepend(symLegend);
  // Bump every text inside the plot svg (covers tick numbers); legend lives
  // in an outer div so it's untouched. Then override axis labels by exact
  // text match for a stronger emphasis.
  const labelText = new Set(['compression ratio', 'time (s)']);
  const svgs = plot.tagName === 'svg' ? [plot] : plot.querySelectorAll('svg');
  for (const svg of svgs) {
    for (const t of svg.querySelectorAll('text')) {
      t.style.setProperty('font-size', '12px', 'important');
    }
  }
  for (const t of plot.querySelectorAll('text')) {
    if (labelText.has(t.textContent.trim())) {
      t.style.setProperty('font-size', '17px', 'important');
      t.style.setProperty('font-weight', '600', 'important');
      t.style.setProperty('fill', '#1b1b1f', 'important');
    }
  }
  div.appendChild(plot);
  attachCustomTooltip(div, plot, dotData);
}

/** Render a simple inline symbol legend: SVG shapes next to domain labels.
 *  Skips domains not in DOMAIN_SYMBOLS. Returns null if nothing to show. */
function buildSymbolLegend(domains) {
  const known = domains.filter(d => DOMAIN_SYMBOLS[d]);
  if (known.length === 0) return null;
  const wrap = document.createElement('div');
  wrap.className = 'sym-legend';
  for (const dom of known) {
    const sym = DOMAIN_SYMBOLS[dom];
    const item = document.createElement('span');
    item.className = 'sym-legend-item';
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('width', '14'); svg.setAttribute('height', '14');
    svg.setAttribute('viewBox', '-8 -8 16 16');
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', d3.symbol(sym, 64)());
    path.setAttribute('fill', '#374151');
    svg.appendChild(path);
    item.appendChild(svg);
    const label = document.createElement('span');
    label.textContent = dom;
    item.appendChild(label);
    wrap.appendChild(item);
  }
  return wrap;
}

/** Wire hover handlers on every dot shape to a single floating tooltip.
 *  ``pointGroups`` is an array of arrays, one per dot mark in render order,
 *  matching the order of ``g[aria-label="dot"]`` groups in the SVG. */
function attachCustomTooltip(container, plot, pointGroups) {
  const tip = document.createElement('div');
  tip.className = 'plot-tip';
  container.appendChild(tip);

  const dotGroups = plot.querySelectorAll('g[aria-label="dot"]');
  if (dotGroups.length !== pointGroups.length) return;

  const place = (e) => {
    const rect = container.getBoundingClientRect();
    let x = e.clientX - rect.left + 10;
    let y = e.clientY - rect.top + 10;
    const tw = tip.offsetWidth, th = tip.offsetHeight;
    if (x + tw > rect.width) x = e.clientX - rect.left - tw - 10;
    if (y + th > rect.height) y = e.clientY - rect.top - th - 10;
    tip.style.left = Math.max(0, x) + 'px';
    tip.style.top = Math.max(0, y) + 'px';
  };

  dotGroups.forEach((g, gi) => {
    const pts = pointGroups[gi];
    const shapes = g.querySelectorAll('circle, path');
    if (shapes.length !== pts.length) return;
    shapes.forEach((shape, i) => {
      const p = pts[i];
      shape.addEventListener('mouseenter', (e) => {
        tip.innerHTML = renderTipContent(p);
        tip.classList.add('show');
        place(e);
      });
      shape.addEventListener('mousemove', place);
      shape.addEventListener('mouseleave', () => tip.classList.remove('show'));
    });
  });
}

function renderTipContent(p) {
  if (p.isAlgoMean) {
    const label = `${p.algo} (geomean across ${p.n} domains)`;
    const head = `
      <div class="head">
        <span class="type-pill">geomean</span>
        <span class="path" title="${escapeHtml(label)}">${escapeHtml(label)}</span>
      </div>
    `;
    return head + sectionRow('', { time: fmtNum(p.time) + 's', ratio: fmtNum(p.ratio) });
  }
  if (p.isMean) {
    const label = `${p.algo} / ${p.domain} (geomean of ${p.n} reps)`;
    const head = `
      <div class="head">
        <span class="type-pill">geomean</span>
        <span class="path" title="${escapeHtml(label)}">${escapeHtml(label)}</span>
      </div>
    `;
    return head + sectionRow('', { time: fmtNum(p.time) + 's', ratio: fmtNum(p.ratio) });
  }
  const head = `
    <div class="head">
      <span class="type-pill">${escapeHtml(p.type || '?')}</span>
      <span class="path" title="${escapeHtml(p.path)}">${escapeHtml(p.path)}</span>
    </div>
  `;
  const axes = sectionRow('', {
    time: fmtNum(p.time) + 's',
    ratio: fmtNum(p.ratio),
  });
  const cfg = sectionRow('config', p.config);
  const res = sectionRow('result', flattenResult(p.result));
  return head + axes + cfg + res;
}

function sectionRow(label, obj) {
  const entries = Object.entries(obj || {});
  if (entries.length === 0) return '';
  const lab = label ? `<span class="lab">${label}</span>` : '';
  const cells = entries.map(([k, v]) =>
    `<span class="kv"><span class="k">${escapeHtml(k)}:</span><span class="v">${escapeHtml(fmtVal(v))}</span></span>`
  ).join('');
  return `<div class="sect">${lab}${cells}</div>`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => (
    { '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]
  ));
}

// ---------- refresh ----------

async function refreshTree() {
  const root = await getJSON('/api/stackpath-tree');
  state.tree = root;
  // Drop any cached runs that no longer exist.
  const live = new Set();
  const walk = (n) => { if (n.is_run) live.add(n.path); for (const c of n.children) walk(c); };
  walk(root);
  for (const k of [...state.runs.keys()]) if (!live.has(k)) state.runs.delete(k);
  // Drop selected paths that no longer exist.
  const all = new Set();
  const walk2 = (n) => { all.add(n.path); for (const c of n.children) walk2(c); };
  walk2(root);
  for (const s of [...state.selected]) if (!all.has(s)) state.selected.delete(s);
  // Default selection: first top-level dir.
  if (state.selected.size === 0 && root.children.length > 0) {
    state.selected.add(root.children[0].path);
  }
  renderTree();
  renderRuns();
}

async function refreshSelections() {
  const { names } = await getJSON('/api/selections');
  state.selections = names;
  renderSelectionsList();
}

// ---------- init ----------

document.getElementById('refresh-btn').onclick = () => refreshTree();
document.getElementById('unsel-btn').onclick = () => {
  state.selected.clear();
  state.activeSelection = null;
  renderTree(); renderSelectionsList(); renderRuns();
};
document.getElementById('save-sel-btn').onclick = () => saveSelection();
document.getElementById('hide-reps').onchange = () => renderRuns();

window.addEventListener('resize', () => renderRuns());

(async () => {
  try {
    await Promise.all([refreshTree(), refreshSelections()]);
  } catch (err) {
    toast(`init failed: ${err.message}`, true);
  }
})();
