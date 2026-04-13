// Fetch viz/results/**/*.json from viz/server.py (a tiny stdlib-only static
// server with DELETE support under viz/results/). The directory listing is
// parsed from http.server's auto-generated HTML index. Results are grouped by
// their containing subfolder under results/: the expts module drops each
// session into a timestamp-named folder, and we render one <details> section
// per folder, expanded by default. Each row and each folder summary has a x
// button that issues a DELETE to the server and re-loads on success.

import { parseDirectoryListing, escapeHtml } from './shared.js';

const meta = document.getElementById('meta');
const container = document.getElementById('groups');
let groups = [];           // [{ folder: string|'', rows: [...] }]
let sortKey = 'timestamp', sortAsc = false;

/** Fetch + parse one JSON result file, tagging it with its folder. */
async function loadRun(folder, file) {
  const path = folder ? `results/${folder}/${file}` : `results/${file}`;
  const r = await fetch(path).then(r => r.json());
  return {
    folder,
    name: file.replace(/\.json$/, ''),
    rewrites: !!r.rules_file,
    ...r,
  };
}

/** Fetch the results tree, load every JSON, group by folder, then render. */
// Exposed on window so batch.js can trigger a refresh after runs complete.
window.load = load;
async function load() {
  try {
    const listing = await fetch('results/').then(r => r.text());
    const { files: topFiles, dirs } = parseDirectoryListing(listing);

    const groupMap = new Map();            // folder -> rows[]
    const add = (folder, row) => {
      if (!groupMap.has(folder)) groupMap.set(folder, []);
      groupMap.get(folder).push(row);
    };

    const isRunFile = f => !f.endsWith('_debug.json') && !f.endsWith('_replay.json');

    // Top-level (ungrouped / legacy) runs.
    const topPromises = topFiles
      .filter(isRunFile)
      .map(f => loadRun('', f).then(r => add('', r)));

    // One pass per subfolder, in parallel.
    const subPromises = dirs.map(async d => {
      const sub = await fetch(`results/${d}/`).then(r => r.text());
      const { files } = parseDirectoryListing(sub);
      await Promise.all(
        files
          .filter(isRunFile)
          .map(f => loadRun(d, f).then(r => add(d, r)))
      );
    });

    await Promise.all([...topPromises, ...subPromises]);

    groups = [...groupMap.entries()].map(([folder, rows]) => ({ folder, rows }));
    // Newest folder first; ungrouped ('') pinned to the bottom.
    groups.sort((a, b) => {
      if (a.folder === '' && b.folder !== '') return 1;
      if (b.folder === '' && a.folder !== '') return -1;
      return a.folder < b.folder ? 1 : a.folder > b.folder ? -1 : 0;
    });

    const total = groups.reduce((n, g) => n + g.rows.length, 0);
    meta.textContent = `${total} runs across ${groups.length} folder${groups.length === 1 ? '' : 's'}`;
    render();
  } catch (e) {
    meta.innerHTML = `<span class="err">failed to load results: ${e}. run <code>make server</code> and open this page via http://localhost:&lt;port&gt;/viz/</span>`;
  }
}

const COLUMNS = [
  [null, ''],
  ['timestamp', 'when'],
  ['name', 'run'],
  [null, 'links'],
  ['rewrites', 'dsr'],
  ['final_cost', 'cost'],
  ['compression_ratio', 'ratio'],
  ['elapsed_secs', 'time (s)'],
  ['arity', 'arity'],
  ['num_matches', 'matches'],
  ['num_expansions', 'expansions'],
];

/** DELETE a path under results/ and reload on success. */
async function deletePath(path, label) {
  if (!confirm(`Delete ${label}? This can't be undone.`)) return false;
  const res = await fetch(path, { method: 'DELETE' });
  if (!res.ok) { alert(`delete failed (${res.status}): ${await res.text()}`); return false; }
  await load();
  return true;
}

/** Delete a single run JSON (and its _debug.json sibling if present). */
async function deleteRun(r) {
  const base = r.folder ? `results/${r.folder}/${r.name}` : `results/${r.name}`;
  if (!confirm(`Delete run "${r.name}"${r.folder ? ` in ${r.folder}` : ''}?`)) return;
  const res = await fetch(`${base}.json`, { method: 'DELETE' });
  if (!res.ok) { alert(`delete failed (${res.status}): ${await res.text()}`); return; }
  if (r.debug_log_file) {
    // Best-effort: ignore 404 if there's no debug file.
    await fetch(r.folder ? `results/${r.folder}/${r.debug_log_file}` : `results/${r.debug_log_file}`, { method: 'DELETE' });
  }
  if (r.replay_log_file) {
    await fetch(r.folder ? `results/${r.folder}/${r.replay_log_file}` : `results/${r.replay_log_file}`, { method: 'DELETE' });
  }
  await load();
}

/** Delete an entire session folder. */
async function deleteFolder(folder) {
  if (!folder) return;
  await deletePath(`results/${folder}`, `folder "${folder}" and all runs inside`);
}

/** Render one folder group as an expanded <details> with its own table. */
function renderGroup(g, maxRatio) {
  const rows = [...g.rows].sort((a, b) => {
    const x = a[sortKey], y = b[sortKey];
    if (x === y) return 0;
    const cmp = x < y ? -1 : 1;
    return sortAsc ? cmp : -cmp;
  });

  const details = document.createElement('details');
  details.className = 'group';
  details.open = true;

  const summary = document.createElement('summary');
  const delFolderBtn = g.folder
    ? `<button class="del del-folder" title="delete folder">×</button>`
    : '';
  summary.innerHTML = `<span class="folder-name">${g.folder || '(ungrouped)'}</span> <span class="folder-count">${rows.length} run${rows.length === 1 ? '' : 's'}</span>${delFolderBtn}`;
  const folderBtn = summary.querySelector('.del-folder');
  if (folderBtn) folderBtn.onclick = (e) => { e.preventDefault(); e.stopPropagation(); deleteFolder(g.folder); };
  details.appendChild(summary);

  const table = document.createElement('table');
  const thead = document.createElement('thead');
  const headTr = document.createElement('tr');
  for (const [k, label, cls] of COLUMNS) {
    const th = document.createElement('th');
    th.textContent = label;
    if (cls) th.classList.add(cls);
    if (k) {
      th.dataset.k = k;
      th.onclick = () => {
        if (sortKey === k) sortAsc = !sortAsc;
        else { sortKey = k; sortAsc = true; }
        render();
      };
    }
    headTr.appendChild(th);
  }
  thead.appendChild(headTr);
  table.appendChild(thead);

  const tbody = document.createElement('tbody');
  for (const r of rows) {
    const tr = document.createElement('tr');
    tr.className = 'run';
    const barW = Math.round(60 * (r.compression_ratio || 0) / maxRatio);
    const replayPath = r.replay_log_file
      ? (r.folder ? `${r.folder}/${r.replay_log_file}` : r.replay_log_file)
      : null;
    // Extract domain name from input_file (e.g. "data/domains/cogsci/dials.json" -> "dials")
    const domain = r.input_file ? r.input_file.replace(/.*\//, '').replace(/\.json$/, '') : '';
    // Extract rules file basename (e.g. ".../drawings.dials.rewrites" -> "drawings.dials.rewrites")
    const rules = r.rules_file ? r.rules_file.replace(/.*\//, '') : '';
    const replayParams = replayPath ? `replay=${encodeURIComponent(replayPath)}&domain=${encodeURIComponent(domain)}&rules=${encodeURIComponent(rules)}` : '';
    const configParams = replayPath ? `config=${encodeURIComponent(replayPath)}&domain=${encodeURIComponent(domain)}&rules=${encodeURIComponent(rules)}` : '';
    const replayLink = replayPath ? `<a class="debug-link" href="interactive.html?${replayParams}" onclick="event.stopPropagation()">replay</a>` : '';
    const configLink = replayPath ? `<a class="debug-link" href="interactive.html?${configParams}" onclick="event.stopPropagation()">config</a>` : '';
    tr.innerHTML = `
      <td><button class="del del-run" title="delete run">×</button></td>
      <td>${fmtTime(r.timestamp)}</td>
      <td><b>${r.name}</b></td>
      <td style="display:flex;gap:.3rem;align-items:center">${replayLink}${configLink}</td>
      <td>${r.rewrites ? '<span class="pill">yes</span>' : '<span class="pill no">no</span>'}</td>
      <td>${fmt(r.final_cost)}</td>
      <td><span class="ratio">${(r.compression_ratio||0).toFixed(3)}×</span><span class="bar" style="width:${barW}px"></span></td>
      <td>${(r.elapsed_secs||0).toFixed(2)}</td>
      <td>${r.arity ?? ''}</td>
      <td>${fmt(r.num_matches)}</td>
      <td>${fmt(r.num_expansions)}</td>
    `;
    tr.querySelector('.del-run').onclick = (e) => { e.stopPropagation(); deleteRun(r); };
    tr.onclick = () => toggleDetail(tr, r);
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  details.appendChild(table);
  return details;
}

/** Render all groups. */
function render() {
  const allRows = groups.flatMap(g => g.rows);
  const maxRatio = Math.max(1, ...allRows.map(r => r.compression_ratio || 0));
  container.innerHTML = '';
  for (const g of groups) container.appendChild(renderGroup(g, maxRatio));
}

/** Format numbers with thousands separators; pass through null/undefined. */
function fmt(n) { return n == null ? '' : typeof n === 'number' ? n.toLocaleString() : n; }

/** Format a unix-epoch-seconds timestamp as a short local date/time. */
function fmtTime(ts) {
  if (ts == null) return '';
  const d = new Date(ts * 1000);
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** Toggle an inline detail row immediately below the clicked run row. */
function toggleDetail(tr, r) {
  const next = tr.nextElementSibling;
  if (next && next.classList.contains('detail-row')) {
    next.remove();
    tr.classList.remove('expanded');
    return;
  }
  const progs = r.rewritten_programs || [];
  const detailTr = document.createElement('tr');
  detailTr.className = 'detail-row';
  const td = document.createElement('td');
  td.colSpan = COLUMNS.length;
  td.innerHTML = `
    <div class="card">
      <div class="kv">
        <span>input</span><b>${r.input_file || ''}</b>
        <span>rules</span><b>${r.rules_file || '—'}</b>
        <span>steps run</span><b>${r.num_steps_run ?? ''}</b>
      </div>
      <details open><summary>best pattern</summary><pre>${escapeHtml(r.pattern || '')}</pre></details>
      <details><summary>${progs.length} rewritten programs</summary><pre>${escapeHtml(progs.join('\n'))}</pre></details>
    </div>
  `;
  detailTr.appendChild(td);
  tr.parentNode.insertBefore(detailTr, tr.nextSibling);
  tr.classList.add('expanded');
}

load();
