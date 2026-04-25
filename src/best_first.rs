use clap::ValueEnum;
use colored::Colorize;
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::cost::{compute_cost, compute_pattern_size};
use crate::debug_log::{SearchTreeLog, TreeNodeLog};
use crate::lang::{StitchEgraph, StitchLanguage};
use crate::search::{Action, SearchState, setup_search};

/// How to order the best-first search heap.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum SearchPriority {
    /// Lowest compressed-corpus-plus-pattern cost first (default).
    Cost,
    /// Deepest patterns first.
    DepthFirst,
    /// Shallowest patterns first.
    BreadthFirst,
    /// Patterns with the most e-class matches first.
    MostMatches,
}

impl SearchPriority {
    /// Parse from the kebab-case string form used by external APIs (e.g. WASM).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cost" => Some(Self::Cost),
            "depth-first" => Some(Self::DepthFirst),
            "breadth-first" => Some(Self::BreadthFirst),
            "most-matches" => Some(Self::MostMatches),
            _ => None,
        }
    }

    /// Kebab-case string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cost => "cost",
            Self::DepthFirst => "depth-first",
            Self::BreadthFirst => "breadth-first",
            Self::MostMatches => "most-matches",
        }
    }
}

/// Computes the heap priority for a node. Lower values are popped first.
/// `DepthFirst` and `MostMatches` invert by subtracting from `usize::MAX` —
/// safe since `depth` and `num_matches` won't approach that bound.
fn priority(strategy: SearchPriority, cost: usize, depth: usize, num_matches: usize) -> usize {
    match strategy {
        SearchPriority::Cost => cost,
        SearchPriority::DepthFirst => usize::MAX - depth,
        SearchPriority::BreadthFirst => depth,
        SearchPriority::MostMatches => usize::MAX - num_matches,
    }
}

/// Output of a completed best-first enumerative search.
pub struct BestFirstResult<L: StitchLanguage> {
    pub best: Option<(usize, SearchState<L>)>,
    pub original_size: usize,
    /// Expansion index (pop count) at which the current best was first discovered.
    pub best_found_at: Option<usize>,
    /// Total number of heap pops performed before the loop stopped.
    pub num_expansions: usize,
    pub egraph: StitchEgraph<L>,
    pub tree_log: Option<SearchTreeLog>,
}

/// One node in the in-memory search tree. Retained for parent-pointer lookups
/// and for the optional serialized debug log.
struct Node<L: StitchLanguage> {
    parent: Option<usize>,
    action: Option<Action<L>>,
    state: SearchState<L>,
    cost: usize,
    depth: usize,
    expanded: bool,
}

/// Runs best-first enumerative search to find a pattern that minimizes cost.
///
/// Maintains a min-heap keyed by `(cost, insertion_order)`. Each pop enumerates
/// every deterministic successor of the node, deduplicates against the set of
/// previously-seen canonical patterns, applies `max_arity` and `follow` filters,
/// and pushes the survivors back onto the heap. Stops at `num_steps` pops or an
/// empty heap. (No `dead_runs` cutoff: the search is systematic, so "no recent
/// improvement" just means we're grinding through a less promising branch.)
pub fn best_first<L: StitchLanguage>(egraph: StitchEgraph<L>, root: egg::Id, args: &crate::Args) -> BestFirstResult<L> {
    let (shared, cost_cache, original_size) = setup_search(egraph, root, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = args.num_steps;
    let max_arity = args.max_arity;
    let debug = args.debug_log;
    let strategy = args.priority;

    let initial_state = SearchState::new(&shared);
    let initial_cost = compute_cost(&shared.egraph, root, &cost_cache, &initial_state, shared.check_slow);
    let initial_prio = priority(strategy, initial_cost, 0, initial_state.matches.len());

    let mut nodes: Vec<Node<L>> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    nodes.push(Node {
        parent: None,
        action: None,
        state: initial_state.clone(),
        cost: initial_cost,
        depth: 0,
        expanded: false,
    });
    heap.push(Reverse((initial_prio, 0)));
    seen.insert(initial_state.pattern.to_string());

    let mut best: Option<(usize, usize)> = None; // (cost, node_id)
    let mut best_found_at: Option<usize> = None;
    let mut expansion_order: Vec<usize> = Vec::new();
    let mut num_expansions: usize = 0;

    while let Some(Reverse((_prio, node_id))) = heap.pop() {
        if num_expansions >= budget {
            println!("{}", format!("reached expansion budget {}", budget).yellow());
            break;
        }

        nodes[node_id].expanded = true;
        expansion_order.push(node_id);

        let successors = nodes[node_id].state.enumerate_successors(&shared);
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

            let child_cost = compute_cost(&shared.egraph, root, &cost_cache, &child_state, shared.check_slow);
            let child_depth = parent_depth + 1;
            let child_prio = priority(strategy, child_cost, child_depth, child_state.matches.len());
            let child_id = nodes.len();

            let cost_to_beat = best.as_ref().map_or(original_size, |(c, _)| *c);
            if child_state.pattern.vars.len() <= max_arity && child_cost < cost_to_beat {
                println!("{} {} {}", format!("[expansion {}]", num_expansions).yellow().bold(), format!("new best: {}", child_cost).green().bold(), child_state.pattern.to_string().cyan());
                best = Some((child_cost, child_id));
                best_found_at = Some(num_expansions);
            }

            nodes.push(Node {
                parent: Some(node_id),
                action: Some(action),
                state: child_state,
                cost: child_cost,
                depth: child_depth,
                expanded: false,
            });
            heap.push(Reverse((child_prio, child_id)));
        }

        num_expansions += 1;
    }

    println!("\n{}", "═══ RESULT ═══".green().bold());
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
                    expanded: n.expanded,
                })
                .collect(),
            expansion_order,
            best_node: best.map(|(_, id)| id),
        })
    } else {
        None
    };

    BestFirstResult {
        best: best_pair,
        original_size,
        best_found_at,
        num_expansions,
        egraph: shared.egraph,
        tree_log,
    }
}
