use clap::ValueEnum;
use colored::Colorize;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::cost::{CostScratch, CostSelection, SearchStateWithCostSelection, compute_cost_and_select, compute_pattern_size};
use crate::debug_log::{SearchTreeLog, TreeNodeLog};
use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchOp};
use crate::lower_bound::{LowerBoundPruner, PruneResult};
use crate::search::{SearchState, SeenTracker, SharedSearchData, SuccessorEnum, setup_search};
use egg::Language;

/// How to order the best-first search heap.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum SearchPriority {
    /// Lowest compressed-corpus-plus-pattern cost first.
    Cost,
    /// Deepest patterns first.
    DepthFirst,
    /// Shallowest patterns first.
    BreadthFirst,
    /// Patterns with the most e-class matches first.
    MostMatches,
    /// Lexicographic `(forced-expansion, cost)` (default)
    ForcedThenCost,
}

impl SearchPriority {
    /// Parse from the kebab-case string form used by external APIs (e.g. WASM).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cost" => Some(Self::Cost),
            "depth-first" => Some(Self::DepthFirst),
            "breadth-first" => Some(Self::BreadthFirst),
            "most-matches" => Some(Self::MostMatches),
            "forced-then-cost" => Some(Self::ForcedThenCost),
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
            Self::ForcedThenCost => "forced-then-cost",
        }
    }
}

/// Computes a node's `(heap key, forced-expansion)`; lower key is popped first
/// (the key is a tuple for lexicographic ordering). `forced_lower_bound` is the
/// parent's forced-expansion, used to early-exit the `ForcedThenCost` scan
/// (forced is monotone, so no child drops below it); the returned forced value
/// is threaded down as the next level's bound. Pass `i64::MIN` when there's no
/// parent. `forced` is 0 for strategies that don't use it.
fn priority<F: LanguageFamily, O: StitchOp>(strategy: SearchPriority, cost: usize, depth: usize, state: &SearchState<F, O>, shared: &SharedSearchData<F, O>, forced_lower_bound: i64) -> ((usize, usize), i64) {
    match strategy {
        SearchPriority::Cost => ((cost, 0), 0),
        SearchPriority::DepthFirst => ((usize::MAX - depth, 0), 0),
        SearchPriority::BreadthFirst => ((depth, 0), 0),
        SearchPriority::MostMatches => ((usize::MAX - state.matches.len(), 0), 0),
        SearchPriority::ForcedThenCost => {
            let forced = state.forced_expansion_argmin(shared, forced_lower_bound).map_or(0, |(v, _)| v);
            // clamp to 0 because anything <= 0 means no forced expansion
            ((forced.max(0) as usize, cost), forced)
        }
    }
}

/// True iff every every e-node in the e-graph has the same cost as all
/// other e-nodes in its class.
fn cost_balanced<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>) -> bool {
    let weights = egraph.analysis.weights;
    egraph.classes().all(|c| c.nodes.iter().all(|n| n.discriminant().intrinsic_size(&weights) + n.children().iter().map(|&ch| egraph[ch].data.size).sum::<u32>() == c.data.size))
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
    /// Heap size when the loop stopped. `0` means the frontier was exhausted
    /// (the search converged); a non-zero value means it hit the `num_steps` cap.
    pub heap_size_at_end: usize,
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
    /// This node's ForcedExpansion (0 unless ordering by `ForcedThenCost`). Used
    /// as the monotone lower bound that early-exits its children's forced scans.
    forced: i64,
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
    if budget.is_none() && time_limit.is_none() && args.max_forced_expansion.0.is_none() {
        panic!("best-first search requires at least one of --num-steps, --time-limit, or --max-forced-expansion");
    }
    let max_arity = args.max_arity;
    let no_zero_arity = args.no_zero_arity;
    let debug = args.debug_log;
    // ForcedThenCost reduces to Cost when the e-graph is cost-balanced
    let strategy = if matches!(args.priority, SearchPriority::ForcedThenCost) && cost_balanced::<F, O>(&shared.egraph) {
        println!("{}", "rules are cost-balanced: ordering by cost (forced-expansion ordering not needed)".dimmed());
        SearchPriority::Cost
    } else {
        args.priority
    };
    let initial_state = SearchState::new(&shared, true);
    let mut scratch = CostScratch::new(&shared.egraph);
    let initial_cost = compute_cost_and_select(&shared.egraph, shared.root, &cost_cache, &mut scratch, &initial_state, shared.check_slow).cost;
    // No parent, so no lower bound: scan fully.
    let (initial_prio, initial_forced) = priority(strategy, initial_cost, 0, &initial_state, &shared, i64::MIN);

    let mut nodes: Vec<Node<F, O>> = Vec::new();
    // Heap key: `(priority, insertion-order)`. `priority` is itself a tuple so
    // it can order lexicographically (e.g. forced-expansion then cost);
    // insertion order breaks remaining ties to stay deterministic.
    let mut heap: BinaryHeap<Reverse<((usize, usize), usize)>> = BinaryHeap::new();
    let mut seen: Option<SeenTracker<F, O>> = args.opt_seen.then(|| {
        // Lift the program-language DSRs onto the frozen-var seen language by
        // re-parsing the rule file at `F::Apply<OpWithFrozenVar<O>>` with `()`
        // analysis: op names route to `Node(..)`, `?x` metavars stay metavars,
        // so the rules match the inserted patterns directly. Plain `lhs => rhs`
        // rules lift cleanly; a parse failure degrades to an empty set with a
        // warning rather than aborting. With `--seen-egraph-saturate` (default
        // on) these run after each insert so the egraph dedups modulo
        // DSR-equivalence; the end-of-run audit uses them either way.
        let weights = shared.egraph.analysis.weights;
        let rules: Vec<egg::Rewrite<F::Apply<crate::lang::OpWithFrozenVar<O>>, ()>> = match args.rules.as_deref() {
            Some(path) => crate::io::from_file(path, &weights).unwrap_or_else(|e| {
                println!("{} {}", "seen-egraph: failed to lift rules:".red(), e);
                vec![]
            }),
            None => vec![],
        };
        // High limits so saturation, not a cap, stops the run where the rules permit.
        SeenTracker::new(rules, args.iter_limit.max(1000), args.node_limit, args.seen_egraph_saturate, args.seen_egraph_saturate_every, args.seen_egraph_decides, args.seen_egraph_saturate_dynamic)
    });

    nodes.push(Node {
        parent: None,
        state: initial_state.clone(),
        cost: initial_cost,
        depth: 0,
        expanded: false,
        lower_bound: None,
        forced: initial_forced,
    });
    heap.push(Reverse((initial_prio, 0)));
    if let Some(s) = seen.as_mut() {
        s.check_and_insert(initial_state.pattern.clone(), initial_state.pattern.frozen_mask());
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

    'search: loop {
        // Check cutoffs before popping so a node isn't discarded from the frontier.
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
        let Some(Reverse((_prio, node_id))) = heap.pop() else {
            break;
        };

        // Re-check the cached lower bound: best may have improved since this node was pushed.
        if let Some(lb) = nodes[node_id].lower_bound
            && let Some((c, _, _)) = best.as_ref()
            && lower_bound_pruner.recheck_cached(lb, *c)
        {
            continue;
        }

        nodes[node_id].expanded = true;
        expansion_order.push(node_id);

        if args.verbose || args.verbose_forced_expansion {
            let tag = format!("[expansion {}]", num_expansions);
            let pat = nodes[node_id].state.pattern.to_string();
            if args.verbose {
                println!("{} {} {}", tag.dimmed(), "expanding:".dimmed(), pat.clone().cyan());
            }
            if args.verbose_forced_expansion {
                let forced_str = match nodes[node_id].state.forced_expansion_argmin(&shared, i64::MIN) {
                    Some((e, root)) => format!("[forced-expansion={} @root={}]", e, nodes[node_id].state.min_term(&shared, root)),
                    None => "[forced-expansion=- (no matches)]".to_string(),
                };
                println!("{} {} {}", tag.dimmed(), pat.cyan(), forced_str.yellow());
            }
        }

        let parent_depth = nodes[node_id].depth;
        // The parent's forced-expansion is a monotone lower bound on each child's,
        // so it early-exits the children's forced scans (the hot path on DSRs).
        let parent_forced = nodes[node_id].forced;
        let mut successors: Vec<SearchState<F, O>> = match nodes[node_id].state.enumerate_successor_actions(&shared, args.opt_dominance_reuse, args.opt_useless_inline, max_arity, &mut dominance_hits, &mut useless_inline_hits) {
            SuccessorEnum::Dominant { child, .. } => vec![child],
            SuccessorEnum::All { actions, rank } => actions.into_iter().map(|(a, _)| nodes[node_id].state.apply_action(&a, &shared, true, Some(&rank))).collect(),
        };

        if let Some(k) = args.max_forced_expansion.0 {
            // Safe to run on the post-dominance successor set: dominance
            // short-circuits preserve forced expansion. dominant-reuse: doesn't
            // change anything about the matching term at each site. useless-inline:
            // replaces a variable with its minimal term, so preserves cost. Both
            // don't change the set of matches.
            //
            // The cap is given in symbols; scale to the family's cost units.
            let cap = k as i64 * F::symbol_cost(&shared.egraph.analysis.weights) as i64;
            successors.retain(|c| c.within_forced_expansion_cap(&shared, cap));
        }

        successors.retain(|c| c.within_match_set_cap(args.max_match_set));

        for child_state in successors {
            if let Some(ref follow) = shared.follow
                && !child_state.matches_follow(follow)
            {
                continue;
            }
            if let Some(s) = seen.as_mut()
                && s.check_and_insert(child_state.pattern.clone(), child_state.pattern.frozen_mask())
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
            let (child_prio, child_forced) = priority(strategy, child_cost, child_depth, &child_state, &shared, parent_forced);
            let child_id = nodes.len();

            let cost_to_beat = best.as_ref().map_or(original_size, |(c, _, _)| *c);
            let arity = child_state.pattern.vars.len();
            // KNOWN DIVERGENCE FROM SMC: this update is *not* guarded by
            // `shared.follow.is_none()`, unlike its counterpart at smc.rs:135. In
            // `--follow` mode best-first therefore records the cheapest matching
            // *prefix* as `best`, whereas SMC records only an exact follow match
            // (and returns `None` if the budget runs out first). The non-prefix
            // children are already filtered out above (see :199), so `best` is
            // always a valid follow-prefix; the two backends just disagree on
            // what they report when no exact hit is reached within budget. This
            // is intentionally left as-is: follow mode is a reachability check
            // and none of the follow tests depend on which backend's
            // budget-exhaustion behaviour is used.
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
                forced: child_forced,
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
    let (seen_len, seen_hits, seen_secs, egraph_hits, exact_hits, full_dom_hits) = seen.as_ref().map_or((0, 0, 0.0, 0, 0, 0), |s| (s.len(), s.hits, s.time.as_secs_f64(), s.egraph_hits, s.exact_hits, s.full_dom_hits));
    println!("{} {}", "seen-set size:".dimmed(), seen_len.to_string().bold());
    println!(
        "{} {} {} {}",
        "seen-set hits:".dimmed(),
        seen_hits.to_string().bold(),
        format!("(egraph hits: {egraph_hits}, exact hits: {exact_hits}, full-dom hits: {full_dom_hits})"),
        format!("(time: {:.3}s)", seen_secs).dimmed(),
    );
    if let Some(s) = seen.as_mut() {
        let saturate_each = s.saturate_each_on();
        let every = if s.saturate_dynamic_on() {
            format!("dynamic (now {}, {} effect / {} no-effect flushes)", s.saturate_every(), s.dynamic_effects, s.dynamic_noeffects)
        } else {
            args.seen_egraph_saturate_every.max(1).to_string()
        };
        let decider = if s.egraph_decides_on() { "egraph" } else { "map" };
        let per_insert_secs = s.egraph_time.as_secs_f64();
        let sat_calls = s.saturate_calls;
        let avg_ms = if sat_calls > 0 { per_insert_secs * 1000.0 / sat_calls as f64 } else { 0.0 };
        let recexpr_secs = s.recexpr_time.as_secs_f64();
        let search_secs = s.egraph_search_time.as_secs_f64();
        let apply_secs = s.egraph_apply_time.as_secs_f64();
        let rebuild_secs = s.egraph_rebuild_time.as_secs_f64();
        let audit = s.audit_seen_egraph();
        println!("{}", "── seen-egraph audit ──".dimmed());
        println!(
            "{} {} {} {}",
            "  rewrites:".dimmed(),
            format!("{} rules, saturate-each={saturate_each}, every={every}, decider={decider}", audit.num_rules).bold(),
            format!("(audit pass: {} firings over {} iters,", audit.applications, audit.iterations).dimmed(),
            format!("stop {})", audit.stop_reason).dimmed(),
        );
        println!("{} {}", "  per-insert saturation:".dimmed(), format!("{per_insert_secs:.3}s over {sat_calls} runs ({avg_ms:.3}ms avg)").bold());
        println!("{} {}", "    of which (egg):".dimmed(), format!("search {search_secs:.3}s, apply {apply_secs:.3}s, rebuild {rebuild_secs:.3}s").bold());
        println!("{} {}", "  frozen-recexpr build:".dimmed(), format!("{recexpr_secs:.3}s").bold());
        println!(
            "{} {} {}",
            "  before saturation:".dimmed(),
            format!("{} enodes, {} eclasses", audit.nodes_before, audit.classes_before).bold(),
            format!("({} distinct Root classes)", audit.root_classes_before).dimmed(),
        );
        println!(
            "{} {} {}",
            "  after saturation: ".dimmed(),
            format!("{} enodes, {} eclasses", audit.nodes_after, audit.classes_after).bold(),
            format!("({} distinct Root classes)", audit.root_classes_after).dimmed(),
        );
        // `distinct_root_classes` already asserts every inserted term is present
        // (hard correctness). Here we just compare counts: genuine inserts vs the
        // true distinct DSR-classes after full saturation. Equal under eager
        // saturation; batching (or saturate-off) defers/forgoes merges, so some
        // genuine inserts are really DSR-equivalent repeats — pure precision loss,
        // never unsound.
        let deferred = audit.unique_inserted - audit.root_classes_after;
        println!(
            "{} {} {}",
            "  inserts vs DSR-classes:".dimmed(),
            format!("{} genuine inserts → {} true DSR-classes", audit.unique_inserted, audit.root_classes_after).bold(),
            if deferred == 0 { "✓ fully deduped".green().to_string() } else { format!("(Δ{deferred} not deduped: batch/off)").yellow().to_string() },
        );
    }
    println!("{} {}", "dominance hits:".dimmed(), dominance_hits.to_string().bold());
    lower_bound_pruner.print_stats();
    println!("{} {}", "useless-frozen hits:".dimmed(), useless_frozen_hits.to_string().bold());
    println!("{} {}", "useless-inline hits:".dimmed(), useless_inline_hits.to_string().bold());
    println!("{} {} {}", "compute_cost calls:".dimmed(), cost_calls.to_string().bold(), format!("(time: {:.3}s)", cost_time.as_secs_f64()).dimmed());
    println!("{} {}", "total search time:".dimmed(), format!("{:.3}s", total_elapsed.as_secs_f64()).bold());

    let best_node_id = best.as_ref().map(|(_, id, _)| *id);
    // Canonicalize the winner's var numbering (DFS first-appearance) before it's
    // handed off for output/rewrite.
    let best_pair = best.map(|(cost, id, selection)| {
        let mut pair = SearchStateWithCostSelection { state: nodes[id].state.clone(), selection };
        pair.canonicalize();
        (cost, pair)
    });

    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, pair))) = (best_found_at, best_pair.as_ref()) {
        println!("{} {}", "best found at expansion:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), pair.state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / *cost as f64).green().bold());
    }

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
        heap_size_at_end: heap.len(),
        data: shared.into_data(),
        tree_log,
    }
}
