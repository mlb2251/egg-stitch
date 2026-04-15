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
  lastRuns: null,            // last set of runs passed to renderPlot (for axis re-draw)
  lastAutoPlotDomain: null,  // {xMin, xMax, yMin, yMax} computed from data (for freeze)
  plotConfig: {              // what gets mapped to color/shape/lightness, and active filters
    color: 'algo', shape: 'domain', lightness: null, filters: {},
  },
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
  state.lastRuns = runs;

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
// Named colors for known algo values; unknown values fall through to d3.schemeTableau10.
const BASE_COLORS = { enum: '#3b82f6', smc: '#f59e0b', babble: '#10b981', stitch: '#ef4444' };
const UNKNOWN_COLOR = '#6b7280';
// Symbol list for the shape dimension; values are assigned in sorted order by index.
// We render one Plot.dot mark per symbol because Plot's symbol scale doesn't
// reliably honour mixed built-in + custom symbols on the same dot mark.
const SYMBOLS = [
  d3.symbolSquare, d3.symbolTriangle, DIAMOND_SQUARE, TRIANGLE_DOWN,
  d3.symbolCircle, d3.symbolStar, d3.symbolCross,
];

let _plotConfigDimsKey = null;

/** Re-render the plot-config panel when dims change; skips if unchanged so
 *  filter inputs the user is editing are not clobbered. */
function renderPlotConfig(dims) {
  const key = dims.join(',');
  if (key === _plotConfigDimsKey) return;
  _plotConfigDimsKey = key;

  const body = document.getElementById('plot-config-body');
  if (!body) return;
  body.innerHTML = '';
  const cfg = state.plotConfig;

  for (const ch of ['color', 'shape', 'lightness']) {
    const row = document.createElement('div');
    row.className = 'cfg-row';
    const label = document.createElement('span');
    label.className = 'cfg-label'; label.textContent = ch;
    const sel = document.createElement('select');
    sel.className = 'cfg-sel';
    sel.appendChild(Object.assign(document.createElement('option'), { value: '', textContent: '—' }));
    for (const d of dims) {
      const opt = document.createElement('option');
      opt.value = d; opt.textContent = d;
      if (cfg[ch] === d) opt.selected = true;
      sel.appendChild(opt);
    }
    sel.onchange = () => {
      cfg[ch] = sel.value || null;
      _plotConfigDimsKey = null;
      if (state.lastRuns) renderPlot(state.lastRuns);
    };
    row.appendChild(label); row.appendChild(sel);
    body.appendChild(row);
  }

  body.appendChild(Object.assign(document.createElement('div'), { className: 'cfg-sep', textContent: 'filter' }));

  for (const d of dims) {
    const row = document.createElement('div');
    row.className = 'cfg-row';
    const label = document.createElement('span');
    label.className = 'cfg-label'; label.textContent = d;
    const inp = document.createElement('input');
    inp.type = 'text'; inp.className = 'cfg-filter'; inp.placeholder = 'any';
    inp.value = cfg.filters[d] ?? '';
    inp.onchange = () => {
      const v = inp.value.trim();
      if (v) cfg.filters[d] = v; else delete cfg.filters[d];
      if (state.lastRuns) renderPlot(state.lastRuns);
    };
    row.appendChild(label); row.appendChild(inp);
    body.appendChild(row);
  }
}

/** Group an array of points by their shape-dim value. */
function groupByShape(pts, shapeDim) {
  const map = new Map();
  for (const p of pts) {
    const k = shapeDim ? String(p[shapeDim] ?? '?') : '__all__';
    if (!map.has(k)) map.set(k, []);
    map.get(k).push(p);
  }
  return map;
}

/** Build a manual color+lightness legend (used when Plot's auto-legend isn't active). */
function buildColorLegend(colorDim, meanPoints, lightnessDim, lightnessVals, colorFn) {
  const colorVals = [...new Set(meanPoints.map(p => String(p[colorDim])))].sort();
  if (colorVals.length === 0) return null;
  const wrap = document.createElement('div');
  wrap.className = 'sym-legend';
  for (const cv of colorVals) {
    for (const lv of (lightnessDim ? lightnessVals : [null])) {
      const mockPt = { [colorDim]: cv };
      if (lv !== null) mockPt[lightnessDim] = lv;
      const color = colorFn(mockPt);
      const item = document.createElement('span');
      item.className = 'sym-legend-item';
      const swatch = document.createElement('span');
      swatch.style.cssText = `display:inline-block;width:12px;height:12px;background:${color};border-radius:2px;flex-shrink:0;`;
      item.appendChild(swatch);
      item.appendChild(Object.assign(document.createElement('span'), {
        textContent: lightnessDim ? `${cv}/${lv}` : cv,
      }));
      wrap.appendChild(item);
    }
  }
  return wrap;
}

function renderPlot(runs) {
  const div = document.getElementById('graph-inner');
  div.innerHTML = '';
  const cfg = state.plotConfig;

  // 1. Validate axes and extract all dims from each run.
  let rawPoints;
  try {
    rawPoints = runs.map(r => {
      const req = (v, name) => {
        if (v == null) throw new Error(`${r.path}: missing ${name}`);
        return v;
      };
      const time = req(r.result?.elapsed_secs, 'result.elapsed_secs');
      const ratio = req(r.result?.compression_ratio, 'result.compression_ratio');
      if (!Number.isFinite(time) || time <= 0) throw new Error(`${r.path}: elapsed_secs must be > 0, got ${time}`);
      if (!Number.isFinite(ratio)) throw new Error(`${r.path}: compression_ratio not finite, got ${ratio}`);
      const dims = {
        algo: req(r.result?.method, 'result.method'),
        domain: req(r.config?.domain, 'config.domain'),
      };
      for (const [k, v] of Object.entries(r.config || {})) {
        if (k !== 'domain') dims[k] = v;
      }
      return { path: r.path, type: r.type, time, ratio, dims, config: r.config, result: r.result };
    });
  } catch (err) {
    const el = document.createElement('div');
    el.className = 'empty'; el.style.color = 'var(--danger)'; el.textContent = err.message;
    div.appendChild(el); return;
  }

  // 2. Discover dims and update config panel (no-op if dims unchanged).
  const allDims = [...new Set(rawPoints.flatMap(p => Object.keys(p.dims)))].sort();
  renderPlotConfig(allDims);

  // 3. Apply filters (string comparison so numeric "2" matches 2).
  const filtered = rawPoints.filter(p =>
    Object.entries(cfg.filters).every(([d, v]) => String(p.dims[d] ?? '') === String(v))
  );
  if (filtered.length === 0) {
    div.appendChild(Object.assign(document.createElement('div'), {
      className: 'empty', textContent: 'no runs match current filters',
    }));
    return;
  }

  // 4. Geomean: group by visual-channel dims, average over everything else.
  const visDims = [cfg.color, cfg.shape, cfg.lightness].filter(Boolean);
  const gmean = xs => Math.exp(xs.reduce((s, x) => s + Math.log(x), 0) / xs.length);

  const groups = new Map();
  for (const p of filtered) {
    const k = visDims.map(d => String(p.dims[d])).join('\x01');
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(p);
  }
  const meanPoints = [...groups.values()].map(pts => {
    const dv = Object.fromEntries(visDims.map(d => [d, pts[0].dims[d]]));
    const ts = pts.map(p => p.time), rs = pts.map(p => p.ratio);
    return { ...dv, _isMean: true, n: pts.length, _rep: pts[0],
      time: gmean(ts), ratio: gmean(rs) };
  });

  // 5. Color scale. If lightness is active we compute colors manually; otherwise
  //    let Plot handle it so we get the auto-legend for free.
  const colorDim = cfg.color;
  const lightnessDim = cfg.lightness;
  const lightnessVals = lightnessDim
    ? [...new Set(meanPoints.map(p => String(p[lightnessDim])))].sort()
    : [];
  let colorFn = null, plotColorScale = null;
  if (!colorDim) {
    colorFn = () => UNKNOWN_COLOR;
  } else if (lightnessDim) {
    const fb = d3.scaleOrdinal(d3.schemeTableau10)
      .domain([...new Set(meanPoints.map(p => String(p[colorDim])))]);
    colorFn = p => {
      const base = BASE_COLORS[String(p[colorDim])] ?? fb(String(p[colorDim]));
      const idx = lightnessVals.indexOf(String(p[lightnessDim]));
      const frac = lightnessVals.length <= 1 ? 0.5 : idx / (lightnessVals.length - 1);
      const hsl = d3.hsl(base);
      hsl.l = 0.65 - frac * 0.35;
      return hsl.formatHex();
    };
  } else {
    const colorVals = [...new Set(meanPoints.map(p => String(p[colorDim])))].sort();
    plotColorScale = {
      legend: true,
      domain: colorVals,
      range: colorVals.map((v, i) => BASE_COLORS[v] ?? d3.schemeTableau10[i % 10]),
    };
  }

  // 6. Symbol scale. Values are sorted and assigned symbols by index.
  const shapeDim = cfg.shape;
  const shapeVals = shapeDim
    ? [...new Set(meanPoints.map(p => String(p[shapeDim])))].sort()
    : [];
  const symbolFor = val => {
    if (!shapeDim) return d3.symbolCircle;
    const idx = shapeVals.indexOf(String(val));
    if (idx < 0 || idx >= SYMBOLS.length) throw new Error(`no symbol for ${shapeDim}=${val}`);
    return SYMBOLS[idx];
  };

  // 7. Points to plot ("geomean only" suppresses the individual filtered points).
  const hideReps = document.getElementById('hide-reps')?.checked;
  const rawForPlot = hideReps ? [] : filtered.map(p => ({ ...p.dims, time: p.time, ratio: p.ratio, _raw: p }));
  const allPlotPoints = [...rawForPlot, ...meanPoints];

  // 8. Axis bounds.
  const allTimes = allPlotPoints.map(p => p.time).filter(t => Number.isFinite(t) && t > 0);
  const allRatios = allPlotPoints.map(p => p.ratio).filter(Number.isFinite);
  const axAuto = {
    xMin: Math.min(1, ...allRatios), xMax: Math.max(...allRatios),
    yMin: Math.min(...allTimes),     yMax: Math.max(...allTimes),
  };
  state.lastAutoPlotDomain = axAuto;
  const ov = getAxisOverrides();
  const xMin = ov.xMin ?? axAuto.xMin, xMax = ov.xMax ?? axAuto.xMax;
  const yMin = ov.yMin ?? axAuto.yMin, yMax = ov.yMax ?? axAuto.yMax;
  const size = Math.max(150, Math.min(div.clientWidth || 400, div.clientHeight || 400));

  // 9. Dot marks: one per shape value to support custom symbols.
  const dotMarks = [], dotData = [];
  const fillStroke = colorFn ?? colorDim;
  const addDots = (pts, r, opacity) => {
    for (const [sv, grp] of groupByShape(pts, shapeDim)) {
      dotMarks.push(Plot.dot(grp, {
        x: 'ratio', y: 'time',
        fill: fillStroke, stroke: fillStroke,
        fillOpacity: opacity, r, strokeWidth: r > 4 ? 2 : 1,
        symbol: symbolFor(sv),
      }));
      dotData.push(grp);
    }
  };
  if (rawForPlot.length > 0) addDots(rawForPlot, 3, 0.3);
  addDots(meanPoints, 9, 0.95);

  const plot = Plot.plot({
    width: size, height: size,
    marginLeft: 75, marginBottom: 60, marginTop: 12, marginRight: 12,
    x: { domain: [xMin, xMax], label: 'compression ratio', labelAnchor: 'center', labelArrow: false, grid: true },
    y: { type: 'log', domain: [yMin, yMax], label: 'time (s)', labelAnchor: 'center', labelArrow: false, grid: true },
    ...(plotColorScale ? { color: plotColorScale } : {}),
    marks: dotMarks,
  });

  // Style text.
  const labelText = new Set(['compression ratio', 'time (s)']);
  for (const svg of (plot.tagName === 'svg' ? [plot] : plot.querySelectorAll('svg'))) {
    for (const t of svg.querySelectorAll('text')) t.style.setProperty('font-size', '12px', 'important');
  }
  for (const t of plot.querySelectorAll('text')) {
    if (labelText.has(t.textContent.trim())) {
      t.style.setProperty('font-size', '17px', 'important');
      t.style.setProperty('font-weight', '600', 'important');
      t.style.setProperty('fill', '#1b1b1f', 'important');
    }
  }

  // Legends: symbol (always manual) + color (manual only when lightness is active).
  const symEntries = shapeVals.map(v => ({ sym: symbolFor(v), label: `${shapeDim}=${v}` }));
  const symLegend = buildSymbolLegend(symEntries);
  if (symLegend) plot.prepend(symLegend);
  if (colorFn && colorDim) {
    const cl = buildColorLegend(colorDim, meanPoints, lightnessDim, lightnessVals, colorFn);
    if (cl) plot.prepend(cl);
  }

  div.appendChild(plot);
  attachCustomTooltip(div, plot, dotData);
}

/** Render a simple inline symbol legend from an array of {sym, label} entries. */
function buildSymbolLegend(entries) {
  if (entries.length === 0) return null;
  const wrap = document.createElement('div');
  wrap.className = 'sym-legend';
  for (const { sym, label } of entries) {
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
    const text = document.createElement('span');
    text.textContent = label;
    item.appendChild(text);
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
  if (p._isMean) {
    const visDims = [state.plotConfig.color, state.plotConfig.shape, state.plotConfig.lightness].filter(Boolean);
    const label = visDims.map(d => `${d}=${p[d]}`).join(', ') + (p.n > 1 ? ` (n=${p.n})` : '');
    const head = `<div class="head"><span class="type-pill">mean</span><span class="path" title="${escapeHtml(label)}">${escapeHtml(label)}</span></div>`;
    return head + sectionRow('', { time: fmtNum(p.time) + 's', ratio: fmtNum(p.ratio) });
  }
  const raw = p._raw;
  const head = `<div class="head"><span class="type-pill">${escapeHtml(raw.type || '?')}</span><span class="path" title="${escapeHtml(raw.path)}">${escapeHtml(raw.path)}</span></div>`;
  return head
    + sectionRow('', { time: fmtNum(raw.time) + 's', ratio: fmtNum(raw.ratio) })
    + sectionRow('config', raw.config)
    + sectionRow('result', flattenResult(raw.result));
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

// ---------- axis overrides ----------

/** Read the four axis override inputs; returns null for any that are blank. */
function getAxisOverrides() {
  const val = (id) => {
    const v = parseFloat(document.getElementById(id)?.value);
    return Number.isFinite(v) ? v : null;
  };
  return { xMin: val('x-min'), xMax: val('x-max'), yMin: val('y-min'), yMax: val('y-max') };
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

// Axis override inputs: re-draw the plot instantly with cached runs.
for (const id of ['x-min', 'x-max', 'y-min', 'y-max']) {
  document.getElementById(id).addEventListener('change', () => {
    if (state.lastRuns) renderPlot(state.lastRuns);
  });
}

// Freeze: fill each input with the effective value currently in use.
document.getElementById('freeze-btn').onclick = () => {
  const d = state.lastAutoPlotDomain;
  if (!d) return;
  const ov = getAxisOverrides();
  document.getElementById('x-min').value = ov.xMin ?? d.xMin;
  document.getElementById('x-max').value = ov.xMax ?? d.xMax;
  document.getElementById('y-min').value = ov.yMin ?? d.yMin;
  document.getElementById('y-max').value = ov.yMax ?? d.yMax;
};

window.addEventListener('resize', () => renderRuns());

(async () => {
  try {
    await Promise.all([refreshTree(), refreshSelections()]);
  } catch (err) {
    toast(`init failed: ${err.message}`, true);
  }
})();
