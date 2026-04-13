// WASM module loading and engine interaction layer.
// All direct wasm-bindgen calls go through this file.

let wasm = null;

/// Load and initialize the WASM module. Returns the module.
export async function loadWasm() {
  if (wasm) return wasm;
  wasm = await import('../pkg/egg_stitch.js');
  await wasm.default();
  return wasm;
}

/// Create a new Engine instance.
export function createEngine(programsText, rulesText, configJson) {
  if (!wasm) throw new Error('WASM not loaded');
  return new wasm.Engine(programsText, rulesText, configJson);
}

/// Run a search on an engine. Returns { results, elapsed }.
export function runSearch(engine, searchType, params) {
  const t0 = performance.now();
  if (searchType === 'smc') {
    engine.run_smc(params.particles, params.steps, params.temperature, params.deadRuns);
  } else {
    engine.step_n(params.budget);
  }
  const elapsed = (performance.now() - t0) / 1000;
  const results = engine.results_json();
  return { results, elapsed };
}
