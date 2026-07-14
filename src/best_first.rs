use clap::ValueEnum;
use colored::Colorize;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::cost::{CostScratch, CostSelection, SearchStateWithCostSelection, compute_cost_and_select};
use crate::footprint::FootprintTracker;
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
    /// Lexicographic `(forced-expansion, cost)`
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
        SearchPriority::ForcedThenCost => match state.forced_expansion_argmin(shared, forced_lower_bound) {
            // clamp to 0 because anything <= 0 means no forced expansion
            Some((forced, _)) => ((forced.max(0) as usize, cost), forced),
            None => ((usize::MAX, cost), i64::MAX),
        },
    }
}

/// True iff every every e-node in the e-graph has the same cost as all
/// other e-nodes in its class.
fn cost_balanced<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>) -> bool {
    let weights = egraph.analysis.weights;
    egraph.classes().all(|c| c.nodes.iter().all(|n| n.discriminant().intrinsic_size(&weights) + n.children().iter().map(|&ch| egraph[ch].data.size).sum::<u32>() == c.data.size))
}

/// A match footprint: `(root, variable bindings in fixed variable order)` tuples.
type Footprint = rustc_hash::FxHashSet<(egg::Id, Vec<egg::Id>)>;

/// A variable's *column*: the set of `(root, value)` pairs it takes across a
/// footprint. Renaming variable `i` of one pattern onto variable `j` of another
/// requires `column[i] ⊆ column[j]` — a per-variable necessary condition that
/// collapses the renaming search from factorial to a constrained bipartite
/// matching (the containment analogue of a marginal hash).
type Column = rustc_hash::FxHashSet<(egg::Id, egg::Id)>;

/// A seen reduct candidate: its footprint, its variable-frozen mask (one bool per
/// variable, in variable order — the freeze-rule flexibility that the containment
/// check must respect so pruning stays sound), and its per-variable columns (used
/// to prune the renaming search in [`dominated_any_drop`]).
type Reduct = (Footprint, Vec<bool>, Vec<Column>);

/// Seen reducts of one arity, with an inverted index from each `(root, value)`
/// pair to the reducts whose footprint contains it. A projection `Q` can be
/// dominated by `P` only if every pair of `Q` lies in `P`'s footprint, so the
/// candidate scan only visits reducts sharing `Q`'s rarest pair — turning an
/// O(bin size) scan into O(shortest posting list). See [`best_first`]'s check.
#[derive(Default)]
struct VspBin {
    reducts: Vec<Reduct>,
    index: rustc_hash::FxHashMap<(egg::Id, egg::Id), Vec<u32>>,
}

/// Per-variable columns of a footprint: `columns(fp)[i]` is the set of
/// `(root, value)` pairs that variable `i` takes across every footprint tuple.
fn columns(fp: &Footprint, arity: usize) -> Vec<Column> {
    let mut cols = vec![Column::default(); arity];
    for (root, b) in fp {
        for (i, v) in b.iter().enumerate() {
            cols[i].insert((*root, *v));
        }
    }
    cols
}

/// A pattern's match footprint as a set of `(root, variable bindings in fixed
/// variable order)` tuples. Returns `None` when the full-substitution count
/// exceeds `cap` (materializing it would blow up). Bindings are kept in variable
/// order rather than sorted, so [`dominated_any_drop`] can decide variable
/// renaming exactly (a single global permutation) instead of over-approximating
/// with a per-tuple multiset. A single materialization serves every dropped
/// variable — projections are taken by omitting a column, never re-materialized.
fn footprint_set<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>, cap: usize) -> Option<Footprint> {
    let arity = state.pattern.vars.len();
    let total: usize = state.matches.iter().map(|m| m.num_substs()).sum();
    if total > cap {
        return None;
    }
    let mut set = rustc_hash::FxHashSet::default();
    for m in &state.matches {
        for full in crate::factor::factors_product(&m.factors) {
            set.insert((m.root_eclass, (0..arity).map(|k| full[k]).collect()));
        }
    }
    Some(set)
}

/// The variable-subset reduct of a pattern: its full match footprint, its
/// variable-frozen mask, and its per-variable columns — or `None` when the
/// footprint exceeds `cap`. Materialized once per pattern and reused for both the
/// domination check (as a candidate over every dropped variable) and, if the
/// pattern survives, its registration as a seen reduct.
fn make_reduct<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>, cap: usize) -> Option<Reduct> {
    let arity = state.pattern.vars.len();
    let fp = footprint_set(state, cap)?;
    let cols = columns(&fp, arity);
    Some((fp, state.pattern.var_frozen.clone(), cols))
}

/// Whether the seen reduct `P` (`seen`) dominates the projection of `full` under
/// *some single global* variable renaming `π`, for *some* dropped variable. For a
/// fixed `drop`, the projection `Q` (omitting variable `drop`) is dominated iff
/// one `π` satisfies both: footprint containment (every projected tuple
/// `(root, b) ∈ Q` has `(root, b∘π) ∈ seen`) and `P` being at least as flexible as
/// `Q` (wherever `P` freezes a shared variable, `Q` freezes the corresponding one
/// — `seen_frozen[j] ⇒ full_frozen[π(j)]`).
///
/// Both must hold under the *same* `π`. The flexibility condition is essential:
/// under the freeze rule a more-frozen `P` reaches fewer expansions than `Q`, so
/// it would not actually dominate `Q` and pruning would lose an optimum. (Mirrors
/// `SeenTracker`'s `frozen_subset`, threaded through the renaming.)
///
/// The per-column renaming test `full_col[i] ⊆ seen_col[j]` (with the flexibility
/// condition) is *independent of which variable is dropped*, so it is computed
/// once into the compatibility matrix `compat[i][j]` and reused for every `drop`.
/// For each `drop`, a feasible renaming is a bijection from `seen`'s slots to
/// `full`'s surviving variables respecting `compat`; we backtrack over it (most-
/// constrained slot first) and verify full tuple containment at each complete
/// renaming — iterating `full` directly, so no per-drop projection is materialized.
/// When columns are distinct the matching is near-unique; `budget` bounds the
/// degenerate case (many equal columns), on exhaustion returning `false`
/// (declining to prune is always sound).
fn dominated_any_drop(full: &Footprint, full_cols: &[Column], full_frozen: &[bool], seen: &Footprint, seen_cols: &[Column], seen_frozen: &[bool]) -> bool {
    let arity = full_frozen.len();
    let n = seen_frozen.len(); // = arity - 1
    // Drop-independent compatibility: `compat[i * n + j]` iff `full` variable `i`
    // may rename onto `seen` slot `j`. The column-containment probe short-circuits
    // on the size gate, so incompatible pairs are cheap.
    let mut compat = vec![false; arity * n];
    for i in 0..arity {
        for j in 0..n {
            compat[i * n + j] = (!seen_frozen[j] || full_frozen[i]) && full_cols[i].len() <= seen_cols[j].len() && full_cols[i].iter().all(|t| seen_cols[j].contains(t));
        }
    }
    (0..arity).any(|drop| {
        // Feasibility: every seen slot needs an admissible surviving variable.
        if (0..n).any(|j| !(0..arity).any(|i| i != drop && compat[i * n + j])) {
            return false;
        }
        // Fill the most-constrained slots first so backtracking fails fast.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&j| (0..arity).filter(|&i| i != drop && compat[i * n + j]).count());
        let mut perm = vec![0usize; n];
        let mut used = vec![false; arity];
        used[drop] = true; // the dropped variable is not part of the renaming
        let mut budget = 10_000usize;
        #[allow(clippy::too_many_arguments)]
        fn rec(k: usize, n: usize, order: &[usize], compat: &[bool], arity: usize, used: &mut [bool], perm: &mut [usize], full: &Footprint, seen: &Footprint, budget: &mut usize) -> bool {
            if k == order.len() {
                return full.iter().all(|(root, b)| seen.contains(&(*root, perm.iter().map(|&i| b[i]).collect())));
            }
            let j = order[k];
            for i in 0..arity {
                if used[i] || !compat[i * n + j] {
                    continue;
                }
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                used[i] = true;
                perm[j] = i;
                if rec(k + 1, n, order, compat, arity, used, perm, full, seen, budget) {
                    return true;
                }
                used[i] = false;
            }
            false
        }
        rec(0, n, &order, &compat, arity, &mut used, &mut perm, full, seen, &mut budget)
    })
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
}

/// One node in the in-memory search tree.
struct Node<F: LanguageFamily, O: StitchOp> {
    state: SearchState<F, O>,
    depth: usize,
    /// Lower bound on cost of any descendant; only set when `--lower-bound` is on.
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
/// and pushes the survivors back onto the heap. Stops at `num_steps` pops, the
/// `time_limit`, or an empty heap (completion). If neither budget is set, runs
/// to completion. (No `dead_runs` cutoff: the search is systematic, so "no
/// recent improvement" just means we're grinding through a less promising branch.)
pub fn best_first<F: LanguageFamily, O: StitchOp>(data: crate::shared::SharedData<F, O>, args: &crate::Args) -> BestFirstResult<F, O> {
    let (shared, cost_cache, original_size) = setup_search(data, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let budget = args.num_steps;
    let time_limit = args.time_limit.map(std::time::Duration::from_secs_f64);
    let max_arity = args.max_arity;
    let no_zero_arity = args.no_zero_arity;
    // ForcedThenCost reduces to Cost when the e-graph is cost-balanced
    let strategy = if matches!(args.priority, SearchPriority::ForcedThenCost) && cost_balanced::<F, O>(&shared.egraph) {
        println!("{}", "rules are cost-balanced: ordering by cost (forced-expansion ordering not needed)".dimmed());
        SearchPriority::Cost
    } else {
        args.priority
    };
    let initial_state = SearchState::new(&shared, args.freeze_rule.resolve(true));
    let mut scratch = CostScratch::new(&shared.egraph);
    let initial_cost = compute_cost_and_select(&shared.egraph, shared.root, &cost_cache, &mut scratch, &initial_state, shared.check_slow).cost;
    // No parent, so no lower bound: scan fully.
    let (initial_prio, initial_forced) = priority(strategy, initial_cost, 0, &initial_state, &shared, i64::MIN);

    let mut nodes: Vec<Node<F, O>> = Vec::new();
    // Heap key: `(priority, insertion-order)`. `priority` is itself a tuple so
    // it can order lexicographically (e.g. forced-expansion then cost);
    // insertion order breaks remaining ties to stay deterministic.
    let mut heap: BinaryHeap<Reverse<((usize, usize), usize)>> = BinaryHeap::new();
    let mut seen: Option<SeenTracker<F, O>> = args.opt_seen.then(SeenTracker::new);
    let mut footprints: Option<FootprintTracker> = args.opt_dedup_by_match.then(FootprintTracker::new);
    // `--opt-var-subset`: already-seen patterns' footprints, keyed by variable
    // count, serve as reduct candidates. `vsp_hits` counts prunes. The caps bound
    // memory/materialization on domains with very large match sets.
    const VSP_CAP: usize = 20_000;
    const VSP_MAX_CANDIDATES: usize = 20_000;
    let mut vsp_seen: rustc_hash::FxHashMap<usize, VspBin> = rustc_hash::FxHashMap::default();
    let mut vsp_hits: usize = 0;
    // Registers an already-computed reduct (see `make_reduct`) as a seen candidate,
    // binned by its variable count. Reuses the materialization done for the
    // domination check rather than recomputing the footprint, and adds the reduct
    // to the bin's inverted `(root, value)`-pair index (once per distinct pair).
    let vsp_register = |reduct: Reduct, seen: &mut rustc_hash::FxHashMap<usize, VspBin>| {
        let bin = seen.entry(reduct.1.len()).or_default();
        if bin.reducts.len() < VSP_MAX_CANDIDATES {
            let idx = bin.reducts.len() as u32;
            let flat: rustc_hash::FxHashSet<(egg::Id, egg::Id)> = reduct.2.iter().flatten().copied().collect();
            for pair in flat {
                bin.index.entry(pair).or_default().push(idx);
            }
            bin.reducts.push(reduct);
        }
    };

    nodes.push(Node {
        state: initial_state.clone(),
        depth: 0,
        lower_bound: None,
        forced: initial_forced,
    });
    heap.push(Reverse((initial_prio, 0)));
    if args.opt_var_subset
        && let Some(reduct) = make_reduct(&initial_state, VSP_CAP)
    {
        vsp_register(reduct, &mut vsp_seen);
    }
    if let Some(s) = seen.as_mut() {
        s.check_and_insert(initial_state.pattern.clone(), initial_state.pattern.frozen_mask());
    }
    if let Some(fp) = footprints.as_mut() {
        // The initial state is node 0; a deferred representative re-reads its
        // match set by index from the (append-only) node array.
        fp.check_state(&initial_state, &shared, 0, &|i| &nodes[i].state.matches[..]);
    }

    let mut best: Option<(usize, usize, CostSelection)> = None; // (cost, node_id, selection)
    let mut best_found_at: Option<usize> = None;
    let mut best_history: Vec<BestHistoryEntry> = Vec::new();
    let mut num_expansions: usize = 0;
    let mut cost_calls: usize = 0;
    let mut cost_time: Duration = Duration::ZERO;
    let mut dominance_hits: usize = 0;
    let mut lower_bound_pruner = LowerBoundPruner::new(args.lower_bound.resolve(true));
    let mut useless_frozen_hits: usize = 0;
    let mut useless_inline_hits: usize = 0;
    // Set when a new best reaches the `--compression-limit` cumulative target;
    // the search breaks after the child node is pushed (so `best`'s node id is valid).
    let mut hit_compression_limit = false;
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

        if args.verbose || args.verbose_forced_expansion || args.verbose_match_structure {
            let tag = format!("[expansion {}]", num_expansions);
            let pat = nodes[node_id].state.pattern.to_string();
            if args.verbose {
                println!("{} {} {}", tag.dimmed(), "expanding:".dimmed(), pat.clone().cyan());
            }
            if args.verbose_match_structure {
                crate::logging::print_match_structure(&nodes[node_id].state.matches, 10);
            }
            if args.verbose_forced_expansion {
                let forced_str = match nodes[node_id].state.forced_expansion_argmin(&shared, i64::MIN) {
                    Some((e, root)) => format!("[forced-expansion={} @root={}]", e, nodes[node_id].state.min_term(&shared, root)),
                    None => "[forced-expansion=- (no in-extraction root)]".to_string(),
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

            // Variable-subset footprint pruning: prune `Q` if projecting its
            // footprint onto its variables minus one is contained (under a single
            // global renaming) in an already-seen pattern with one fewer variable.
            // Any `arity >= 1` is a candidate, including the 0-variable projection
            // (compare a lone-variable pattern to a seen concrete one): containment
            // forces `P ∈ R` at every match root `R`, and `Q[σ] ∈ R` too, so
            // `Q[σ] ≡ P` — the dropped variable is vacuous and `Q` is dominated.
            //
            // MUST stay before the `FootprintTracker` check below: that check is
            // deliberately last and hands the tracker `id = nodes.len()` on the
            // contract that a surviving child is pushed at that index (its deferred
            // representative reads the match set back by id). A prune placed *after*
            // it would break that contract — a later child would take the id — so
            // any successor prune has to happen first.
            // The child's footprint is materialized once here and reused both for
            // this check (over every dropped variable) and for registration below.
            let child_reduct = if args.opt_var_subset { make_reduct(&child_state, VSP_CAP) } else { None };
            if let Some((ref fp, ref vf, ref cols)) = child_reduct {
                let arity = vf.len();
                if arity >= 1
                    && let Some(bin) = vsp_seen.get(&(arity - 1))
                {
                    // Per surviving column, its rarest present pair (shortest posting
                    // list) and whether it holds a *dead* pair — one absent from the
                    // index, so no seen reduct contains it. A projection keeping a
                    // column with a dead pair can be dominated by nobody.
                    let mut dead = vec![false; arity];
                    let mut col_rarest: Vec<Option<(&(egg::Id, egg::Id), usize)>> = vec![None; arity];
                    for i in 0..arity {
                        for pair in &cols[i] {
                            match bin.index.get(pair) {
                                None => {
                                    dead[i] = true;
                                    break;
                                }
                                Some(list) => {
                                    if col_rarest[i].is_none_or(|(_, l)| list.len() < l) {
                                        col_rarest[i] = Some((pair, list.len()));
                                    }
                                }
                            }
                        }
                    }
                    // Gather the candidate reducts: for each dropped variable `v`, a
                    // dominator must contain *every* surviving column's rarest pair (each
                    // column must embed into some seen column), so it lies in the
                    // intersection of those pairs' posting lists. Lists are sorted (indices
                    // pushed in increasing order), so membership is a binary search.
                    let mut cand: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
                    for v in 0..arity {
                        if (0..arity).any(|i| i != v && dead[i]) {
                            continue; // a kept column has a dead pair: undominatable
                        }
                        let lists: Vec<&[u32]> = (0..arity).filter(|&i| i != v).filter_map(|i| col_rarest[i].map(|(p, _)| bin.index[p].as_slice())).collect();
                        match lists.iter().min_by_key(|l| l.len()) {
                            None => cand.extend(0..bin.reducts.len() as u32), // no pairs kept (0-var projection)
                            Some(shortest) => cand.extend(shortest.iter().copied().filter(|idx| lists.iter().all(|l| l.binary_search(idx).is_ok()))),
                        }
                    }
                    let dominated = cand.iter().any(|&idx| {
                        let (sfp, sf, sc) = &bin.reducts[idx as usize];
                        dominated_any_drop(fp, cols, vf, sfp, sc, sf)
                    });
                    if dominated {
                        vsp_hits += 1;
                        continue;
                    }
                }
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

            // Placed last as it is very expensive. `nodes.len()` is the index this
            // child will occupy (it is pushed unconditionally below if it survives),
            // letting a deferred representative re-read its match set on a collision.
            if let Some(fp) = footprints.as_mut()
                && fp.check_state(&child_state, &shared, nodes.len(), &|i| &nodes[i].state.matches[..])
            {
                continue;
            }

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
            if arity <= max_arity && !(no_zero_arity && arity == 0) && child_cost < cost_to_beat && (args.allow_useless_vars || !child_state.has_useless_var(&shared)) {
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
                // `--compression-limit` early stop: this best already reaches the
                // target ratio, so no better one is needed. Break after pushing the
                // node below (its id must be live for the winner extraction).
                if args.compression_limit.is_some_and(|limit| original_size as f64 / child_cost as f64 >= limit) {
                    hit_compression_limit = true;
                }
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

            if let Some(reduct) = child_reduct {
                vsp_register(reduct, &mut vsp_seen);
            }
            nodes.push(Node {
                state: child_state,
                depth: child_depth,
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

            if hit_compression_limit {
                println!("{}", format!("reached compression limit {:.3}", args.compression_limit.unwrap_or(0.0)).yellow());
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
    let (fp_len, fp_hits, fp_skips, fp_capped, fp_secs) = footprints.as_ref().map_or((0, 0, 0, 0, 0.0), |f| (f.len(), f.hits, f.proxy_skips, f.capped, f.time.as_secs_f64()));
    println!("{} {}", "footprint-set size:".dimmed(), fp_len.to_string().bold());
    println!("{} {} {}", "footprint-set hits:".dimmed(), fp_hits.to_string().bold(), format!("(proxy-skips: {}, capped: {}, time: {:.3}s)", fp_skips, fp_capped, fp_secs).dimmed());
    if args.opt_var_subset {
        println!("{} {}", "var-subset hits:".dimmed(), vsp_hits.to_string().bold());
    }
    println!("{} {}", "dominance hits:".dimmed(), dominance_hits.to_string().bold());
    lower_bound_pruner.print_stats();
    println!("{} {}", "useless-frozen hits:".dimmed(), useless_frozen_hits.to_string().bold());
    println!("{} {}", "useless-inline hits:".dimmed(), useless_inline_hits.to_string().bold());
    println!("{} {} {}", "compute_cost calls:".dimmed(), cost_calls.to_string().bold(), format!("(time: {:.3}s)", cost_time.as_secs_f64()).dimmed());
    println!("{} {}", "total search time:".dimmed(), format!("{:.3}s", total_elapsed.as_secs_f64()).bold());

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

    BestFirstResult {
        best: best_pair,
        original_size,
        best_history,
        best_found_at,
        num_expansions,
        heap_size_at_end: heap.len(),
        data: shared.into_data(),
    }
}
