pub mod best_first;
pub mod cost;
pub mod debug_log;
pub mod follow;
pub mod io;
pub mod lang;
pub mod logging;
pub mod matching;
pub mod math;
pub mod pattern;
pub mod results;
pub mod revexpr;
pub mod search;
pub mod smc;

// ── WASM API ──────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm")]
mod wasm_api {
    use serde::Serialize;
    use wasm_bindgen::prelude::*;

    use crate::best_first::{BestFirstConfig, SearchPriority, best_first};
    use crate::cost::{compute_cost, compute_pattern_size};
    use crate::search::{SearchState, setup_search};

    #[derive(Serialize)]
    struct StateInfo {
        state_id: usize,
        pattern: String,
        cost: usize,
        num_matches: usize,
        arity: usize,
        pattern_size: usize,
        compression_ratio: f64,
    }

    #[derive(Serialize)]
    struct SuccessorInfo {
        state_id: usize,
        action: String,
        pattern: String,
        cost: usize,
        num_matches: usize,
        arity: usize,
        pattern_size: usize,
        cost_diff: i64,
    }

    /// Interactive search engine exposed to JavaScript via WASM.
    ///
    /// Owns the e-graph and shared search context. Manages a pool of
    /// `SearchState`s that the JS side references by integer id.
    #[wasm_bindgen]
    pub struct Engine {
        shared: crate::search::SharedSearchData,
        root: egg::Id,
        original_size: usize,
        states: Vec<SearchState>,
    }

    #[wasm_bindgen]
    impl Engine {
        /// Load programs from a JSON array of s-expression strings.
        /// Optionally apply rewrite rules (pass `undefined` / `null` for none).
        #[wasm_bindgen(constructor)]
        pub fn new(programs_json: &str, rules_text: Option<String>) -> Result<Engine, JsError> {
            let (egraph, root, _cost_before) = crate::io::load_egraph_from_strings(programs_json, rules_text.as_deref());
            let (shared, original_size) = setup_search(egraph, root, None, false, 0.5, false);
            Ok(Engine { shared, root, original_size, states: Vec::new() })
        }

        /// Original (pre-compression) corpus size.
        pub fn original_size(&self) -> usize {
            self.original_size
        }

        /// Number of e-classes in the loaded e-graph.
        pub fn num_eclasses(&self) -> usize {
            self.shared.egraph.classes().count()
        }

        /// Create the initial search state (`?#0` matching every e-class).
        /// Returns the new state id.
        pub fn create_state(&mut self) -> usize {
            let state = SearchState::new(&self.shared);
            let id = self.states.len();
            self.states.push(state);
            id
        }

        /// Get info about a state (pattern, cost, matches, compression ratio).
        pub fn state_info(&self, state_id: usize) -> Result<JsValue, JsError> {
            let state = self.states.get(state_id).ok_or_else(|| JsError::new("invalid state_id"))?;
            let cost = compute_cost(&self.shared.egraph, self.root, state, false);
            let info = StateInfo {
                state_id,
                pattern: state.pattern.to_string(),
                cost,
                num_matches: state.matches.len(),
                arity: state.pattern.vars.len(),
                pattern_size: compute_pattern_size(&state.pattern),
                compression_ratio: self.original_size as f64 / cost as f64,
            };
            Ok(serde_wasm_bindgen::to_value(&info)?)
        }

        /// Enumerate all one-step successors of a state.
        ///
        /// Each successor is immediately stored in the state pool;
        /// the returned JSON array contains state ids the JS side can
        /// navigate to directly.
        pub fn successors(&mut self, state_id: usize) -> Result<JsValue, JsError> {
            let parent_cost = compute_cost(&self.shared.egraph, self.root, &self.states[state_id], false);
            // Clone state before calling enumerate_successors to satisfy borrow checker.
            let parent_clone = self.states[state_id].clone();
            let succs = parent_clone.enumerate_successors(&self.shared);
            let mut infos = Vec::new();
            for (action, state) in succs {
                let c = compute_cost(&self.shared.egraph, self.root, &state, false);
                let new_id = self.states.len();
                infos.push(SuccessorInfo {
                    state_id: new_id,
                    action: action.to_string(),
                    pattern: state.pattern.to_string(),
                    cost: c,
                    num_matches: state.matches.len(),
                    arity: state.pattern.vars.len(),
                    pattern_size: compute_pattern_size(&state.pattern),
                    cost_diff: c as i64 - parent_cost as i64,
                });
                self.states.push(state);
            }
            Ok(serde_wasm_bindgen::to_value(&infos)?)
        }

        /// Run automated best-first search starting from the given state.
        ///
        /// `priority` is one of: `"cost"`, `"depth-first"`, `"breadth-first"`, `"most-matches"`.
        /// Returns a `SearchTreeLog` as JSON (same format the existing tree viewer uses).
        pub fn run_search(&self, state_id: usize, priority: &str, budget: usize, max_arity: usize) -> Result<JsValue, JsError> {
            let strategy = match priority {
                "cost" => SearchPriority::Cost,
                "depth-first" => SearchPriority::DepthFirst,
                "breadth-first" => SearchPriority::BreadthFirst,
                "most-matches" => SearchPriority::MostMatches,
                _ => return Err(JsError::new("invalid priority: use cost|depth-first|breadth-first|most-matches")),
            };
            let config = BestFirstConfig {
                budget,
                max_arity,
                debug: true, // always produce tree log for the UI
                priority: strategy,
            };
            let initial = self.states.get(state_id).ok_or_else(|| JsError::new("invalid state_id"))?.clone();
            let result = best_first(&self.shared, self.root, self.original_size, initial, &config);
            Ok(serde_wasm_bindgen::to_value(&result.tree_log)?)
        }

        /// Compute cost of a single state.
        pub fn cost(&self, state_id: usize) -> Result<usize, JsError> {
            let state = self.states.get(state_id).ok_or_else(|| JsError::new("invalid state_id"))?;
            Ok(compute_cost(&self.shared.egraph, self.root, state, false))
        }
    }
}
