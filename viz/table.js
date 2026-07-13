"use strict";

// Each page sets window.TABLE_KIND to "table1" or "table2" before loading this script.
const KIND = window.TABLE_KIND || "table1";
const TITLE = KIND === "table2" ? "Table 2" : "Table 1";
const RESULTS_PREFIX = `/viz/results/${KIND}`;
// Table 2 runs without DSRs, so there's no "E-graph min" (cost_after_rewrites) column.
const SHOW_EGRAPH_MIN = KIND !== "table2";

const DOMAIN_ORDER = ["nuts-bolts", "dials", "wheels", "furniture"];
const DOMAIN_LABELS = {
  "nuts-bolts": "Nuts & Bolts",
  "dials": "Dials",
  "wheels": "Wheels",
  "furniture": "Furniture",
};
const METHODS = ["enum", "smc", "babble", "stitch"];
const METHOD_LABELS = { enum: "Enum", smc: "SMC", babble: "babble", stitch: "Stitch" };

const state = {
  runs: [],        // list of timestamp strings, newest first
  data: {},        // timestamp -> loaded JSON
  active: null,    // primary selected timestamp (the A side in compare)
  compare: null,   // optional B side; when set, view is in compare mode
};

/** Fetch the directory listing for the table's results folder and extract timestamps. */
async function listRuns() {
  const res = await fetch(`${RESULTS_PREFIX}/`);
  if (!res.ok) return [];
  const html = await res.text();
  const names = [];
  const re = /href="([^"/]+)\/"/g;
  let m;
  while ((m = re.exec(html)) !== null) names.push(m[1]);
  names.sort().reverse();
  return names;
}

/** Lazy-load a run's JSON, caching the result. */
async function loadRun(ts) {
  if (state.data[ts]) return state.data[ts];
  const res = await fetch(`${RESULTS_PREFIX}/${ts}/${KIND}.json`);
  if (!res.ok) throw new Error(`${ts}: HTTP ${res.status}`);
  const json = await res.json();
  state.data[ts] = json;
  return json;
}

/** Build a method->result map for a domain, filling in missing methods with null. */
function methodMap(domainEntry) {
  const out = {};
  for (const r of domainEntry.results) out[r.method] = r;
  return out;
}

function fmt(x, digits) {
  return x == null || Number.isNaN(x) ? null : x.toFixed(digits);
}

/** Geometric mean of an array, ignoring null/NaN/non-positive values. */
function geoMean(vals) {
  const pos = vals.filter(v => v != null && !Number.isNaN(v) && v > 0);
  if (pos.length === 0) return null;
  return Math.exp(pos.reduce((s, v) => s + Math.log(v), 0) / pos.length);
}

/** Build a single-run table. */
function renderSolo(ts, data) {
  const domains = data.domains;
  const rows = DOMAIN_ORDER.filter(d => d in domains).map(d => {
    const entry = domains[d];
    const by = methodMap(entry);
    const cr = m => (by[m] ? by[m].compression_ratio : null);
    const tm = m => (by[m] ? by[m].elapsed_secs : null);
    const fc = m => (by[m] ? by[m].final_cost : null);
    const lib = m => (by[m] ? (by[m].library || []) : []);
    const init = (by.enum || Object.values(by)[0] || {}).initial_cost ?? null;
    return { d, init, egMin: entry.egraph_min_size ?? null, cr, tm, fc, lib };
  });

  const cell = v => v == null ? '<span class="na">—</span>' : v;
  /** Index of the method with the winning value, or -1 if none. */
  const bestIdx = (vals, prefer) => {
    let best = -1, bestV = null;
    vals.forEach((v, i) => {
      if (v == null) return;
      if (bestV == null || (prefer === "max" ? v > bestV : v < bestV)) { best = i; bestV = v; }
    });
    return best;
  };

  const gmCr = METHODS.map(m => geoMean(rows.map(r => r.cr(m))));
  const gmT  = METHODS.map(m => geoMean(rows.map(r => r.tm(m))));

  const body = rows.map(r => {
    const crVals = METHODS.map(m => r.cr(m));
    const tVals = METHODS.map(m => r.tm(m));
    const bestCr = bestIdx(crVals, "max");
    const bestT = bestIdx(tVals, "min");
    const metric = (vals, digits, bestI) => (i) => {
      const cls = [i === 0 ? "group-left" : "", i === bestI ? "best" : ""].filter(Boolean).join(" ");
      return `<td class="${cls}">${cell(fmt(vals[i], digits))}</td>`;
    };
    const crCell = metric(crVals, 2, bestCr);
    const tCell = metric(tVals, 1, bestT);
    return `
      <tr>
        <td class="domain">${DOMAIN_LABELS[r.d] ?? r.d}</td>
        <td>${cell(r.init)}</td>
        ${SHOW_EGRAPH_MIN ? `<td>${cell(r.egMin)}</td>` : ""}
        ${METHODS.map((_, i) => crCell(i)).join("")}
        ${METHODS.map((_, i) => tCell(i)).join("")}
      </tr>
    `;
  }).join("");

  const bestGmCr = bestIdx(gmCr, "max");
  const bestGmT  = bestIdx(gmT, "min");
  const gmRow = `
    <tr class="geo-mean">
      <td class="domain">Geo. mean</td>
      <td></td>
      ${SHOW_EGRAPH_MIN ? "<td></td>" : ""}
      ${gmCr.map((v, i) => {
        const cls = [i === 0 ? "group-left" : "", i === bestGmCr ? "best" : ""].filter(Boolean).join(" ");
        return `<td class="${cls}">${cell(fmt(v, 2))}</td>`;
      }).join("")}
      ${gmT.map((v, i) => {
        const cls = [i === 0 ? "group-left" : "", i === bestGmT ? "best" : ""].filter(Boolean).join(" ");
        return `<td class="${cls}">${cell(fmt(v, 1))}</td>`;
      }).join("")}
    </tr>
  `;

  const libs = renderLibraries(rows);

  return `
    <h1>${TITLE} <span class="timestamp">· ${ts}</span></h1>
    ${renderConfig(data.config)}
    <table class="t1">
      <thead>
        <tr>
          <th class="spacer"></th>
          <th class="spacer"></th>
          ${SHOW_EGRAPH_MIN ? '<th class="spacer"></th>' : ""}
          <th colspan="4">Compression Ratio</th>
          <th colspan="4">Time (s)</th>
        </tr>
        <tr>
          <th class="domain">Domain</th>
          <th>Original size</th>
          ${SHOW_EGRAPH_MIN ? "<th>E-graph min</th>" : ""}
          ${METHODS.map((m, i) => `<th class="${i === 0 ? 'group-left' : ''}">${METHOD_LABELS[m]}</th>`).join("")}
          ${METHODS.map((m, i) => `<th class="${i === 0 ? 'group-left' : ''}">${METHOD_LABELS[m]}</th>`).join("")}
        </tr>
      </thead>
      <tbody>${body}${gmRow}</tbody>
    </table>
    ${libs}
  `;
}

/** Escape a string for safe insertion into HTML text content. */
function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

/** Render a library section grouped by domain, with per-method entries. */
function renderLibraries(rows) {
  const blocks = rows.map(r => {
    const parts = METHODS.map(m => {
      const lib = r.lib(m);
      if (!lib || lib.length === 0) return null;
      const label = `<span class="lib-method">${METHOD_LABELS[m]}</span>`;
      if (lib.length === 1) {
        return `<div class="lib-row"><div class="lib-hdr">${label}</div><pre class="lib">${esc(lib[0])}</pre></div>`;
      }
      const items = lib.map((s, i) => `<li>${esc(s)}</li>`).join("");
      return `<div class="lib-row"><div class="lib-hdr">${label}<span class="lib-count">${lib.length} functions</span></div><ol class="lib-list">${items}</ol></div>`;
    }).filter(Boolean).join("");
    if (!parts) return "";
    return `
      <section class="lib-block">
        <h3>${DOMAIN_LABELS[r.d] ?? r.d}</h3>
        ${parts}
      </section>
    `;
  }).filter(Boolean).join("");
  if (!blocks) return "";
  return `<div class="libs"><h2 class="libs-title">Libraries</h2>${blocks}</div>`;
}

function renderConfig(cfg) {
  if (!cfg) return "";
  const smc = cfg.smc || {};
  const enm = cfg.enum || {};
  return `
    <div class="config-bar">
      <span><b>Enum</b> · num_steps=${enm.num_steps ?? "?"}</span>
      <span><b>SMC</b> · num_steps=${smc.num_steps ?? "?"} · num_particles=${smc.num_particles ?? "?"} · T=${smc.temperature ?? "?"}</span>
    </div>
  `;
}

/** Format a delta pill. `higherIsBetter` controls red/green direction. */
function deltaPill(a, b, digits, higherIsBetter) {
  if (a == null || b == null || Number.isNaN(a) || Number.isNaN(b)) return "";
  const d = b - a;
  if (Math.abs(d) < Math.pow(10, -digits) / 2) {
    return `<span class="delta flat">±0</span>`;
  }
  const good = higherIsBetter ? d > 0 : d < 0;
  const cls = good ? "good" : "bad";
  const arrow = d > 0 ? "▲" : "▼";
  return `<span class="delta ${cls}">${arrow}${Math.abs(d).toFixed(digits)}</span>`;
}

/** Render a cell that pairs two values (a above, b below) with a delta pill. */
function pairCell(a, b, digits, higherIsBetter) {
  const na = '<span class="na">—</span>';
  const af = a == null ? na : a.toFixed(digits);
  const bf = b == null ? na : b.toFixed(digits);
  return `
    <div class="pair">
      <span class="a">${af}</span>
      <span class="b">${bf} ${deltaPill(a, b, digits, higherIsBetter)}</span>
    </div>
  `;
}

function renderCompare(tsA, tsB, dataA, dataB) {
  const rows = DOMAIN_ORDER.filter(d => d in dataA.domains || d in dataB.domains).map(d => {
    const a = dataA.domains[d] || { results: [] };
    const b = dataB.domains[d] || { results: [] };
    const byA = methodMap(a);
    const byB = methodMap(b);
    return {
      d,
      initA: (byA.enum || Object.values(byA)[0] || {}).initial_cost ?? null,
      initB: (byB.enum || Object.values(byB)[0] || {}).initial_cost ?? null,
      egA: a.egraph_min_size ?? null,
      egB: b.egraph_min_size ?? null,
      crA: m => (byA[m] ? byA[m].compression_ratio : null),
      crB: m => (byB[m] ? byB[m].compression_ratio : null),
      tA: m => (byA[m] ? byA[m].elapsed_secs : null),
      tB: m => (byB[m] ? byB[m].elapsed_secs : null),
    };
  });

  const intCell = (a, b) => {
    if (a == null && b == null) return '<span class="na">—</span>';
    if (a === b) return a ?? b;
    return `<div class="pair"><span class="a">${a ?? "—"}</span><span class="b">${b ?? "—"}</span></div>`;
  };

  const body = rows.map(r => `
    <tr>
      <td class="domain">${DOMAIN_LABELS[r.d] ?? r.d}</td>
      <td>${intCell(r.initA, r.initB)}</td>
      ${SHOW_EGRAPH_MIN ? `<td>${intCell(r.egA, r.egB)}</td>` : ""}
      ${METHODS.map((m, i) => `<td class="${i === 0 ? 'group-left' : ''}">${pairCell(r.crA(m), r.crB(m), 2, true)}</td>`).join("")}
      ${METHODS.map((m, i) => `<td class="${i === 0 ? 'group-left' : ''}">${pairCell(r.tA(m), r.tB(m), 1, false)}</td>`).join("")}
    </tr>
  `).join("");

  const gmCrA = METHODS.map(m => geoMean(rows.map(r => r.crA(m))));
  const gmCrB = METHODS.map(m => geoMean(rows.map(r => r.crB(m))));
  const gmTA  = METHODS.map(m => geoMean(rows.map(r => r.tA(m))));
  const gmTB  = METHODS.map(m => geoMean(rows.map(r => r.tB(m))));
  const gmRow = `
    <tr class="geo-mean">
      <td class="domain">Geo. mean</td>
      <td></td>
      ${SHOW_EGRAPH_MIN ? "<td></td>" : ""}
      ${METHODS.map((m, i) => `<td class="${i === 0 ? 'group-left' : ''}">${pairCell(gmCrA[i], gmCrB[i], 2, true)}</td>`).join("")}
      ${METHODS.map((m, i) => `<td class="${i === 0 ? 'group-left' : ''}">${pairCell(gmTA[i], gmTB[i], 1, false)}</td>`).join("")}
    </tr>
  `;

  return `
    <h1>${TITLE} · compare</h1>
    <div class="compare-hdr">
      <span class="tag a">A · ${tsA}</span>
      <span class="muted">→</span>
      <span class="tag b">B · ${tsB}</span>
      <span class="muted">· green = B is better, red = B is worse</span>
    </div>
    <table class="t1">
      <thead>
        <tr>
          <th class="spacer"></th>
          <th class="spacer"></th>
          ${SHOW_EGRAPH_MIN ? '<th class="spacer"></th>' : ""}
          <th colspan="4">Compression Ratio (higher is better)</th>
          <th colspan="4">Time (s) (lower is better)</th>
        </tr>
        <tr>
          <th class="domain">Domain</th>
          <th>Original size</th>
          ${SHOW_EGRAPH_MIN ? "<th>E-graph min</th>" : ""}
          ${METHODS.map((m, i) => `<th class="${i === 0 ? 'group-left' : ''}">${METHOD_LABELS[m]}</th>`).join("")}
          ${METHODS.map((m, i) => `<th class="${i === 0 ? 'group-left' : ''}">${METHOD_LABELS[m]}</th>`).join("")}
        </tr>
      </thead>
      <tbody>${body}${gmRow}</tbody>
    </table>
  `;
}

function renderSidebar() {
  const ul = document.getElementById("runs");
  ul.innerHTML = state.runs.map(ts => {
    const isA = ts === state.active;
    const isB = ts === state.compare;
    const cls = isA ? "compare-a" : isB ? "compare-b" : "";
    // Show a compare button on non-primary rows when a primary is selected.
    // On the B row itself (in compare mode), show an × to exit compare.
    let action = "";
    if (isB) {
      action = `<button class="cmp-btn exit" data-ts="${ts}" title="Exit compare">×</button>`;
    } else if (!isA && state.active) {
      action = `<button class="cmp-btn" data-ts="${ts}" title="Compare with ${state.active}">⇆</button>`;
    }
    return `
      <li class="${cls}" data-ts="${ts}">
        <span class="ts">${ts}</span>
        ${action}
      </li>
    `;
  }).join("");

  ul.querySelectorAll("li").forEach(li => {
    const ts = li.dataset.ts;
    li.addEventListener("click", () => {
      state.active = ts;
      state.compare = null;
      render();
    });
  });

  ul.querySelectorAll(".cmp-btn").forEach(btn => {
    btn.addEventListener("click", e => {
      e.stopPropagation();
      const ts = btn.dataset.ts;
      state.compare = btn.classList.contains("exit") ? null : ts;
      render();
    });
  });
}

async function render() {
  renderSidebar();
  const main = document.getElementById("main");
  try {
    if (state.active && state.compare) {
      const [dataA, dataB] = await Promise.all([loadRun(state.active), loadRun(state.compare)]);
      main.innerHTML = renderCompare(state.active, state.compare, dataA, dataB);
    } else {
      const ts = state.active ?? state.runs[0];
      if (!ts) { main.innerHTML = `<div class="empty">No ${KIND} runs yet. Run <code>./run.py ${KIND}</code>.</div>`; return; }
      const data = await loadRun(ts);
      main.innerHTML = renderSolo(ts, data);
    }
  } catch (e) {
    main.innerHTML = `<div class="empty err">${e.message}</div>`;
  }
}

async function init() {
  state.runs = await listRuns();
  state.active = state.runs[0] ?? null;
  render();
}

init();
