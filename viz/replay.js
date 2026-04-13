// Replay log loading and step-by-step execution.
// Planned for deprecation -- kept separate from core interactive logic.

import { parseDirectoryListing } from './shared.js';

const $ = id => document.getElementById(id);

// ── Replay state ────────────────────────────────────────────────────────────

let replayJsonText = null;  // raw JSON string, sent to Rust for bulk replay
let replaySteps = [];       // parsed steps, used for single-step replay
let replayIdx = 0;
let replayExpectedCost = null;

export function getReplaySteps() { return replaySteps; }
export function getReplayIdx() { return replayIdx; }
export function getReplayExpectedCost() { return replayExpectedCost; }
export function setReplayExpectedCost(v) { replayExpectedCost = v; }

// ── Scanning for available replays ──────────────────────────────────────────

/// Populate the replay <select> dropdown with available replay logs for the current domain.
export async function scanReplays(engine) {
  const sel = $('selReplay');
  const domain = $('selDomain').value;
  sel.innerHTML = '<option value="">none</option>';
  try {
    const listing = await fetch('results/').then(r => r.text());
    const { files: topFiles, dirs } = parseDirectoryListing(listing);

    const candidates = [];

    for (const f of topFiles) {
      if (f.endsWith('_debug.json') || f.endsWith('_replay.json')) continue;
      try {
        const r = await fetch(`results/${f}`).then(r => r.json());
        if (r.replay_log_file && r.input_file && r.input_file.includes(`/${domain}.json`)) {
          candidates.push({ label: f.replace(/\.json$/, ''), path: r.replay_log_file, finalCost: r.final_cost });
        }
      } catch { /* skip bad files */ }
    }

    for (const d of dirs) {
      try {
        const sub = await fetch(`results/${d}/`).then(r => r.text());
        const { files } = parseDirectoryListing(sub);
        for (const f of files) {
          if (f.endsWith('_debug.json') || f.endsWith('_replay.json')) continue;
          try {
            const r = await fetch(`results/${d}/${f}`).then(r => r.json());
            if (r.replay_log_file && r.input_file && r.input_file.includes(`/${domain}.json`)) {
              candidates.push({ label: `${d}/${f.replace(/\.json$/, '')}`, path: `${d}/${r.replay_log_file}`, finalCost: r.final_cost });
            }
          } catch { /* skip */ }
        }
      } catch { /* skip */ }
    }

    for (const c of candidates) {
      const opt = document.createElement('option');
      opt.value = c.path;
      opt.dataset.finalCost = c.finalCost ?? '';
      opt.textContent = c.label + (c.finalCost != null ? ` (cost ${c.finalCost})` : '');
      sel.appendChild(opt);
    }
  } catch (e) {
    console.warn('could not scan replays:', e);
  }
}

// ── Config application ──────────────────────────────────────────────────────

/// Apply a replay config to the UI controls and engine.
export function applyReplayConfig(config, engine) {
  if (!config) return;
  if (config.priority) $('cfgPriority').value = config.priority;
  if (config.budget) $('cfgBudget').value = config.budget;
  if (config.max_arity) $('cfgArity').value = config.max_arity;
  if (engine) {
    engine.set_priority(config.priority || 'cost');
    engine.set_max_arity(config.max_arity || 2);
  }
}

// ── Replay button state ─────────────────────────────────────────────────────

export function updateReplayButtons(engine) {
  const hasSteps = replaySteps.length > 0 && replayIdx < replaySteps.length && engine;
  $('btnReplay').disabled = !hasSteps;
  $('btnReplayAll').disabled = !hasSteps;
}

// ── Single-step replay ──────────────────────────────────────────────────────

/// Replay one step using the Rust engine. Returns true if a node was expanded.
export function replayOneStep(engine, openNodes, statusBar) {
  if (replayIdx >= replaySteps.length) return false;
  const step = replaySteps[replayIdx];
  replayIdx++;

  const nodeId = engine.find_unexpanded_by_pattern(step.pattern);
  if (nodeId < 0) {
    if (engine.has_pattern(step.pattern)) return true; // already expanded, skip
    const msg = `replay error at step ${replayIdx}: pattern not found in tree: ${step.pattern}`;
    statusBar.innerHTML = `<b class="bad">${msg}</b>`;
    console.error(msg);
    return false;
  }

  // Validate matches and cost against expected values.
  const info = engine.node_info_json(nodeId);
  const matchesOk = step.num_matches == null || info.num_matches === step.num_matches;
  const costOk = step.cost == null || info.cost === step.cost;
  if (!matchesOk || !costOk) {
    const parts = [`replay mismatch at step ${replayIdx}: ${step.pattern}`];
    parts.push(`  matches: ${info.num_matches} (expected ${step.num_matches})`);
    parts.push(`  cost: ${info.cost} (expected ${step.cost})`);
    parts.push(`  pattern_size: ${info.pattern_size}`);
    statusBar.innerHTML = `<b class="bad">${parts.join('<br>')}</b>`;
    console.error(parts.join('\n'));
    return false;
  }

  engine.expand_node(nodeId);
  openNodes.add(nodeId);
  return true;
}

// ── Bulk replay ─────────────────────────────────────────────────────────────

/// Await this to guarantee the browser paints the current DOM state.
const paint = () => new Promise(r => requestAnimationFrame(() => setTimeout(r, 0)));

/// Run a replay from raw JSON text. Parsing + execution happen in Rust.
/// Returns { error, replayMs, nExpanded, bestCost }.
export async function runReplayFromJson(engine, json, statusBar) {
  const stepCount = replaySteps.length || '?';
  statusBar.textContent = `replaying ${stepCount} steps\u2026`;
  await paint();

  const t0 = performance.now();
  let error = null;
  try {
    const config = engine.replay_from_json(json);
    if (config.priority) $('cfgPriority').value = config.priority;
    if (config.max_arity) $('cfgArity').value = config.max_arity;
    replayIdx = replaySteps.length;
  } catch (e) {
    error = e.message;
  }
  const replayMs = (performance.now() - t0).toFixed(0);
  const nExpanded = engine.num_expansions();
  const bestCost = engine.best_cost();
  return { error, replayMs, nExpanded, bestCost };
}

/// Fetch a replay log by URL and run it.
export async function runReplayFromUrl(engine, url, statusBar) {
  const text = await fetch(url).then(r => {
    if (!r.ok) throw new Error(`${r.status} loading ${url}`);
    return r.text();
  });
  return runReplayFromJson(engine, text, statusBar);
}

// ── Event wiring ────────────────────────────────────────────────────────────

/// Wire up the replay <select> change handler.
export function wireReplaySelect(engine, statusBar) {
  $('selReplay').addEventListener('change', async () => {
    const sel = $('selReplay');
    const path = sel.value;
    if (!path) {
      replayJsonText = null; replaySteps = []; replayIdx = 0; replayExpectedCost = null;
      updateReplayButtons(engine);
      return;
    }
    const opt = sel.options[sel.selectedIndex];
    replayExpectedCost = opt.dataset.finalCost ? parseInt(opt.dataset.finalCost) : null;
    try {
      replayJsonText = await fetch(`results/${path}`).then(r => {
        if (!r.ok) throw new Error(r.status);
        return r.text();
      });
      const log = JSON.parse(replayJsonText);
      applyReplayConfig(log.config, engine);
      replaySteps = log.steps || [];
      replayIdx = 0;
      updateReplayButtons(engine);
      statusBar.textContent = `loaded replay: ${replaySteps.length} steps` + (replayExpectedCost != null ? ` (expected cost: ${replayExpectedCost})` : '');
    } catch (err) {
      alert('failed to load replay: ' + err);
    }
  });
}

/// Get the raw JSON text for bulk replay (null if none loaded).
export function getReplayJsonText() { return replayJsonText; }
