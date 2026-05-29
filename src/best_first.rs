use clap::ValueEnum;
use colored::Colorize;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::cost::{CostScratch, CostSelection, SearchStateWithCostSelection, compute_cost_and_select, compute_pattern_size};
use crate::debug_log::{SearchTreeLog, TreeNodeLog};
use crate::lang::{LanguageFamily, StitchOp};
use crate::lower_bound::{LowerBoundPruner, PruneResult};
use crate::search::{SearchState, SeenTracker, SuccessorEnum, setup_search};

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

/// One "new best" event recorded during search.
#[derive(Serialize, Clone)]
pub struct BestHistoryEntry {
    /// Expansion index (pop count) at which this best was discovered.
    pub expansion: usize,
    /// Wall-clock seconds since search start when this best was discovered.
    pub elapsed_secs: f64,
    pub cost: usize,
    pub pattern: String,
}

/// Output of a completed best-first enumerative search.
pub struct BestFirstResult<F: LanguageFamily, O: StitchOp> {
    /// `(cost, winning state + the cost selection the optimiser picked for it)`.
    /// Threading the selection out saves `multiple_step_search` from re-running
    /// `compute_cost_and_select` just to recover it.
    pub best: Option<(usize, SearchStateWithCostSelection<F, O>)>,
    pub original_size: usize,
    /// Expansion index (pop count) at which the current best was first discovered.
    pub best_found_at: Option<usize>,
    /// Every successive "new best" event, in discovery order.
    pub best_history: Vec<BestHistoryEntry>,
    /// Total number of heap pops performed before the loop stopped.
    pub num_expansions: usize,
    pub data: crate::shared::SharedData<F, O>,
    pub tree_log: Option<SearchTreeLog>,
}

/// One node in the in-memory search tree. Retained for parent-pointer lookups
/// and for the optional serialized debug log.
struct Node<F: LanguageFamily, O: StitchOp> {
    parent: Option<usize>,
    state: SearchState<F, O>,
    cost: usize,
    depth: usize,
    expanded: bool,
    /// Lower bound on cost of any descendant; only set when `--opt-lower-bound` is on.
    /// Re-checked on pop in case `best` improved between push and pop.
    lower_bound: Option<usize>,
}

/// Runs best-first enumerative search to find a pattern that minimizes cost.
///
/// Maintains a min-heap keyed by `(cost, insertion_order)`. Each pop enumerates
/// every deterministic successor of the node, deduplicates against the set of
/// previously-seen canonical patterns, applies `max_arity` and `follow` filters,
/// and pushes the survivors back onto the heap. Stops at `num_steps` pops or an
/// empty heap. (No `dead_runs` cutoff: the search is systematic, so "no recent
/// improvement" just means we're grinding through a less promising branch.)
pub fn best_first<F: LanguageFamily, O: StitchOp>(data: crate::shared::SharedData<F, O>, args: &crate::Args) -> BestFirstResult<F, O> {
    let (shared, cost_cache, original_size) = setup_search(data, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = args.num_steps;
    let time_limit = args.time_limit.map(std::time::Duration::from_secs_f64);
    if budget.is_none() && time_limit.is_none() {
        panic!("best-first search requires at least one of --num-steps or --time-limit");
    }
    let max_arity = args.max_arity;
    let no_zero_arity = args.no_zero_arity;
    let debug = args.debug_log;
    let strategy = args.priority;

    let initial_state = SearchState::new(&shared, Some(0));
    let mut scratch = CostScratch::new(&shared.egraph);
    let initial_cost = compute_cost_and_select(&shared.egraph, shared.root, &cost_cache, &mut scratch, &initial_state, shared.check_slow).cost;
    let initial_prio = priority(strategy, initial_cost, 0, initial_state.matches.len());

    let mut nodes: Vec<Node<F, O>> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    let mut seen: Option<SeenTracker<F, O>> = args.opt_seen.then(SeenTracker::new);

    nodes.push(Node {
        parent: None,
        state: initial_state.clone(),
        cost: initial_cost,
        depth: 0,
        expanded: false,
        lower_bound: None,
    });
    heap.push(Reverse((initial_prio, 0)));
    if let Some(s) = seen.as_mut() {
        s.check_and_insert(initial_state.pattern.clone(), initial_state.frozen_count.unwrap_or(0));
    }

    let mut best: Option<(usize, usize, CostSelection)> = None; // (cost, node_id, selection)
    let mut best_found_at: Option<usize> = None;
    let mut best_history: Vec<BestHistoryEntry> = Vec::new();
    let mut expansion_order: Vec<usize> = Vec::new();
    let mut num_expansions: usize = 0;
    let mut cost_calls: usize = 0;
    let mut cost_time: Duration = Duration::ZERO;
    let mut dominance_hits: usize = 0;
    let mut lower_bound_pruner = LowerBoundPruner::new(args.opt_lower_bound);
    let mut useless_frozen_hits: usize = 0;
    let mut useless_inline_hits: usize = 0;
    let search_start = Instant::now();

    'search: while let Some(Reverse((_prio, node_id))) = heap.pop() {
        if let Some(b) = budget
            && num_expansions >= b
        {
            println!("{}", format!("reached expansion budget {}", b).yellow());
            break;
        }
        if let Some(limit) = time_limit
            && search_start.elapsed() >= limit
        {
            println!("{}", format!("reached time limit {:.3}s", limit.as_secs_f64()).yellow());
            break;
        }

        // Re-check the cached lower bound: best may have improved since this node was pushed.
        if let Some(lb) = nodes[node_id].lower_bound
            && let Some((c, _, _)) = best.as_ref()
            && lower_bound_pruner.recheck_cached(lb, *c)
        {
            continue;
        }

        nodes[node_id].expanded = true;
        expansion_order.push(node_id);

        if args.verbose {
            println!("{} {} {}", format!("[expansion {}]", num_expansions).dimmed(), "expanding:".dimmed(), nodes[node_id].state.pattern.to_string().cyan());
        }

        let parent_depth = nodes[node_id].depth;
        let successors: Vec<SearchState<F, O>> = match nodes[node_id].state.enumerate_successor_actions(&shared, args.opt_dominance_reuse, args.opt_useless_inline, max_arity, &mut dominance_hits, &mut useless_inline_hits) {
            SuccessorEnum::Dominant { child, .. } => vec![child],
            SuccessorEnum::All(actions) => actions.into_iter().map(|(a, _)| nodes[node_id].state.apply_action(&a, &shared)).collect(),
        };

        for child_state in successors {
            if let Some(ref follow) = shared.follow
                && !child_state.matches_follow(follow)
            {
                continue;
            }
            if let Some(s) = seen.as_mut()
                && s.check_and_insert(child_state.pattern.clone(), child_state.frozen_count.unwrap_or(0))
            {
                continue;
            }

            // Useless-frozen pruning: a frozen metavar bound to the same
            // (closed-under-pattern-binders) arg in every match adds no
            // compression. Stitch analog: argument-capture pruning.
            if args.opt_useless_frozen && child_state.is_useless_frozen(&shared) {
                useless_frozen_hits += 1;
                continue;
            }

            // Optimistic lower bound on this child's descendants — every match
            // collapses to one node. Skip the full cost call (and the descent)
            // when the bound already exceeds the current best.
            let cost_to_beat = best.as_ref().map_or(usize::MAX, |(c, _, _)| *c);
            let child_lower_bound = match lower_bound_pruner.try_prune(&shared.egraph, shared.root, &cost_cache, &mut scratch, &child_state, cost_to_beat) {
                PruneResult::Pruned => continue,
                PruneResult::Keep(lb) => Some(lb),
                PruneResult::Disabled => None,
            };

            let cost_t = Instant::now();
            // Capture the selection here so updates to `best` can stash it
            // without re-running the optimisation in `multiple_step_search`.
            // Cost-equal to the old `compute_cost` call — same underlying work.
            let child_selection = compute_cost_and_select(&shared.egraph, shared.root, &cost_cache, &mut scratch, &child_state, shared.check_slow);
            let child_cost = child_selection.cost;
            cost_time += cost_t.elapsed();
            cost_calls += 1;
            let child_depth = parent_depth + 1;
            let child_prio = priority(strategy, child_cost, child_depth, child_state.matches.len());
            let child_id = nodes.len();

            let cost_to_beat = best.as_ref().map_or(original_size, |(c, _, _)| *c);
            let arity = child_state.pattern.vars.len();
            if arity <= max_arity && !(no_zero_arity && arity == 0) && child_cost < cost_to_beat && !child_state.has_useless_var(&shared) {
                let elapsed = search_start.elapsed().as_secs_f64();
                println!(
                    "{} {} {} {}",
                    format!("[expansion {}]", num_expansions).yellow().bold(),
                    format!("new best: {}", child_cost).green().bold(),
                    child_state.pattern.to_string().cyan(),
                    format!("(t={:.3}s)", elapsed).dimmed()
                );
                best = Some((child_cost, child_id, child_selection.clone()));
                best_found_at = Some(num_expansions);
                best_history.push(BestHistoryEntry {
                    expansion: num_expansions,
                    elapsed_secs: elapsed,
                    cost: child_cost,
                    pattern: child_state.pattern.to_string(),
                });
            }

            // Mirrors SMC's `follow exact match` exit (src/smc.rs:132): once
            // a successor is alpha-equivalent to the follow target the search
            // has reached the goal, and continuing risks overwriting `best`
            // with a cheaper non-matching pattern that slipped past the prefix
            // filter. Record this child as best and stop.
            //
            // NOTE: --follow mode is a reachability check — we only care that
            // the target pattern is constructible via the expansions BFS/SMC
            // has access to. Overwriting `best` with the follow-hit child even
            // when its cost is worse than a previously-recorded best is
            // intentional: the reported `best` in follow mode is "the follow
            // target we reached", not "the cheapest pattern seen". Do not
            // "fix" this by guarding the overwrite on `child_cost < best.cost`.
            let exact_follow_hit = shared.follow.as_ref().is_some_and(|f| crate::follow::matches_follow_serialized(&child_state, f, &shared.egraph));

            nodes.push(Node {
                parent: Some(node_id),
                state: child_state,
                cost: child_cost,
                depth: child_depth,
                expanded: false,
                lower_bound: child_lower_bound,
            });
            heap.push(Reverse((child_prio, child_id)));

            if exact_follow_hit {
                let elapsed = search_start.elapsed().as_secs_f64();
                println!(
                    "{} {} {} {}",
                    format!("[expansion {}]", num_expansions).yellow().bold(),
                    format!("follow exact match: {}", child_cost).green().bold(),
                    nodes[child_id].state.pattern.to_string().cyan(),
                    format!("(t={:.3}s)", elapsed).dimmed()
                );
                best = Some((child_cost, child_id, child_selection));
                best_found_at = Some(num_expansions);
                num_expansions += 1;
                break 'search;
            }
        }

        num_expansions += 1;
    }

    let total_elapsed = search_start.elapsed();
    println!("\n{}", "═══ STATS ═══".blue().bold());
    println!("{} {}", "expansions:".dimmed(), num_expansions.to_string().bold());
    println!("{} {}", "nodes created:".dimmed(), nodes.len().to_string().bold());
    println!("{} {}", "heap size at end:".dimmed(), heap.len().to_string().bold());
    let (seen_len, seen_hits, seen_secs) = seen.as_ref().map_or((0, 0, 0.0), |s| (s.len(), s.hits, s.time.as_secs_f64()));
    println!("{} {}", "seen-set size:".dimmed(), seen_len.to_string().bold());
    println!("{} {} {}", "seen-set hits:".dimmed(), seen_hits.to_string().bold(), format!("(time: {:.3}s)", seen_secs).dimmed());
    println!("{} {}", "dominance hits:".dimmed(), dominance_hits.to_string().bold());
    lower_bound_pruner.print_stats();
    println!("{} {}", "useless-frozen hits:".dimmed(), useless_frozen_hits.to_string().bold());
    println!("{} {}", "useless-inline hits:".dimmed(), useless_inline_hits.to_string().bold());
    println!("{} {} {}", "compute_cost calls:".dimmed(), cost_calls.to_string().bold(), format!("(time: {:.3}s)", cost_time.as_secs_f64()).dimmed());
    println!("{} {}", "total search time:".dimmed(), format!("{:.3}s", total_elapsed.as_secs_f64()).bold());

    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, best_id, _))) = (best_found_at, best.as_ref()) {
        let state = &nodes[*best_id].state;
        println!("{} {}", "best found at expansion:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / *cost as f64).green().bold());
    }

    let best_node_id = best.as_ref().map(|(_, id, _)| *id);
    let best_pair = best.map(|(cost, id, selection)| (cost, SearchStateWithCostSelection { state: nodes[id].state.clone(), selection }));

    let tree_log = if debug {
        let weights = shared.egraph.analysis.weights;
        Some(SearchTreeLog {
            original_size,
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(id, n)| TreeNodeLog {
                    id,
                    parent: n.parent,
                    pattern: n.state.pattern.to_string(),
                    arity: n.state.pattern.vars.len(),
                    pattern_size: compute_pattern_size(&n.state.pattern, &weights),
                    num_matches: n.state.matches.len(),
                    cost: n.cost,
                    expanded: n.expanded,
                })
                .collect(),
            expansion_order,
            best_node: best_node_id,
        })
    } else {
        None
    };

    BestFirstResult {
        best: best_pair,
        original_size,
        best_history,
        best_found_at,
        num_expansions,
        data: shared.into_data(),
        tree_log,
    }
}
