use rustc_hash::FxHashMap;
use serde::Serialize;
use std::collections::BTreeSet;

use crate::cost::{compute_cost, compute_pattern_size};
use crate::debug_log::{ReplayConfig, ReplayLog, ReplayStep};
use crate::search::{Action, SearchState, SharedSearchData};

/// How to order the best-first search heap.
#[derive(Clone, Debug, Copy)]
pub enum SearchPriority {
    /// Lowest compressed-corpus-plus-pattern cost first (default).
    Cost,
    /// Deepest patterns first (depth-first).
    DepthFirst,
    /// Shallowest patterns first (breadth-first).
    BreadthFirst,
    /// Patterns with the most e-class matches first.
    MostMatches,
}

impl SearchPriority {
    /// Parse from the string format used by replay logs and the WASM API.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cost" => Some(Self::Cost),
            "depth-first" => Some(Self::DepthFirst),
            "breadth-first" => Some(Self::BreadthFirst),
            "most-matches" => Some(Self::MostMatches),
            _ => None,
        }
    }

    /// String representation matching replay log format.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cost => "cost",
            Self::DepthFirst => "depth-first",
            Self::BreadthFirst => "breadth-first",
            Self::MostMatches => "most-matches",
        }
    }
}

/// Configuration for a best-first search run.
pub struct BestFirstConfig {
    pub budget: usize,
    pub max_arity: usize,
    pub priority: SearchPriority,
}

/// One node in the in-memory search tree.
pub(crate) struct Node {
    pub(crate) parent: Option<usize>,
    pub(crate) children: Vec<usize>,
    pub(crate) action: Option<Action>,
    pub(crate) state: SearchState,
    pub(crate) cost: usize,
    pub(crate) depth: usize,
    pub(crate) priority: i64,
    pub(crate) expanded: bool,
}

/// Serializable snapshot of a search node for rendering.
#[derive(Serialize)]
pub struct NodeSnapshot {
    pub id: usize,
    pub parent: Option<usize>,
    pub action: Option<String>,
    pub pattern: String,
    pub cost: usize,
    pub num_matches: usize,
    pub arity: usize,
    pub pattern_size: usize,
    pub priority: f64,
    pub expanded: bool,
    pub depth: usize,
}

/// Serializable heap entry for rendering.
#[derive(Serialize)]
pub struct HeapEntry {
    pub node_id: usize,
    pub priority: f64,
}

/// Returns the heap priority for a node given the search strategy.
/// Lower values are explored first.
fn priority(strategy: &SearchPriority, cost: usize, depth: usize, num_matches: usize) -> i64 {
    match strategy {
        SearchPriority::Cost => cost as i64,
        SearchPriority::DepthFirst => -(depth as i64),
        SearchPriority::BreadthFirst => depth as i64,
        SearchPriority::MostMatches => -(num_matches as i64),
    }
}

/// Build a `NodeSnapshot` from a `Node`.
fn snapshot(id: usize, node: &Node) -> NodeSnapshot {
    NodeSnapshot {
        id,
        parent: node.parent,
        action: node.action.as_ref().map(|a| a.to_string()),
        pattern: node.state.pattern.to_string(),
        cost: node.cost,
        num_matches: node.state.matches.len(),
        arity: node.state.pattern.vars.len(),
        pattern_size: compute_pattern_size(&node.state.pattern),
        priority: node.priority as f64,
        expanded: node.expanded,
        depth: node.depth,
    }
}

/// Core expansion logic shared by batch and interactive search.
#[allow(clippy::too_many_arguments)]
///
/// Marks `node_id` as expanded, enumerates its successors, deduplicates
/// against `seen`, pushes survivors onto `heap`, and updates `best`.
fn expand_one(node_id: usize, nodes: &mut Vec<Node>, heap: &mut BTreeSet<(i64, usize)>, seen: &mut FxHashMap<String, usize>, best: &mut Option<(usize, usize)>, expansion_order: &mut Vec<usize>, shared: &SharedSearchData, root: egg::Id, strategy: &SearchPriority, max_arity: usize) {
    nodes[node_id].expanded = true;
    expansion_order.push(node_id);

    let successors = nodes[node_id].state.enumerate_successors(shared);
    let parent_depth = nodes[node_id].depth;
    let first_child = nodes.len();

    for (action, child_state) in successors {
        if let Some(ref follow) = shared.follow
            && !child_state.matches_follow(follow)
        {
            continue;
        }
        let key = child_state.pattern.to_string();
        let child_id = nodes.len();
        if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(key) {
            e.insert(child_id);
        } else {
            continue;
        }

        let child_cost = compute_cost(&shared.egraph, root, &child_state, shared.check_slow);
        let child_depth = parent_depth + 1;

        if child_state.pattern.vars.len() <= max_arity && best.as_ref().is_none_or(|(c, _)| child_cost < *c) {
            *best = Some((child_cost, child_id));
        }

        let child_prio = priority(strategy, child_cost, child_depth, child_state.matches.len());
        nodes.push(Node {
            parent: Some(node_id),
            children: Vec::new(),
            action: Some(action),
            state: child_state,
            cost: child_cost,
            depth: child_depth,
            priority: child_prio,
            expanded: false,
        });
        heap.insert((child_prio, child_id));
    }
    nodes[node_id].children = (first_child..nodes.len()).collect();
}

// ── Interactive search (WASM-friendly, steppable) ──────────────────────────

/// Persistent best-first search that owns all state. JS issues commands
/// (`step`, `expand_node`) and reads snapshots for rendering — no search
/// logic lives on the JS side.
pub struct InteractiveSearch {
    shared: SharedSearchData,
    root: egg::Id,
    original_size: usize,
    nodes: Vec<Node>,
    heap: BTreeSet<(i64, usize)>,
    seen: FxHashMap<String, usize>,
    best: Option<(usize, usize)>,
    best_found_at: Option<usize>,
    expansion_order: Vec<usize>,
    strategy: SearchPriority,
    max_arity: usize,
}

impl InteractiveSearch {
    /// Create a new search. Initializes the root state and places it on the heap.
    pub fn new(shared: SharedSearchData, root: egg::Id, original_size: usize, strategy: SearchPriority, max_arity: usize) -> Self {
        let initial = SearchState::new(&shared);
        let cost = compute_cost(&shared.egraph, root, &initial, shared.check_slow);
        let prio = priority(&strategy, cost, 0, initial.matches.len());
        let pat = initial.pattern.to_string();

        let mut heap = BTreeSet::new();
        heap.insert((prio, 0));
        let mut seen = FxHashMap::default();
        seen.insert(pat, 0);

        Self {
            shared,
            root,
            original_size,
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                action: None,
                state: initial,
                cost,
                depth: 0,
                priority: prio,
                expanded: false,
            }],
            heap,
            seen,
            best: None,
            best_found_at: None,
            expansion_order: Vec::new(),
            strategy,
            max_arity,
        }
    }

    /// Pop the best node from the heap and expand it. Returns the expanded node id.
    pub fn step(&mut self) -> Option<usize> {
        let (prio, node_id) = *self.heap.iter().next()?;
        self.heap.remove(&(prio, node_id));
        self.do_expand(node_id);
        Some(node_id)
    }

    /// Run up to `n` expansion steps. Returns the number actually expanded.
    pub fn step_n(&mut self, n: usize) -> usize {
        let mut count = 0;
        for _ in 0..n {
            if self.step().is_none() {
                break;
            }
            count += 1;
        }
        count
    }

    /// Expand a specific node by id (for manual clicks and replay).
    /// Returns false if the node doesn't exist or is already expanded.
    pub fn expand_node(&mut self, node_id: usize) -> bool {
        if node_id >= self.nodes.len() || self.nodes[node_id].expanded {
            return false;
        }
        self.heap.remove(&(self.nodes[node_id].priority, node_id));
        self.do_expand(node_id);
        true
    }

    /// Find an unexpanded node by pattern string (for replay). O(1) lookup.
    pub fn find_unexpanded_by_pattern(&self, pattern: &str) -> Option<usize> {
        self.seen.get(pattern).copied().filter(|&id| !self.nodes[id].expanded)
    }

    /// Check if any node (expanded or not) has the given pattern.
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.seen.contains_key(pattern)
    }

    /// Parse a replay log JSON string, apply its config, and run all steps.
    /// Returns the config so the caller can update UI.
    pub fn replay_from_json(&mut self, json: &str) -> Result<ReplayConfig, String> {
        let log: ReplayLog =
            serde_json::from_str(json).map_err(|e| format!("failed to parse replay: {e}"))?;
        if let Some(strategy) = SearchPriority::parse(&log.config.priority) {
            self.set_priority(strategy);
        }
        self.set_max_arity(log.config.max_arity);
        self.replay(&log.steps)?;
        Ok(log.config)
    }

    /// Replay a sequence of steps entirely in Rust. Returns `Ok(steps_replayed)`
    /// on success, or `Err(message)` on the first mismatch/missing pattern.
    pub fn replay(&mut self, steps: &[ReplayStep]) -> Result<usize, String> {
        for (i, step) in steps.iter().enumerate() {
            let node_id = match self.find_unexpanded_by_pattern(&step.pattern) {
                Some(id) => id,
                None => {
                    if self.has_pattern(&step.pattern) {
                        continue; // already expanded, skip
                    }
                    return Err(format!(
                        "step {}: pattern not found: {}",
                        i + 1,
                        step.pattern
                    ));
                }
            };
            let node = &self.nodes[node_id];
            if node.state.matches.len() != step.num_matches {
                return Err(format!(
                    "step {}: matches mismatch for {}: got {} expected {}",
                    i + 1,
                    step.pattern,
                    node.state.matches.len(),
                    step.num_matches,
                ));
            }
            if node.cost != step.cost {
                return Err(format!(
                    "step {}: cost mismatch for {}: got {} expected {}",
                    i + 1,
                    step.pattern,
                    node.cost,
                    step.cost,
                ));
            }
            self.expand_node(node_id);
        }
        Ok(steps.len())
    }

    // ── Snapshots for rendering ────────────────────────────────────────

    /// Snapshot of a single node.
    pub fn node_snapshot(&self, id: usize) -> Option<NodeSnapshot> {
        self.nodes.get(id).map(|n| snapshot(id, n))
    }

    /// Snapshot of all nodes (for tree rendering).
    pub fn all_nodes_snapshot(&self) -> Vec<NodeSnapshot> {
        self.nodes.iter().enumerate().map(|(id, n)| snapshot(id, n)).collect()
    }

    /// Top `n` heap entries sorted by priority ascending (best first).
    pub fn heap_top(&self, n: usize) -> Vec<HeapEntry> {
        self.heap.iter().take(n).map(|&(priority, node_id)| HeapEntry { node_id, priority: priority as f64 }).collect()
    }

    // ── Simple getters ─────────────────────────────────────────────────

    pub fn expansion_order(&self) -> &[usize] {
        &self.expansion_order
    }
    pub fn original_size(&self) -> usize {
        self.original_size
    }
    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }
    pub fn num_expansions(&self) -> usize {
        self.expansion_order.len()
    }
    pub fn heap_size(&self) -> usize {
        self.heap.len()
    }
    pub fn best_cost(&self) -> Option<usize> {
        self.best.map(|(c, _)| c)
    }
    pub fn best_node_id(&self) -> Option<usize> {
        self.best.map(|(_, id)| id)
    }
    /// Expansion index at which the best node was first found.
    pub fn best_found_at(&self) -> Option<usize> {
        self.best_found_at
    }
    /// Best cost and a reference to its search state.
    pub fn best_state(&self) -> Option<(usize, &SearchState)> {
        self.best.map(|(cost, id)| (cost, &self.nodes[id].state))
    }
    pub fn shared(&self) -> &SharedSearchData {
        &self.shared
    }

    // ── Node accessors (for SMC) ─────────────────────────────────────

    /// Expand a node if not already expanded, return its child node IDs.
    pub fn ensure_expanded(&mut self, node_id: usize) -> &[usize] {
        if !self.nodes[node_id].expanded {
            let prio = self.nodes[node_id].priority;
            self.heap.remove(&(prio, node_id));
            self.do_expand(node_id);
        }
        &self.nodes[node_id].children
    }

    /// Cost of a node.
    pub fn node_cost(&self, node_id: usize) -> usize {
        self.nodes[node_id].cost
    }

    /// Number of pattern variables for a node.
    pub fn node_num_vars(&self, node_id: usize) -> usize {
        self.nodes[node_id].state.pattern.vars.len()
    }

    /// Current max arity setting.
    pub fn max_arity(&self) -> usize {
        self.max_arity
    }

    // ── Settings ───────────────────────────────────────────────────────

    /// Rebuild the heap with a new priority strategy.
    pub fn set_priority(&mut self, strategy: SearchPriority) {
        self.strategy = strategy;
        self.rekey_heap();
    }

    /// Rekey all unexpanded nodes in the heap using the current strategy.
    pub fn rekey_heap(&mut self) {
        let ids: Vec<usize> = self.heap.iter().map(|&(_, id)| id).collect();
        self.heap.clear();
        for id in ids {
            let n = &mut self.nodes[id];
            n.priority = priority(&self.strategy, n.cost, n.depth, n.state.matches.len());
            self.heap.insert((n.priority, id));
        }
    }

    /// Update max arity and recompute best node.
    pub fn set_max_arity(&mut self, max_arity: usize) {
        self.max_arity = max_arity;
        self.best = None;
        self.best_found_at = None;
        for (id, node) in self.nodes.iter().enumerate() {
            if node.state.pattern.vars.len() <= max_arity && self.best.as_ref().is_none_or(|(c, _)| node.cost < *c) {
                self.best = Some((node.cost, id));
            }
        }
    }

    // ── Debug log generation ───────────────────────────────────────────

    /// Build a `ReplayLog` from the expansion history. Pass `budget` from the
    /// search config (or 0 for WASM/interactive use where budget is open-ended).
    pub fn replay_log(&self, budget: usize) -> ReplayLog {
        ReplayLog {
            config: ReplayConfig {
                priority: self.strategy.as_str().to_string(),
                budget,
                max_arity: self.max_arity,
            },
            steps: self
                .expansion_order
                .iter()
                .map(|&id| {
                    let n = &self.nodes[id];
                    ReplayStep {
                        pattern: n.state.pattern.to_string(),
                        action: n.action.as_ref().map(|a| a.to_string()),
                        num_matches: n.state.matches.len(),
                        cost: n.cost,
                    }
                })
                .collect(),
        }
    }

    /// Internal: expand a node using the shared `expand_one` helper.
    fn do_expand(&mut self, node_id: usize) {
        let old_best = self.best;
        expand_one(node_id, &mut self.nodes, &mut self.heap, &mut self.seen, &mut self.best, &mut self.expansion_order, &self.shared, self.root, &self.strategy, self.max_arity);
        if self.best != old_best {
            self.best_found_at = Some(self.expansion_order.len().saturating_sub(1));
        }
    }
}
