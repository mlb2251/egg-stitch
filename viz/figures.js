// Figures & tables dashboard, served by viz/server.py (which serves the repo
// root, so /figures/ and /results/ are reachable and every response carries a
// Last-Modified header). Loads once when the page opens — there is no
// auto-refresh; reload the page to pick up newly rendered figures.
//
// On load it crawls /figures/ (recursively, parsing http.server's directory
// listing the same way analysis.js does) plus the top-level /results/*.json,
// HEADs each artifact for its Last-Modified time, and renders the PNGs inline
// and the .tex tables (raw LaTeX source, collapsible), grouped by experiment.
//
// Open via: make server, then http://localhost:<port>/viz/figures.html

const meta = document.getElementById('meta');
const container = document.getElementById('sections');

// Experiment sections in display order; the regex assigns each artifact path to
// one. Anything unmatched falls into an 'other' section appended at the end.
const SECTION_ORDER = ['table1', 'table2', 'table3', 'table4', 'table5', 'table7',
  'molecules', 'curve-grid', 'arity', 'ablation', 'other'];
const SECTION_RE = /^(curve-grid|molecules|arity|ablation|table\d+)/;

/** Raw LaTeX of each .tex artifact, prefetched at load so expands are instant. */
const texSrc = new Map();

/** Parse http.server's directory-listing HTML into { files, dirs } (names only). */
function extractLinks(html) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const files = [], dirs = [];
  for (const a of doc.querySelectorAll('a')) {
    const h = a.getAttribute('href');
    if (!h || h.startsWith('?') || h === '../' || h === '/') continue;
    if (h.endsWith('/')) dirs.push(decodeURIComponent(h.replace(/\/$/, '')));
    else files.push(decodeURIComponent(h));
  }
  return { files, dirs };
}

/** Recursively collect every file path under a server directory (absolute paths). */
async function walk(dir) {
  const html = await fetch(`${dir}/`).then(r => {
    if (!r.ok) throw new Error(`${dir}/ -> ${r.status}`);
    return r.text();
  });
  const { files, dirs } = extractLinks(html);
  let out = files.map(f => `${dir}/${f}`);
  for (const d of dirs) out = out.concat(await walk(`${dir}/${d}`));
  return out;
}

/** HEAD a path and return its Last-Modified as a Date (null if unavailable). */
async function mtime(path) {
  try {
    const r = await fetch(path, { method: 'HEAD' });
    const lm = r.headers.get('Last-Modified');
    return lm ? new Date(lm) : null;
  } catch { return null; }
}

/** The experiment section a figures-relative path belongs to. */
function sectionOf(rel) {
  const m = rel.match(SECTION_RE);
  return m ? m[1] : 'other';
}

/** Crawl figures/ + results/, fetch all timestamps and tex sources, then render. */
async function load() {
  try {
    const figPaths = await walk('/figures');
    const resListing = await fetch('/results/').then(r => r.text());
    const resFiles = extractLinks(resListing).files
      .filter(f => f.endsWith('.json'))
      .map(f => `/results/${f}`);

    // Timestamps for every artifact, and LaTeX source for every .tex, in parallel.
    const times = new Map();
    const texPaths = figPaths.filter(p => p.endsWith('.tex'));
    await Promise.all([
      ...[...figPaths, ...resFiles].map(async p => times.set(p, await mtime(p))),
      ...texPaths.map(async p =>
        texSrc.set(p, await fetch(p).then(r => r.text()).catch(() => '(failed to load)'))),
    ]);

    // Bucket artifacts into sections.
    const sections = new Map();
    const bucket = name => {
      if (!sections.has(name)) sections.set(name, { data: [], tables: [], figures: [] });
      return sections.get(name);
    };
    for (const p of figPaths) {
      const rel = p.slice('/figures/'.length);
      const s = bucket(sectionOf(rel));
      (p.endsWith('.tex') ? s.tables : s.figures).push(p);
    }
    for (const p of resFiles) {
      const rel = p.slice('/results/'.length).replace(/\.json$/, '');
      bucket(sectionOf(rel)).data.push(p);
    }

    const pngCount = figPaths.filter(p => p.endsWith('.png')).length;
    const allTimes = [...times.values()].filter(Boolean);
    const newest = allTimes.length ? new Date(Math.max(...allTimes.map(d => +d))) : null;
    meta.textContent =
      `${pngCount} figures · ${texPaths.length} tables · ${resFiles.length} data files`
      + (newest ? ` · newest write ${fmtAbs(newest)}` : '');

    render(sections, times);
  } catch (e) {
    meta.innerHTML = `<span class="err">failed to load figures: ${e}. Run `
      + `<code>make server</code> and open this page via `
      + `http://localhost:&lt;port&gt;/viz/figures.html</span>`;
  }
}

/** Render every non-empty section in the canonical order (unknowns last). */
function render(sections, times) {
  container.innerHTML = '';
  const names = SECTION_ORDER.filter(n => sections.has(n))
    .concat([...sections.keys()].filter(n => !SECTION_ORDER.includes(n)));
  for (const name of names) {
    container.appendChild(renderSection(name, sections.get(name), times));
  }
}

/** One experiment section: source-data links, .tex tables, then a PNG grid. */
function renderSection(name, sec, times) {
  const details = document.createElement('details');
  details.className = 'group';
  details.open = true;

  const all = [...sec.data, ...sec.tables, ...sec.figures];
  const ts = all.map(p => times.get(p)).filter(Boolean);
  const newest = ts.length ? new Date(Math.max(...ts.map(d => +d))) : null;

  const summary = document.createElement('summary');
  summary.innerHTML =
    `<span class="folder-name">${name}</span>`
    + `<span class="folder-count">${sec.figures.length} fig · ${sec.tables.length} tex · ${sec.data.length} data</span>`
    + (newest ? `<span class="when">updated ${fmtRel(newest)}</span>` : '');
  details.appendChild(summary);

  const body = document.createElement('div');
  body.className = 'section-body';
  const cmds = commandsFor(name);
  if (cmds.length) {
    const box = document.createElement('div');
    box.className = 'cmds';
    for (const [label, cmd] of cmds) box.appendChild(cmdRow(label, cmd));
    body.appendChild(box);
  }
  for (const p of sec.data) body.appendChild(dataRow(p, times.get(p)));
  for (const p of sec.tables) body.appendChild(texCard(p, times.get(p)));
  if (sec.figures.length) {
    const grid = document.createElement('div');
    grid.className = 'fig-grid';
    for (const p of sec.figures) grid.appendChild(figCard(p, times.get(p)));
    body.appendChild(grid);
  }
  details.appendChild(body);
  return details;
}

/** The rerun / delete-cache shell commands for a section.
 *
 * Returns [label, command] pairs. Delete commands target only OUR runners'
 * caches — the ``enum-*``/``smc-*`` files under ``results/tableN/`` (which
 * include our ``enum-baseline``/``enum-dsrs-at-start`` best-first baselines)
 * and ``enum_*`` under ``results/arity/`` — and never babble.json / stitch.json,
 * so re-running reuses those tools' cached numbers. Each "rerun ours" deletes
 * our cache first (else the run is a no-op cache hit), recomputes, then renders
 * so the figures on this page update. Ablation is an ours-only study, so its
 * whole per-table cache is ours. */
function commandsFor(name) {
  const table = (n, extraRender = '') => {
    const del = `rm -f results/${n}/enum-*.json results/${n}/smc-*.json`;
    return [
      ['rerun ours', `${del} && python3 -c 'from expts import *; ${n}()' && python3 scripts/render_tables.py${extraRender}`],
      ['delete ours cache', del],
    ];
  };
  if (/^table[123457]$/.test(name)) return table(name);
  if (name === 'molecules') {
    // The molecule figures derive from table5's data; recompute + re-render both.
    const c = table('table5', ' && python3 scripts/render_molecules.py');
    return [['rerun ours (table5)', c[0][1]], ['delete ours cache (table5)', c[1][1]]];
  }
  if (name === 'curve-grid') {
    // Render-only composite of the tables 3/4/5/7 geomeans; no cache of its own.
    return [['re-render (from tables 3/4/5/7)', 'python3 scripts/render_tables.py']];
  }
  if (name === 'arity') {
    const del = 'rm -f results/arity/enum_*.json';  // leaves stitch_*.json
    return [
      ['rerun ours', `${del} && python3 -c 'from expts import *; arity_experiment()' && python3 scripts/render_arity.py`],
      ['delete ours cache', del],
    ];
  }
  if (name === 'ablation') {
    const del = 'rm -rf results/ablation/table*';  // ours-only study
    return [
      ['rerun', `${del} && python3 -c 'from expts import *; ablation()' && python3 scripts/render_ablation.py`],
      ['delete cache', del],
    ];
  }
  return [];
}

/** A command row: a copy button, a label, and the command text. */
function cmdRow(label, cmd) {
  const row = document.createElement('div');
  row.className = 'cmd-row';
  const btn = document.createElement('button');
  btn.className = 'copy';
  btn.textContent = 'copy';
  btn.onclick = async () => {
    await copyText(cmd);
    btn.textContent = 'copied ✓';
    btn.classList.add('ok');
    setTimeout(() => { btn.textContent = 'copy'; btn.classList.remove('ok'); }, 1200);
  };
  const lab = document.createElement('span');
  lab.className = 'cmd-label';
  lab.textContent = label;
  const code = document.createElement('code');
  code.className = 'cmd-text';
  code.textContent = cmd;
  row.append(btn, lab, code);
  return row;
}

/** Copy text to the clipboard, falling back to a hidden textarea if needed. */
async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand('copy'); } finally { ta.remove(); }
  }
}

/** A single PNG figure card: the image (click to open full-size) + its mtime. */
function figCard(path, t) {
  const fig = document.createElement('figure');
  fig.className = 'fig-card';
  const name = path.split('/').pop();
  fig.innerHTML = `
    <a href="${path}" target="_blank"><img loading="lazy" src="${path}" alt="${esc(name)}"></a>
    <figcaption><span class="fname">${esc(name)}</span><span class="ftime">${t ? fmtAbs(t) : '—'}</span></figcaption>`;
  return fig;
}

/** A .tex table card: filename, mtime, a raw link, and collapsible LaTeX source. */
function texCard(path, t) {
  const el = document.createElement('div');
  el.className = 'tex-card card';
  const name = path.split('/').pop();
  el.innerHTML = `
    <div class="tex-head">
      <span class="fname">${esc(name)}</span>
      <span class="ftime">${t ? fmtAbs(t) : '—'}</span>
      <a href="${path}" target="_blank" class="raw-link">raw</a>
    </div>
    <details><summary>LaTeX source</summary><pre>${esc(texSrc.get(path) || '')}</pre></details>`;
  return el;
}

/** A source-data row: the results/*.json this section's figures derive from. */
function dataRow(path, t) {
  const el = document.createElement('div');
  el.className = 'data-row';
  const name = path.split('/').pop();
  el.innerHTML =
    `<span class="data-label">source</span>`
    + `<a href="${path}" target="_blank"><code>${esc(name)}</code></a>`
    + `<span class="ftime">${t ? fmtAbs(t) : '—'}</span>`;
  return el;
}

/** Absolute local date/time, e.g. "Jul 24, 26 · 11:43". */
function fmtAbs(d) {
  return d.toLocaleString([], {
    year: '2-digit', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

/** Coarse relative age, e.g. "3h ago". */
function fmtRel(d) {
  const s = (Date.now() - d.getTime()) / 1000;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

/** Minimal HTML escape for text inserted via innerHTML. */
function esc(s) { return String(s).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c])); }

load();
