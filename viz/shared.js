// Shared constants and utilities used across the viz UI.

export { escapeHtml } from './tree-render.js';

/// Await this to guarantee the browser paints the current DOM state.
export const paint = () => new Promise(r => requestAnimationFrame(() => setTimeout(r, 0)));

export const DOMAIN_DIR = '/data/domains/cogsci';
export const RULES_DIR = '/babble/harness/data/benchmark-dsrs';
export const ALL_DOMAINS = ['dials', 'furniture', 'nuts-bolts', 'wheels'];
export const RULES_FOR = {
  'dials': 'drawings.dials.rewrites',
  'furniture': 'drawings.furniture.rewrites',
  'nuts-bolts': 'drawings.nuts-bolts.rewrites',
  'wheels': 'drawings.wheels.rewrites',
};

/// Fetch programs + rules texts for a given domain/rules pair.
export async function fetchDomainData(domain, rulesFile) {
  const programsText = await fetch(`${DOMAIN_DIR}/${domain}.json`).then(r => {
    if (!r.ok) throw new Error(`${r.status} loading ${domain}.json`);
    return r.text();
  });
  let rulesText;
  if (rulesFile) {
    rulesText = await fetch(`${RULES_DIR}/${rulesFile}`).then(r => {
      if (!r.ok) throw new Error(`${r.status} loading ${rulesFile}`);
      return r.text();
    });
  }
  return { programsText, rulesText };
}

/// Parse an http.server directory listing HTML into { files, dirs }.
export function parseDirectoryListing(html) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const files = [], dirs = [];
  for (const a of doc.querySelectorAll('a')) {
    const h = a.getAttribute('href');
    if (!h || h.startsWith('?') || h === '../' || h === '/') continue;
    if (h.endsWith('/')) dirs.push(h.replace(/\/$/, ''));
    else if (h.endsWith('.json')) files.push(h);
  }
  return { files, dirs };
}

// ── Session folder + saving ─────────────────────────────────────────────────

let sessionFolder = null;

/// Get or create a timestamp-based session folder name.
export function getSessionFolder() {
  if (!sessionFolder) {
    const d = new Date();
    const pad = n => String(n).padStart(2, '0');
    sessionFolder = `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}_${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`;
  }
  return sessionFolder;
}

/// Reset the session folder (for starting a fresh batch).
export function resetSessionFolder() { sessionFolder = null; }

/// Save a JSON string to viz/results/<session>/<filename> via PUT.
export async function saveFile(filename, jsonStr) {
  const folder = getSessionFolder();
  const path = `viz/results/${folder}/${filename}`;
  const resp = await fetch(`/${path}`, { method: 'PUT', body: jsonStr, headers: { 'Content-Type': 'application/json' } });
  if (!resp.ok) throw new Error(`save failed: ${resp.status}`);
  return `${folder}/${filename}`;
}

/// Build a RunResult-compatible JSON object from engine state.
export function buildRunResult(eng, searchType, domain, rulesFile, elapsedSecs) {
  const r = eng.results_json();
  return {
    timestamp: Date.now() / 1000,
    search: searchType,
    input_file: `data/domains/cogsci/${domain}.json`,
    rules_file: rulesFile || null,
    elapsed_secs: elapsedSecs,
    initial_cost: r.original_size,
    cost_after_rewrites: r.original_size,
    final_cost: r.best_cost,
    compression_ratio: r.compression_ratio,
    pattern: r.pattern,
    arity: r.arity,
    num_matches: r.num_matches,
    num_expansions: r.num_expansions,
    num_steps_run: r.num_expansions,
  };
}

/// Save engine results. Returns { folder }.
export async function saveSearchResults(eng, domain, rulesFile, searchType, elapsed, outputName) {
  const result = buildRunResult(eng, searchType, domain, rulesFile, elapsed);
  await saveFile(`${outputName}.json`, JSON.stringify(result, null, 2));
  return { folder: getSessionFolder() };
}
