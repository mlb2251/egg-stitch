// Fetch viz/results/*.json from a running `python3 -m http.server` and render.
// Directory listing is parsed from http.server's auto-generated HTML index.

const meta = document.getElementById('meta');
const tbody = document.querySelector('#tbl tbody');
let rows = [];
let sortKey = 'timestamp', sortAsc = false;

/** Parse http.server's directory listing HTML and return hrefs ending in .json. */
function extractJsonLinks(html) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  return [...doc.querySelectorAll('a')]
    .map(a => a.getAttribute('href'))
    .filter(h => h && h.endsWith('.json'));
}

/** Fetch the results directory, load every JSON file, and kick off rendering. */
async function load() {
  try {
    const listing = await fetch('results/').then(r => r.text());
    const files = extractJsonLinks(listing);
    const loaded = await Promise.all(files.map(async f => {
      const r = await fetch('results/' + f).then(r => r.json());
      return { name: f.replace(/\.json$/, ''), rewrites: !!r.rules_file, ...r };
    }));
    rows = loaded;
    meta.textContent = `${rows.length} runs loaded`;
    render();
  } catch (e) {
    meta.innerHTML = `<span class="err">failed to load results: ${e}. run <code>make server</code> and open this page via http://localhost:&lt;port&gt;/viz/</span>`;
  }
}

/** Render the sorted table body. */
function render() {
  const maxRatio = Math.max(1, ...rows.map(r => r.compression_ratio || 0));
  rows.sort((a, b) => {
    const x = a[sortKey], y = b[sortKey];
    if (x === y) return 0;
    const cmp = x < y ? -1 : 1;
    return sortAsc ? cmp : -cmp;
  });
  tbody.innerHTML = '';
  for (const r of rows) {
    const tr = document.createElement('tr');
    tr.className = 'run';
    const barW = Math.round(60 * (r.compression_ratio || 0) / maxRatio);
    tr.innerHTML = `
      <td>${fmtTime(r.timestamp)}</td>
      <td><b>${r.name}</b></td>
      <td>${r.rewrites ? '<span class="pill">yes</span>' : '<span class="pill no">no</span>'}</td>
      <td>${fmt(r.initial_cost)}</td>
      <td>${fmt(r.final_cost)}</td>
      <td><span class="ratio">${(r.compression_ratio||0).toFixed(3)}×</span><span class="bar" style="width:${barW}px"></span></td>
      <td>${(r.elapsed_secs||0).toFixed(2)}</td>
      <td>${r.arity ?? ''}</td>
      <td>${r.pattern_size ?? ''}</td>
      <td>${fmt(r.num_matches)}</td>
      <td>${fmt(r.usage_matches)}</td>
      <td>${fmt(r.approx_cost)}</td>
      <td>${fmt(r.num_expansions)}</td>
      <td>${r.best_iteration ?? ''}</td>
      <td>${r.debug_log_file ? `<a class="debug-link" href="debug.html?file=${encodeURIComponent(r.debug_log_file)}" onclick="event.stopPropagation()">view</a>` : ''}</td>
    `;
    tr.onclick = () => showDetail(r);
    tbody.appendChild(tr);
  }
}

/** Format numbers with thousands separators; pass through null/undefined. */
function fmt(n) { return n == null ? '' : typeof n === 'number' ? n.toLocaleString() : n; }

/** Format a unix-epoch-seconds timestamp as a short local date/time. */
function fmtTime(ts) {
  if (ts == null) return '';
  const d = new Date(ts * 1000);
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

document.querySelectorAll('#tbl th').forEach(th => {
  th.onclick = () => {
    const k = th.dataset.k;
    if (sortKey === k) sortAsc = !sortAsc;
    else { sortKey = k; sortAsc = true; }
    render();
  };
});

/** Render the per-run detail card below the table. */
function showDetail(r) {
  const d = document.getElementById('detail');
  const progs = r.rewritten_programs || [];
  d.innerHTML = `
    <div class="card">
      <h3>${r.name}</h3>
      <div class="kv">
        <span>input</span><b>${r.input_file || ''}</b>
        <span>rules</span><b>${r.rules_file || '—'}</b>
        <span>steps run</span><b>${r.num_steps_run ?? ''}</b>
      </div>
      <details open><summary>best pattern</summary><pre>${esc(r.pattern || '')}</pre></details>
      <details><summary>${progs.length} rewritten programs</summary><pre>${esc(progs.join('\n'))}</pre></details>
    </div>
  `;
  d.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
}

/** Minimal HTML escape for untrusted text inserted via innerHTML. */
function esc(s) { return String(s).replace(/[&<>]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;'}[c])); }

load();
