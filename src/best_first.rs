use colored::Colorize;
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

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

/// One node in the in-memory search tree. Retained for parent-pointer lookups
/// and for the optional serialized debug log.
struct Node {
    parent: Option<usize>,
    action: Option<Action>,
    state: SearchState,
    cost: usize,
    depth: usize,
    priority: i64,
    expanded: bool,
}

/// Returns the heap priority for a node given the search strategy.
/// Lower values are explored first (min-heap via `Reverse`).
fn priority(strategy: &SearchPriority, cost: usize, depth: usize, num_matches: usize) -> i64 {
    match strategy {
        SearchPriority::Cost => cost as i64,
        SearchPriority::DepthFirst => -(depth as i64),
        SearchPriority::BreadthFirst => depth as i64,
        SearchPriority::MostMatches => -(num_matches as i64),
    }
}

/// Runs best-first enumerative search to find a pattern that minimizes cost.
///
/// Maintains a min-heap keyed by `(cost, insertion_order)`. Each pop enumerates
/// every deterministic successor of the node, deduplicates against the set of
/// previously-seen canonical patterns, applies `max_arity` and `follow` filters,
/// and pushes the survivors back onto the heap. Stops at `budget` pops or an
/// empty heap.
pub fn best_first(shared: &SharedSearchData, root: egg::Id, original_size: usize, initial_state: SearchState, config: &BestFirstConfig) -> BestFirstResult {
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = config.budget;
    let max_arity = config.max_arity;
    let debug = config.debug;
    let strategy = &config.priority;

    let initial_cost = compute_cost(&shared.egraph, root, &initial_state, shared.check_slow);

    let mut nodes: Vec<Node> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    let initial_prio = priority(strategy, initial_cost, 0, initial_state.matches.len());
    nodes.push(Node {
        parent: None,
        action: None,
        state: initial_state.clone(),
        cost: initial_cost,
        depth: 0,
        priority: initial_prio,
        expanded: false,
    });
    heap.push(Reverse((initial_prio, 0)));
    seen.insert(initial_state.pattern.to_string());

    let mut best: Option<(usize, usize)> = None; // (cost, node_id)
    let mut best_found_at: Option<usize> = None;
    let mut expansion_order: Vec<usize> = Vec::new();
    let mut replay_steps: Vec<ReplayStep> = Vec::new();
    let mut num_expansions: usize = 0;

    while let Some(Reverse((_cost, node_id))) = heap.pop() {
        if num_expansions >= budget {
            println!("{}", format!("reached expansion budget {}", budget).yellow());
            break;
        }

        nodes[node_id].expanded = true;
        expansion_order.push(node_id);
        replay_steps.push(ReplayStep {
            pattern: nodes[node_id].state.pattern.to_string(),
            action: None,
            num_matches: nodes[node_id].state.matches.len(),
            cost: nodes[node_id].cost,
        });

        let successors = nodes[node_id].state.enumerate_successors(shared);

        for (action, child_state) in successors {
            if let Some(ref follow) = shared.follow
                && !child_state.matches_follow(follow) {
                    continue;
                }
            let key = child_state.pattern.to_string();
            if !seen.insert(key) {
                continue;
            }

            let child_cost = compute_cost(&shared.egraph, root, &child_state, shared.check_slow);
            let child_id = nodes.len();
            let child_depth = nodes[node_id].depth + 1;

            if child_state.pattern.vars.len() <= max_arity && best.as_ref().is_none_or(|(c, _)| child_cost < *c) {
                println!("{} {} {}", format!("[expansion {}]", num_expansions).yellow().bold(), format!("new best: {}", child_cost).green().bold(), child_state.pattern.to_string().cyan());
                best = Some((child_cost, child_id));
                best_found_at = Some(num_expansions);
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
        let priority_str = match strategy {
            SearchPriority::Cost => "cost",
            SearchPriority::DepthFirst => "depth-first",
            SearchPriority::BreadthFirst => "breadth-first",
            SearchPriority::MostMatches => "most-matches",
        };
        Some(ReplayLog {
            config: ReplayConfig {
                priority: priority_str.to_string(),
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
