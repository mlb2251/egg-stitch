use colored::Colorize;
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::cost::{compute_cost, compute_pattern_size};
use crate::debug_log::{SearchTreeLog, TreeNodeLog};
use crate::lang::StitchEgraph;
use crate::search::{Action, SearchState, setup_search};

/// Output of a completed best-first enumerative search.
pub struct BestFirstResult {
    pub best: Option<(usize, SearchState)>,
    pub original_size: usize,
    /// Expansion index (pop count) at which the current best was first discovered.
    pub best_found_at: Option<usize>,
    /// Total number of heap pops performed before the loop stopped.
    pub num_expansions: usize,
    pub egraph: StitchEgraph,
    pub tree_log: Option<SearchTreeLog>,
}

/// One node in the in-memory search tree. Retained for parent-pointer lookups
/// and for the optional serialized debug log.
struct Node {
    parent: Option<usize>,
    action: Option<Action>,
    state: SearchState,
    cost: usize,
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
pub fn best_first(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> BestFirstResult {
    let (shared, original_size) = setup_search(egraph, root, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = args.num_steps;
    let max_arity = args.max_arity;
    let debug = args.debug_log;

    let initial_state = SearchState::new(&shared);
    let initial_cost = compute_cost(&shared.egraph, root, &initial_state, shared.check_slow);

    let mut nodes: Vec<Node> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    nodes.push(Node {
        parent: None,
        action: None,
        state: initial_state.clone(),
        cost: initial_cost,
        expanded: false,
    });
    heap.push(Reverse((initial_cost, 0)));
    seen.insert(initial_state.pattern.to_string());

    let mut best: Option<(usize, usize)> = None; // (cost, node_id)
    let mut best_found_at: Option<usize> = None;
    let mut expansion_order: Vec<usize> = Vec::new();
    let mut num_expansions: usize = 0;

    while let Some(Reverse((_cost, node_id))) = heap.pop() {
        if num_expansions >= budget {
            println!("{}", format!("reached expansion budget {}", budget).yellow());
            break;
        }

        nodes[node_id].expanded = true;
        expansion_order.push(node_id);

        let successors = nodes[node_id].state.enumerate_successors(&shared);

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

            if child_state.pattern.vars.len() <= max_arity && best.as_ref().is_none_or(|(c, _)| child_cost < *c) {
                println!(
                    "{} {} {}",
                    format!("[expansion {}]", num_expansions).yellow().bold(),
                    format!("new best: {}", child_cost).green().bold(),
                    child_state.pattern.to_string().cyan()
                );
                best = Some((child_cost, child_id));
                best_found_at = Some(num_expansions);
            }

            nodes.push(Node {
                parent: Some(node_id),
                action: Some(action),
                state: child_state,
                cost: child_cost,
                expanded: false,
            });
            heap.push(Reverse((child_cost, child_id)));
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
