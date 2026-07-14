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

/// A per-tuple *value multiset* signature: `(root, commutative hash of bindings)`.
/// Column renaming only permutes a tuple's bindings, so it preserves this
/// signature. Containment `proj ⊆ seen` under any renaming therefore requires every
/// projected tuple's signature to be an actual `seen` tuple — a hash-lookup
/// necessary condition that rejects non-dominators with no renaming search. The
/// hash is a *sum* of per-value hashes, so it is order-independent and a projected
/// tuple's signature is the full tuple's minus the dropped value's hash — computed
/// in O(1) with no allocation. Hash collisions only admit extra candidates, which
/// the exact renaming verify then rejects, so pruning stays sound.
type TupleSig = (egg::Id, u64);

/// Hash of a single binding value (used additively to build [`TupleSig`]).
fn value_hash(id: egg::Id) -> u64 {
    (usize::from(id) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Commutative (sum) hash of a tuple's bindings — the multiset signature body.
fn multiset_hash(bindings: &[egg::Id]) -> u64 {
    bindings.iter().copied().map(value_hash).fold(0u64, u64::wrapping_add)
}

/// A seen reduct candidate: its footprint, its variable-frozen mask (one bool per
/// variable, in variable order — the freeze-rule flexibility that the containment
/// check must respect so pruning stays sound), its per-variable columns (used to
/// prune the renaming search in [`dominated_any_drop`]), and a *multiset* of its
/// per-tuple value-multiset signatures — how many tuples carry each signature.
/// Since a renaming is a bijection, distinct `Q` tuples map to distinct `P` tuples,
/// so `P` must hold at least as many tuples of each signature as the projection;
/// the counts make that the necessary-condition filter (strictly stronger than set
/// membership).
type Reduct = (Footprint, Vec<bool>, Vec<Column>, rustc_hash::FxHashMap<TupleSig, u32>);

/// Seen reducts of one arity, with an inverted index from each per-tuple value
/// multiset ([`TupleSig`]) to the reducts whose footprint contains a tuple with
/// that signature. A projection `Q` can be dominated by `P` only if every tuple of
/// `Q` matches a `seen` tuple signature, so the candidate scan only visits reducts
/// sharing one of `Q`'s tuple signatures — turning an O(bin size) scan into
/// O(posting list). Signatures are far more selective than single bindings, so the
/// lists stay short even on dense domains. See [`best_first`]'s check.
#[derive(Default)]
struct VspBin {
    reducts: Vec<Reduct>,
    index: rustc_hash::FxHashMap<TupleSig, Vec<u32>>,
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
/// variable order)` tuples, or `None` when the substitution count exceeds `cap`
/// (materializing it would blow up). Bindings are kept in variable order rather
/// than sorted, so [`dominated_any_drop`] can decide variable renaming exactly (a
/// single global permutation) instead of over-approximating with a per-tuple
/// multiset. A single materialization serves every dropped variable — projections
/// are taken by omitting a column, never re-materialized.
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
/// variable-frozen mask, and its per-variable columns — or `None` when it is too
/// expensive to build/check. Materialized once per pattern and reused for both the
/// domination check (as a candidate over every dropped variable) and, if the
/// pattern survives, its registration as a seen reduct.
///
/// `cap` bounds the substitution count so materializing the footprint can't blow
/// up; a pattern above it is skipped (neither checked nor registered). Also
/// precomputes the per-tuple value-multiset signatures ([`TupleSig`]) used to
/// reject non-dominators cheaply.
fn make_reduct<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>, cap: usize) -> Option<Reduct> {
    let arity = state.pattern.vars.len();
    let fp = footprint_set(state, cap)?;
    let cols = columns(&fp, arity);
    let mut sigs: rustc_hash::FxHashMap<TupleSig, u32> = rustc_hash::FxHashMap::default();
    for (root, b) in &fp {
        *sigs.entry((*root, multiset_hash(b))).or_insert(0) += 1;
    }
    Some((fp, state.pattern.var_frozen.clone(), cols, sigs))
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
/// The heavy lifting is avoided by a **value-multiset necessary condition**: a
/// column renaming only permutes a tuple's bindings, so for `Q ⊆ seen` to hold,
/// every projected tuple's multiset signature ([`TupleSig`]) must be an actual
/// `seen` tuple. This is a hash lookup per tuple, and it rejects the overwhelming
/// majority of candidates — which share some bindings but no whole tuple — without
/// any renaming search. It is exact (up to hash collisions, which the verify below
/// rejects): if a projected tuple's multiset is absent from `seen`, no `π` can
/// place it there.
///
/// Only when the filter passes do we search for `π`: the per-column test
/// `full_col[i] ⊆ seen_col[j]` (with the flexibility condition) is independent of
/// the dropped variable, so it is computed once into the compatibility matrix
/// `compat[i][j]`; then for each `drop` we find *one* compat-respecting bijection
/// from `seen`'s slots to `full`'s surviving variables by augmenting-path bipartite
/// matching (Kuhn's algorithm — polynomial) and verify that single renaming. We
/// deliberately do not enumerate alternative matchings: checking one keeps the cost
/// bounded with no tuning knob, at the price of occasionally declining an ambiguous
/// domination (sound — declining to prune never changes correctness).
///
/// `full`/`full_hash` are the child's footprint tuples and their multiset hashes
/// (parallel slices, precomputed once by the caller); a projected tuple's hash is
/// `full_hash[t] − value_hash(dropped binding)`, computed here in O(1).
#[allow(clippy::too_many_arguments)]
fn dominated_any_drop(full: &[(egg::Id, Vec<egg::Id>)], full_hash: &[u64], full_cols: &[Column], full_frozen: &[bool], seen: &Footprint, seen_cols: &[Column], seen_frozen: &[bool], seen_sigs: &rustc_hash::FxHashMap<TupleSig, u32>) -> bool {
    let arity = full_frozen.len();
    let n = seen_frozen.len(); // = arity - 1
    // Value-multiset filter first (pure hash lookups): a dropped variable can only
    // yield containment if the projection's tuples embed into `seen`'s. Because a
    // renaming is a bijection, distinct projected tuples need distinct seen tuples,
    // so `seen` must carry at least as many tuples of each signature as the
    // projection. We count over the child's *full* footprint, which over-counts by
    // a uniform fiber (the tuples that collapse together when `drop` is projected
    // out), so we divide the counts by their GCD to recover the true per-signature
    // multiplicity before comparing to `seen`. Any missing signature rejects
    // immediately; the GCD pass runs only once every signature is present. Drops
    // failing this are rejected before the far more expensive compatibility matrix,
    // so `compat` is computed lazily only once some drop survives.
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let mut need: rustc_hash::FxHashMap<TupleSig, u32> = rustc_hash::FxHashMap::default();
    let mut passes = |drop: usize| {
        need.clear();
        for ((root, b), &h) in full.iter().zip(full_hash) {
            let sig = (*root, h.wrapping_sub(value_hash(b[drop])));
            if !seen_sigs.contains_key(&sig) {
                return false; // a projected tuple's multiset is absent: no domination
            }
            *need.entry(sig).or_insert(0) += 1;
        }
        let g = need.values().copied().reduce(gcd).unwrap_or(1).max(1);
        need.iter().all(|(sig, &cnt)| cnt <= g * seen_sigs[sig])
    };
    let mut compat: Option<Vec<bool>> = None;
    (0..arity).filter(|&drop| passes(drop)).any(|drop| {
        let compat = compat.get_or_insert_with(|| {
            // Drop-independent compatibility: `c[i * n + j]` iff `full` variable `i`
            // may rename onto `seen` slot `j`.
            let mut c = vec![false; arity * n];
            for i in 0..arity {
                for j in 0..n {
                    c[i * n + j] = (!seen_frozen[j] || full_frozen[i]) && full_cols[i].len() <= seen_cols[j].len() && full_cols[i].iter().all(|t| seen_cols[j].contains(t));
                }
            }
            c
        });
        // Find *one* compat-respecting bijection from seen slots to surviving `full`
        // variables via augmenting-path bipartite matching (Kuhn's algorithm —
        // polynomial), then verify that single renaming. We deliberately do not
        // backtrack over alternative matchings: verifying one keeps the work bounded
        // with no tuning knob, at the cost of occasionally declining an ambiguous
        // domination (sound — declining never changes correctness).
        let mut slot_of_col = vec![usize::MAX; arity]; // full column -> matched seen slot
        #[allow(clippy::too_many_arguments)]
        fn augment(j: usize, drop: usize, arity: usize, n: usize, compat: &[bool], slot_of_col: &mut [usize], visited: &mut [bool]) -> bool {
            for i in 0..arity {
                if i == drop || !compat[i * n + j] || visited[i] {
                    continue;
                }
                visited[i] = true;
                if slot_of_col[i] == usize::MAX || augment(slot_of_col[i], drop, arity, n, compat, slot_of_col, visited) {
                    slot_of_col[i] = j;
                    return true;
                }
            }
            false
        }
        if !(0..n).all(|j| augment(j, drop, arity, n, compat, &mut slot_of_col, &mut vec![false; arity])) {
            return false; // no perfect matching: no renaming exists for this drop
        }
        let mut perm = vec![0usize; n];
        for (i, &j) in slot_of_col.iter().enumerate() {
            if j != usize::MAX {
                perm[j] = i;
            }
        }
        full.iter().all(|(root, b)| seen.contains(&(*root, perm.iter().map(|&i| b[i]).collect())))
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
    // to the bin's inverted tuple-signature index (once per distinct signature).
    let vsp_register = |reduct: Reduct, seen: &mut rustc_hash::FxHashMap<usize, VspBin>| {
        let bin = seen.entry(reduct.1.len()).or_default();
        if bin.reducts.len() < VSP_MAX_CANDIDATES {
            let idx = bin.reducts.len() as u32;
            for sig in reduct.3.keys() {
                bin.index.entry(*sig).or_default().push(idx);
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
            if let Some((ref fp, ref vf, ref cols, _)) = child_reduct {
                let arity = vf.len();
                if arity >= 1
                    && let Some(bin) = vsp_seen.get(&(arity - 1))
                {
                    // Footprint tuples and their multiset hashes, once, for the
                    // value-multiset filter (a renaming preserves the multiset).
                    let full: Vec<(egg::Id, Vec<egg::Id>)> = fp.iter().cloned().collect();
                    let full_hash: Vec<u64> = full.iter().map(|(_, b)| multiset_hash(b)).collect();
                    // Gather candidates via the tuple-signature index: a dominator of
                    // the projection dropping `v` must contain *every* projected tuple,
                    // so it lies in the *intersection* of the projection's tuple-signature
                    // posting lists. We intersect up to `VSP_GATHER_TUPLES` of them —
                    // enough to narrow sharply even when individual multisets are common,
                    // without scanning the whole footprint. Any dominating drop is still
                    // covered (it must contain all of these tuples), so this is complete.
                    // Posting lists are sorted (indices pushed in increasing order), so
                    // membership is a binary search; we walk the shortest list.
                    const VSP_GATHER_TUPLES: usize = 4;
                    let mut cand: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
                    let sig_v = |t: usize, v: usize| (full[t].0, full_hash[t].wrapping_sub(value_hash(full[t].1[v])));
                    let m = full.len().min(VSP_GATHER_TUPLES);
                    for v in 0..arity {
                        // Every one of the first `m` projected tuples must be present;
                        // if any is absent no seen reduct can dominate this drop.
                        let Some(lists) = (0..m).map(|t| bin.index.get(&sig_v(t, v)).map(Vec::as_slice)).collect::<Option<Vec<_>>>() else {
                            continue;
                        };
                        let Some(shortest) = lists.iter().min_by_key(|l| l.len()) else {
                            continue;
                        };
                        cand.extend(shortest.iter().copied().filter(|idx| lists.iter().all(|l| l.binary_search(idx).is_ok())));
                    }
                    let dominated = cand.iter().any(|&idx| {
                        let (sfp, sf, sc, ssig) = &bin.reducts[idx as usize];
                        dominated_any_drop(&full, &full_hash, cols, vf, sfp, sc, sf, ssig)
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
