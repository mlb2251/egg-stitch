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
    use wasm_bindgen::prelude::*;

    use crate::best_first::{InteractiveSearch, SearchPriority};
    use crate::search::setup_search;
    use crate::smc::SmcConfig;

    /// Interactive search engine exposed to JavaScript via WASM.
    ///
    /// Wraps an `InteractiveSearch` that owns all search state (heap, seen
    /// set, best tracking, node tree). JS issues commands and reads snapshots
    /// — no search logic lives on the JS side.
    #[wasm_bindgen]
    pub struct Engine {
        inner: InteractiveSearch,
    }

    #[wasm_bindgen]
    impl Engine {
        /// Load programs and initialize search. Priority defaults to "cost", max_arity to 2.
        #[wasm_bindgen(constructor)]
        pub fn new(programs_json: &str, rules_text: Option<String>) -> Result<Engine, JsError> {
            let (egraph, root, _) = crate::io::load_egraph_from_strings(programs_json, rules_text.as_deref());
            let (shared, original_size) = setup_search(egraph, root, None, false, 0.5, false);
            let search = InteractiveSearch::new(shared, root, original_size, SearchPriority::Cost, 2);
            Ok(Engine { inner: search })
        }

        // ── Simple getters ─────────────────────────────────────────────

        /// Original (pre-compression) corpus size.
        pub fn original_size(&self) -> usize {
            self.inner.original_size()
        }

        /// Total number of nodes in the search tree.
        pub fn num_nodes(&self) -> usize {
            self.inner.num_nodes()
        }

        /// Number of nodes expanded so far.
        pub fn num_expansions(&self) -> usize {
            self.inner.num_expansions()
        }

        /// Number of unexpanded nodes on the heap.
        pub fn heap_size(&self) -> usize {
            self.inner.heap_size()
        }

        /// Best cost found so far, or -1 if none.
        pub fn best_cost(&self) -> f64 {
            self.inner.best_cost().map_or(-1.0, |c| c as f64)
        }

        /// Node id of the best node, or -1 if none.
        pub fn best_node_id(&self) -> i32 {
            self.inner.best_node_id().map_or(-1, |id| id as i32)
        }

        // ── Commands ───────────────────────────────────────────────────

        /// Pop the best node from the heap and expand it. Returns the
        /// expanded node id, or -1 if the heap is empty.
        pub fn step(&mut self) -> i32 {
            self.inner.step().map_or(-1, |id| id as i32)
        }

        /// Run up to `n` expansion steps. Returns the count actually expanded.
        pub fn step_n(&mut self, n: usize) -> usize {
            self.inner.step_n(n)
        }

        /// Expand a specific node (for manual clicks and replay).
        /// Returns true if the node was expanded.
        pub fn expand_node(&mut self, node_id: usize) -> bool {
            self.inner.expand_node(node_id)
        }

        /// Find an unexpanded node by pattern string (for replay).
        /// Returns the node id, or -1 if not found.
        pub fn find_unexpanded_by_pattern(&self, pattern: &str) -> i32 {
            self.inner.find_unexpanded_by_pattern(pattern).map_or(-1, |id| id as i32)
        }

        /// Check if any node has the given pattern (for replay error reporting).
        pub fn has_pattern(&self, pattern: &str) -> bool {
            self.inner.has_pattern(pattern)
        }

        // ── SMC ────────────────────────────────────────────────────────

        /// Run SMC search over the shared tree. After this call, the tree
        /// is populated and can be queried with `nodes_json()`, `best_cost()`,
        /// `expansion_order_json()`, etc. Replay also works.
        pub fn run_smc(&mut self, num_particles: usize, num_steps: usize, temperature: f64, dead_runs: usize) {
            let config = SmcConfig { num_particles, num_steps, temperature, dead_runs, verbose: false };
            crate::smc::smc(&mut self.inner, &config);
        }

        // ── Settings ───────────────────────────────────────────────────

        /// Rekey all unexpanded nodes in the heap using the current priority strategy.
        pub fn rekey_heap(&mut self) {
            self.inner.rekey_heap();
        }

        /// Change the heap priority strategy. Rebuilds the heap.
        pub fn set_priority(&mut self, priority: &str) -> Result<(), JsError> {
            let strategy = SearchPriority::parse(priority).ok_or_else(|| JsError::new("invalid priority: use cost|depth-first|breadth-first|most-matches"))?;
            self.inner.set_priority(strategy);
            Ok(())
        }

        /// Change the max arity and recompute best node.
        pub fn set_max_arity(&mut self, max_arity: usize) {
            self.inner.set_max_arity(max_arity);
        }

        /// Parse a replay log JSON string, apply its config (priority, max_arity),
        /// and run all steps entirely in Rust. Returns the config as JSON so JS
        /// can update dropdowns.
        pub fn replay_from_json(&mut self, json: &str) -> Result<JsValue, JsError> {
            let config = self
                .inner
                .replay_from_json(json)
                .map_err(|e| JsError::new(&e))?;
            Ok(serde_wasm_bindgen::to_value(&config)?)
        }

        // ── Snapshots (JSON via serde-wasm-bindgen) ────────────────────

        /// Full node tree snapshot for rendering.
        pub fn nodes_json(&self) -> Result<JsValue, JsError> {
            Ok(serde_wasm_bindgen::to_value(&self.inner.all_nodes_snapshot())?)
        }

        /// Top `n` heap entries sorted by priority (ascending = best first).
        pub fn heap_top_json(&self, n: usize) -> Result<JsValue, JsError> {
            Ok(serde_wasm_bindgen::to_value(&self.inner.heap_top(n))?)
        }

        /// Info for a single node.
        pub fn node_info_json(&self, node_id: usize) -> Result<JsValue, JsError> {
            let snap = self.inner.node_snapshot(node_id).ok_or_else(|| JsError::new("invalid node_id"))?;
            Ok(serde_wasm_bindgen::to_value(&snap)?)
        }

        /// Expansion order as a JSON array of node ids.
        pub fn expansion_order_json(&self) -> Result<JsValue, JsError> {
            Ok(serde_wasm_bindgen::to_value(self.inner.expansion_order())?)
        }
    }
}
