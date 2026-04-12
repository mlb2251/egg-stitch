use colored::Colorize;
use rustc_hash::FxHashSet;
use serde::Serialize;
use std::collections::BTreeSet;

use crate::cost::{compute_cost, compute_pattern_size};
use crate::debug_log::{ReplayConfig, ReplayLog, ReplayStep, SearchTreeLog, TreeNodeLog};
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
    pub debug: bool,
    pub priority: SearchPriority,
}

/// Output of a completed best-first enumerative search.
pub struct BestFirstResult {
    pub best: Option<(usize, SearchState)>,
    pub original_size: usize,
    /// Expansion index (pop count) at which the current best was first discovered.
    pub best_found_at: Option<usize>,
    /// Total number of heap pops performed before the loop stopped.
    pub num_expansions: usize,
    pub tree_log: Option<SearchTreeLog>,
    /// Lightweight replay log: just the sequence of (pattern, action) choices.
    pub replay_log: Option<ReplayLog>,
}

/// One node in the in-memory search tree.
pub(crate) struct Node {
    pub(crate) parent: Option<usize>,
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
fn expand_one(node_id: usize, nodes: &mut Vec<Node>, heap: &mut BTreeSet<(i64, usize)>, seen: &mut FxHashSet<String>, best: &mut Option<(usize, usize)>, expansion_order: &mut Vec<usize>, shared: &SharedSearchData, root: egg::Id, strategy: &SearchPriority, max_arity: usize) {
    nodes[node_id].expanded = true;
    expansion_order.push(node_id);

    let successors = nodes[node_id].state.enumerate_successors(shared);
    let parent_depth = nodes[node_id].depth;

    for (action, child_state) in successors {
        if let Some(ref follow) = shared.follow
            && !child_state.matches_follow(follow)
        {
            continue;
        }
        let key = child_state.pattern.to_string();
        if !seen.insert(key) {
            continue;
        }

        let child_cost = compute_cost(&shared.egraph, root, &child_state, shared.check_slow);
        let child_id = nodes.len();
        let child_depth = parent_depth + 1;

        if child_state.pattern.vars.len() <= max_arity && best.as_ref().is_none_or(|(c, _)| child_cost < *c) {
            *best = Some((child_cost, child_id));
        }

        let child_prio = priority(strategy, child_cost, child_depth, child_state.matches.len());
        nodes.push(Node {
            parent: Some(node_id),
            action: Some(action),
            state: child_state,
            cost: child_cost,
            depth: child_depth,
            priority: child_prio,
            expanded: false,
        });
        heap.insert((child_prio, child_id));
    }
}

/// Runs best-first enumerative search to find a pattern that minimizes cost.
///
/// Maintains a min-heap keyed by `(priority, insertion_order)`. Each pop
/// enumerates every deterministic successor of the node, deduplicates against
/// previously-seen canonical patterns, applies `max_arity` and `follow` filters,
/// and pushes survivors back. Stops at `budget` pops or an empty heap.
pub fn best_first(shared: &SharedSearchData, root: egg::Id, original_size: usize, initial_state: SearchState, config: &BestFirstConfig) -> BestFirstResult {
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = config.budget;
    let max_arity = config.max_arity;
    let debug = config.debug;
    let strategy = &config.priority;

    let initial_cost = compute_cost(&shared.egraph, root, &initial_state, shared.check_slow);
    let initial_prio = priority(strategy, initial_cost, 0, initial_state.matches.len());

    let mut nodes: Vec<Node> = Vec::new();
    let mut heap: BTreeSet<(i64, usize)> = BTreeSet::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    seen.insert(initial_state.pattern.to_string());
    nodes.push(Node {
        parent: None,
        action: None,
        state: initial_state,
        cost: initial_cost,
        depth: 0,
        priority: initial_prio,
        expanded: false,
    });
    heap.insert((initial_prio, 0));

    let mut best: Option<(usize, usize)> = None;
    let mut best_found_at: Option<usize> = None;
    let mut expansion_order: Vec<usize> = Vec::new();
    let mut replay_steps: Vec<ReplayStep> = Vec::new();
    let mut num_expansions: usize = 0;
    let search_start = std::time::Instant::now();

    while let Some((_, node_id)) = heap.pop_first() {
        if num_expansions >= budget {
            println!("{}", format!("reached expansion budget {}", budget).yellow());
            break;
        }

        replay_steps.push(ReplayStep {
            pattern: nodes[node_id].state.pattern.to_string(),
            action: None,
            num_matches: nodes[node_id].state.matches.len(),
            cost: nodes[node_id].cost,
        });

        let old_best = best;
        expand_one(node_id, &mut nodes, &mut heap, &mut seen, &mut best, &mut expansion_order, shared, root, strategy, max_arity);

        if best != old_best {
            let (cost, id) = best.unwrap();
            println!("{} {} {}", format!("[expansion {}]", num_expansions).yellow().bold(), format!("new best: {}", cost).green().bold(), nodes[id].state.pattern.to_string().cyan(),);
            best_found_at = Some(num_expansions);
        }

        num_expansions += 1;
    }

    let search_elapsed = search_start.elapsed();
    println!("\n{}", "═══ RESULT ═══".green().bold());
    println!("{} {}", "search time:".dimmed(), format!("{:.1?}", search_elapsed).yellow());
    println!("{} {}", "expansions:".dimmed(), num_expansions.to_string().yellow());
    if let (Some(iter), Some((cost, best_id))) = (best_found_at, best) {
        let state = &nodes[best_id].state;
        println!("{} {}", "best found at expansion:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / cost as f64).green().bold());
    }

    let best_pair = best.map(|(cost, id)| (cost, nodes[id].state.clone()));

    let tree_log = if debug {
        Some(SearchTreeLog {
            original_size,
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(id, n)| TreeNodeLog {
                    id,
                    parent: n.parent,
                    action: n.action.as_ref().map(|a| a.to_string()),
                    pattern: n.state.pattern.to_string(),
                    arity: n.state.pattern.vars.len(),
                    pattern_size: compute_pattern_size(&n.state.pattern),
                    num_matches: n.state.matches.len(),
                    cost: n.cost,
                    priority: n.priority,
                    expanded: n.expanded,
                })
                .collect(),
            expansion_order,
            best_node: best.map(|(_, id)| id),
        })
    } else {
        None
    };

    let replay_log = if debug {
        Some(ReplayLog {
            config: ReplayConfig {
                priority: strategy.as_str().to_string(),
                budget,
                max_arity,
            },
            steps: replay_steps,
        })
    } else {
        None
    };

    BestFirstResult {
        best: best_pair,
        original_size,
        best_found_at,
        num_expansions,
        tree_log,
        replay_log,
    }
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
    seen: FxHashSet<String>,
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
        let mut seen = FxHashSet::default();
        seen.insert(pat);

        Self {
            shared,
            root,
            original_size,
            nodes: vec![Node {
                parent: None,
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

    /// Find an unexpanded node by pattern string (for replay).
    pub fn find_unexpanded_by_pattern(&self, pattern: &str) -> Option<usize> {
        self.nodes.iter().position(|n| !n.expanded && n.state.pattern.to_string() == pattern)
    }

    /// Check if any node (expanded or not) has the given pattern.
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.nodes.iter().any(|n| n.state.pattern.to_string() == pattern)
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
    pub fn shared(&self) -> &SharedSearchData {
        &self.shared
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

    /// Build a `SearchTreeLog` (same format as batch search debug output).
    pub fn tree_log(&self) -> SearchTreeLog {
        SearchTreeLog {
            original_size: self.original_size,
            nodes: self
                .nodes
                .iter()
                .enumerate()
                .map(|(id, n)| TreeNodeLog {
                    id,
                    parent: n.parent,
                    action: n.action.as_ref().map(|a| a.to_string()),
                    pattern: n.state.pattern.to_string(),
                    arity: n.state.pattern.vars.len(),
                    pattern_size: compute_pattern_size(&n.state.pattern),
                    num_matches: n.state.matches.len(),
                    cost: n.cost,
                    priority: n.priority,
                    expanded: n.expanded,
                })
                .collect(),
            expansion_order: self.expansion_order.clone(),
            best_node: self.best.map(|(_, id)| id),
        }
    }

    /// Build a `ReplayLog` from the expansion history.
    pub fn replay_log(&self) -> ReplayLog {
        ReplayLog {
            config: ReplayConfig {
                priority: self.strategy.as_str().to_string(),
                budget: 0,
                max_arity: self.max_arity,
            },
            steps: self
                .expansion_order
                .iter()
                .map(|&id| {
                    let n = &self.nodes[id];
                    ReplayStep {
                        pattern: n.state.pattern.to_string(),
                        action: None,
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
