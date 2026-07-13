use crate::candidates::enumerate_candidates;
use crate::factor::{Factor, factored_min};
use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights, enode_fv};
use crate::matching::MatchAtEClass;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{CostFunction, Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};

/// `egg::CostFunction` that mirrors `StitchAnalysis`'s weighted size:
/// `intrinsic_size(weights) + Σ child costs`. Use this rather than
/// `egg::AstSize` whenever extracting from a `StitchEgraph` — under
/// non-uniform `Weights` (e.g. `--sym-var-cost 100`) the AstSize-min term and
/// the weighted-min term diverge, and any size compared against `data.size`
/// (which is weighted) needs the extractor to agree.
pub struct WeightedSize {
    pub weights: Weights,
}

impl<L: StitchLanguage> CostFunction<L> for WeightedSize {
    type Cost = u64;
    fn cost<C: FnMut(Id) -> Self::Cost>(&mut self, enode: &L, mut costs: C) -> Self::Cost {
        let intrinsic = enode.discriminant().intrinsic_size(&self.weights) as u64;
        enode.children().iter().map(|&c| costs(c)).sum::<u64>() + intrinsic
    }
}

/// A candidate choice of `S_k` (`variable_indices[k]`) — the captured
/// pattern-internal fv promoted to higher-order args at metavar `?#k`. The set
/// of substs it permits is implicit: a subst is "compatible" iff its per-slot fv
/// `R^k ⊆ S_k` for every slot — only those can be rewritten, since
/// `wrap_subst_args` would otherwise hit a fv it can't permute onto a wrap-lam
/// slot. Because that test is per-slot (hence per-factor), the kept set is
/// recovered on demand by [`filter_factors_by_candidate`] without ever
/// materialising the product.
#[derive(Debug, Clone)]
pub struct CostCandidate {
    pub variable_indices: Vec<Vec<i32>>,
}

/// Selection picked by the cost optimizer: the candidate with minimum total
/// cost, plus that cost.
#[derive(Debug, Clone)]
pub struct CostSelection {
    pub cost: usize,
    pub candidate: CostCandidate,
}

/// A `SearchState` paired with the `CostSelection` the cost optimiser picked
/// for it. The search threads this back to `multiple_step_search` so the
/// downstream display + rewrite path doesn't have to redo the optimisation.
#[derive(Clone)]
pub struct SearchStateWithCostSelection<F: crate::lang::LanguageFamily, O: crate::lang::StitchOp> {
    pub state: crate::search::SearchState<F, O>,
    pub selection: CostSelection,
}

impl<F: LanguageFamily, O: StitchOp> SearchStateWithCostSelection<F, O> {
    /// Canonicalizes the winning state's variable order (DFS first-appearance)
    /// and remaps the selection's `variable_indices` by the same permutation.
    /// Run on the winner before output/rewrite. We permute the existing
    /// selection rather than re-running `compute_cost_and_select` because the
    /// optimiser breaks ties between equal-cost candidates nondeterministically
    /// (hash-order dependent) — recomputing could pick a different candidate and
    /// make output flaky. The permutation preserves the exact selection (cost is
    /// var-order invariant), just renumbered.
    pub fn canonicalize(&mut self) {
        let perm = self.state.canonicalize_vars();
        let vi = &self.selection.candidate.variable_indices;
        let mut new_vi = vec![Vec::new(); vi.len()];
        for (k, &nk) in perm.iter().enumerate() {
            new_vi[nk] = vi[k].clone();
        }
        self.selection.candidate.variable_indices = new_vi;
    }
}

/// Build a copy of `eclass` in `egraph` with every free DB leaf permuted onto
/// wrap-lam slots in preparation for the call-site β at `?#k`. For each free
/// `$n` at recursion depth `initial_depth` (so its index relative to our root
/// is `r = n - initial_depth`):
///   - `0 ≤ r < d_k` (pattern-internal): replaced by `$rank_map[r]` — the
///     wrap-lam slot that the body's η-app `(?#k … $r …)` re-binds at apply
///     time.
///   - `r ≥ d_k` (above-pattern free): replaced by `$(r - d_k + h)` — shifted
///     past the `h` wrap-lams so it continues referencing the call-site binder
///     it always did.
///
/// Bound leaves (`n < initial_depth`) pass through unchanged. Picks the
/// size-minimal enode per visited eclass; memoized per `(eclass, initial_depth)`.
pub(crate) fn shift_free_egraph<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, eclass: Id, d_k: u32, rank_map: &FxHashMap<i32, u32>, h: u32, initial_depth: u32, memo: &mut FxHashMap<(Id, u32), Id>) -> Id {
    let canonical = egraph.find(eclass);
    if let Some(&cached) = memo.get(&(canonical, initial_depth)) {
        return cached;
    }
    // No fv ≥ initial_depth → subtree is closed under our recursion's binders;
    // nothing to transform.
    if egraph[canonical].data.fv.iter().all(|&i| i < initial_depth as i32) {
        memo.insert((canonical, initial_depth), canonical);
        return canonical;
    }
    let weights = egraph.analysis.weights;
    let rep = egraph[canonical]
        .nodes
        .iter()
        .min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&c| egraph[c].data.size as u64).sum::<u64>())
        .expect("non-empty eclass")
        .clone();
    // Under intersection-fv semantics the size-minimal rep is also fv-minimal,
    // so its syntactic fv matches the eclass's analysis fv.
    let rep_fv = enode_fv(&rep, |c| &egraph[c].data.fv);
    assert_eq!(
        &rep_fv, &egraph[canonical].data.fv,
        "shift_free_egraph rep fv {:?} differs from eclass data.fv {:?}; intersection-fv assumption (min-size rep is fv-minimal) violated",
        rep_fv, egraph[canonical].data.fv
    );
    let disc = rep.discriminant();
    if let Some(n) = disc.de_bruijn_index() {
        let r = n - initial_depth as i32;
        let new_n = if r < d_k as i32 {
            let rank = *rank_map.get(&r).unwrap_or_else(|| panic!("captured DB index r={} for d_k={} not in slot's variable_indices map {:?}", r, d_k, rank_map));
            rank as i32 + initial_depth as i32
        } else {
            r - d_k as i32 + h as i32 + initial_depth as i32
        };
        let new_disc = F::map_discriminant(disc, |_| O::make_db_var(new_n).expect("higher-order capture requires a DB-var-bearing leaf op"));
        let new_id = egraph.add(F::make(new_disc, vec![]));
        memo.insert((canonical, initial_depth), new_id);
        return new_id;
    }
    let new_children: Vec<Id> = rep
        .children()
        .iter()
        .enumerate()
        .map(|(j, &c)| {
            let child_depth = initial_depth + if disc.binds_child(j) { 1 } else { 0 };
            shift_free_egraph::<F, O>(egraph, c, d_k, rank_map, h, child_depth, memo)
        })
        .collect();
    let new_id = egraph.add(F::make(disc, new_children));
    memo.insert((canonical, initial_depth), new_id);
    new_id
}

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Eclasses reachable from `root`, in postorder (children before parents).
    /// `solve` iterates this so child sizes settle before their parents reconsider.
    visit_order: Vec<Id>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    parents_of: FxHashMap<Id, Vec<Id>>,
}

impl CostCache {
    /// Builds the cache from the egraph rooted at `root`.
    pub fn new<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> Self {
        let mut parents_of = FxHashMap::<Id, Vec<Id>>::default();
        for class in egraph.classes() {
            for enode in &class.nodes {
                for &child in enode.children() {
                    parents_of.entry(egraph.find(child)).or_default().push(class.id);
                }
            }
        }

        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut visited = vec![false; max_id + 1];
        let mut visit_order: Vec<Id> = Vec::new();
        let mut stack: Vec<Result<Id, Id>> = vec![Err(egraph.find(root))]; // Err=enter, Ok=exit
        let mut on_stack = FxHashSet::<Id>::default();
        while let Some(state) = stack.pop() {
            match state {
                Err(id) => {
                    if visited[usize::from(id)] || !on_stack.insert(id) {
                        continue;
                    }
                    stack.push(Ok(id));
                    for enode in &egraph[id].nodes {
                        for &child in enode.children() {
                            stack.push(Err(egraph.find(child)));
                        }
                    }
                }
                Ok(id) => {
                    on_stack.remove(&id);
                    visited[usize::from(id)] = true;
                    visit_order.push(id);
                }
            }
        }

        Self { visit_order, parents_of }
    }
}

/// Reusable allocations for repeated cost computations. Build once with `new(egraph)`
/// and pass `&mut` to `compute_cost` / `compute_size_for_candidate` to avoid reallocating across calls.
pub struct CostScratch {
    pub runner: RunnerScratch,
    pub rewrite: RewriteScratch,
}

impl CostScratch {
    /// Builds the scratch space for a given egraph. The egraph's per-eclass weighted
    /// size is captured into `runner.original` here and reused across all subsequent calls.
    pub fn new<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> Self {
        Self {
            runner: RunnerScratch::new(egraph),
            rewrite: RewriteScratch::default(),
        }
    }
}

/// Allocations owned by `StitchAnalysisRunner` itself (independent of the analysis).
/// Two parallel dense vectors indexed by `usize::from(Id)`: `original` holds the
/// un-rewritten weighted size per eclass (built once at construction), `overrides`
/// is the working size table that `solve` relaxes downward. Both are sized to
/// `max_id + 1`.
pub struct RunnerScratch {
    original: Vec<i64>,
    overrides: Vec<i64>,
    /// Per-eclass dirty flag indexed by `usize::from(Id)`. `solve` only re-evaluates
    /// dirty eclasses; visiting clears the flag, and an improvement re-dirties the
    /// eclass's parents so they reconsider next time around.
    dirty: Vec<bool>,
}

impl RunnerScratch {
    /// Captures `original` from the egraph; `overrides` and `dirty` are left empty
    /// and filled by `reset` at the start of each solve.
    fn new<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> Self {
        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut original = vec![0i64; max_id + 1];
        for class in egraph.classes() {
            original[usize::from(class.id)] = class.data.size as i64;
        }
        Self { original, overrides: Vec::new(), dirty: Vec::new() }
    }

    /// Resets `overrides` to a copy of `original` and marks every eclass clean.
    /// Callers (or the analysis) seed dirty bits via `set` / `mark_dirty` before
    /// `solve` runs; nothing else needs revisiting. `original` is preserved.
    fn reset(&mut self) {
        self.overrides.clear();
        self.overrides.extend_from_slice(&self.original);
        self.dirty.clear();
        self.dirty.resize(self.original.len(), false);
    }
}

/// Pluggable per-eclass relaxation rule. `best` is an associated function (no `&self`)
/// so the solver can pass `&StitchAnalysisRunner<Self>` without conflicting borrows;
/// analysis-owned data is reached via `sizes.analysis`.
pub trait StitchAnalysis<L: StitchLanguage>: Sized {
    /// Candidate size for `eclass` given currently known sizes.
    fn best(sizes: &StitchAnalysisRunner<L, Self>, eclass: Id) -> i64;
    /// Extra dirty-propagation edges beyond syntactic parents. When `id`
    /// improves, these eclasses are also marked dirty. Used when `best` reads
    /// non-syntactic dependencies (e.g. rewrite-arg eclasses).
    fn extra_parents(&self, _id: Id) -> &[Id] {
        &[]
    }
}

/// Dense per-eclass size table with a fallback to the unrewritten weighted size
/// (`egraph[id].data.size`). An entry is set only when the rewritten size beats the default.
pub struct StitchAnalysisRunner<'a, L: StitchLanguage, A: StitchAnalysis<L>> {
    egraph: &'a StitchEgraph<L>,
    cache: &'a CostCache,
    scratch: &'a mut RunnerScratch,
    pub analysis: A,
}

impl<'a, L: StitchLanguage, A: StitchAnalysis<L>> StitchAnalysisRunner<'a, L, A> {
    fn new(egraph: &'a StitchEgraph<L>, cache: &'a CostCache, scratch: &'a mut RunnerScratch, analysis: A) -> Self {
        scratch.reset();
        StitchAnalysisRunner { egraph, cache, scratch, analysis }
    }
    pub fn get(&self, id: Id) -> i64 {
        self.scratch.overrides[usize::from(id)]
    }
    /// Writes a new size for `id` and marks every parent dirty so they reconsider.
    /// `id` itself is left clean — re-evaluating won't beat the value we just wrote.
    fn set(&mut self, id: Id, v: i64) {
        self.scratch.overrides[usize::from(id)] = v;
        let parents = self.cache.parents_of.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
        for &p in parents {
            self.scratch.dirty[usize::from(p)] = true;
        }
        let extras = self.analysis.extra_parents(id);
        for &p in extras {
            self.scratch.dirty[usize::from(p)] = true;
        }
    }
    fn mark_dirty(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = true;
    }
    fn mark_clean(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = false;
    }
    fn is_dirty(&self, id: Id) -> bool {
        self.scratch.dirty[usize::from(id)]
    }
    fn any_dirty(&self) -> bool {
        self.scratch.dirty.iter().any(|&d| d)
    }
    pub fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    /// Minimum size over the enodes of `eclass`. Panics if the eclass has no enodes.
    pub fn min_enode_size(&self, eclass: Id) -> i64 {
        let weights = &self.egraph.analysis.weights;
        self.egraph[eclass].nodes.iter().map(|enode| enode.discriminant().intrinsic_size(weights) as i64 + self.sum(enode.children())).min().unwrap()
    }
    pub fn weights(&self) -> &Weights {
        &self.egraph.analysis.weights
    }
    /// Iterates eclasses reachable from the root in postorder (children first),
    /// re-evaluating only those marked dirty. Visiting clears the flag, and `set`
    /// re-marks parents on any improvement. Repeats until a full pass finds nothing
    /// better. Initial dirty bits are seeded by callers/analyses before `solve`.
    fn solve(&mut self) {
        while self.any_dirty() {
            for &id in &self.cache.visit_order {
                if !self.is_dirty(id) {
                    continue;
                }
                self.mark_clean(id);
                let new = A::best(self, id);
                if new < self.get(id) {
                    self.set(id, new);
                }
            }
        }
    }
}

/// Reusable index map: match-root eclass → index into `search_state.matches`.
/// We store an index (not a `&MatchAtEClass`) so the map is `'static`-friendly and can
/// be reused across calls bound to different `SearchState`s.
#[derive(Default)]
pub struct RewriteScratch {
    pub eclass_to_match_idx: FxHashMap<Id, usize>,
    /// Captured-arg eclass → match roots that read it. Feeds `extra_parents` so a
    /// shrunk arg re-dirties the match roots reading it via the (non-syntactic)
    /// rewrite path. Mirrors `LowerBoundAnalysis::arg_to_match_roots`.
    pub arg_to_match_roots: FxHashMap<Id, Vec<Id>>,
}

impl RewriteScratch {
    /// Refills the index maps from `search_state`. Clears first; retains capacity.
    /// Match-root and row ids are canonical by construction (see `MatchAtEClass`):
    /// they originate from `egraph.classes()` and propagate unchanged through
    /// `build_subset_matches`; the egraph isn't unioned during search.
    pub fn fill<F: LanguageFamily, O: StitchOp>(&mut self, search_state: &SearchState<F, O>) {
        self.eclass_to_match_idx.clear();
        self.arg_to_match_roots.clear();
        for (i, m) in search_state.matches.iter().enumerate() {
            self.eclass_to_match_idx.insert(m.root_eclass, i);
            for f in &m.factors {
                for row in &f.rows {
                    for &v in row {
                        self.arg_to_match_roots.entry(v).or_default().push(m.root_eclass);
                    }
                }
            }
        }
        for roots in self.arg_to_match_roots.values_mut() {
            roots.sort_unstable();
            roots.dedup();
        }
    }
}

/// How a candidate restricts the substs eligible to rewrite at each match.
///
/// Either way the rewrite cost is evaluated *factored* — `min over ∏factors of
/// Σslots = Σfactors of (min over rows of Σslots)` — so the product is never
/// materialised. Candidate compatibility is per-slot (`captures(R^k) ⊆ S_k`),
/// hence per-factor, so a lambda-capture candidate just filters each factor's
/// rows; the product structure (and the separable min) is preserved.
pub enum KeptArgs<'a> {
    /// Every subst is compatible — min over each match's own factors (the
    /// common, lambda-free path).
    AllFactored,
    /// A lambda-capture candidate: per match, the candidate-filtered factors,
    /// or `None` when some factor filtered empty (the match keeps no subst and
    /// doesn't rewrite).
    Filtered(&'a [Option<Vec<Factor>>]),
}

/// Default analysis: at each match root, rewriting via `inv_0(args...)` is allowed,
/// otherwise we fall back to the minimum enode size.
pub struct RewriteAnalysis<'a, F: LanguageFamily, O: StitchOp> {
    pub search_state: &'a SearchState<F, O>,
    pub eclass_to_match_idx: &'a FxHashMap<Id, usize>,
    /// See `RewriteScratch::arg_to_match_roots`. Drives `extra_parents`.
    pub arg_to_match_roots: &'a FxHashMap<Id, Vec<Id>>,
    pub ho_arity: &'a [u32],
    pub kept: KeptArgs<'a>,
}

impl<'a, F: LanguageFamily, O: StitchOp> StitchAnalysis<F::Apply<O>> for RewriteAnalysis<'a, F, O> {
    fn best(sizes: &StitchAnalysisRunner<F::Apply<O>, Self>, eclass: Id) -> i64 {
        // Try not rewriting self but YES allowing rewrites of descendants.
        let mut best = sizes.min_enode_size(eclass);
        // For every way we match at this eclass (if any), try all ways of rewriting it.
        if let Some(&i) = sizes.analysis.eclass_to_match_idx.get(&eclass) {
            let weights = sizes.weights();
            let ho_arity = sizes.analysis.ho_arity;
            let stub_size = F::stub_application_size(ho_arity.len(), weights) as i64;
            // Per-slot arg cost: the captured eclass size plus, for HO slots,
            // the cost of the `h` wrap-λs. Additively separable across slots,
            // which is exactly what lets the min factor over independent groups.
            let arg_cost = |k: usize, v: Id| -> i64 {
                let h = ho_arity[k];
                let wrap = if h > 0 { F::lams_cost(h, weights) as i64 } else { 0 };
                wrap + sizes.get(v)
            };
            // Separable min over a factor list: stub + Σfactors (min over rows of Σslots).
            let min_over = |factors: &[Factor]| -> i64 { stub_size + factored_min(factors, arg_cost) };
            let rewrite_size = match &sizes.analysis.kept {
                KeptArgs::AllFactored => Some(min_over(&sizes.analysis.search_state.matches[i].factors)),
                KeptArgs::Filtered(per) => per[i].as_deref().map(min_over),
            };
            if let Some(rs) = rewrite_size {
                best = best.min(rs);
            }
        }
        best
    }
    fn extra_parents(&self, id: Id) -> &[Id] {
        self.arg_to_match_roots.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Returns the total cost (compressed corpus size + abstraction body size),
/// optimised over every meaningful `variable_indices` candidate. See
/// `enumerate_candidates` for the candidate space. Each `?#k` with
/// `|S_k| > 0` has its body uses applied to the enclosing binders
/// (`(@ … (@ ?#k $0) … $h-1)`), which adds `h * (app_cost + sym_var_cost)`
/// per occurrence.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    compute_cost_and_select(egraph, root, cache, scratch, search_state, check_slow).cost
}

/// Optimise over every meaningful candidate. Returns the chosen candidate
/// (variable_indices + kept-subst mask) and its total cost, so callers
/// downstream (`apply_abstraction`, `build_rewritten_egraph`) can use the
/// same selection without redoing the optimisation.
pub fn compute_cost_and_select<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> CostSelection {
    assert!(
        !search_state.matches.is_empty(),
        "compute_cost_and_select: search_state.matches is empty; a pattern with no matches has no rewrite candidates to optimise over. Callers (best_first, smc) must filter empty-match states before scoring."
    );
    let candidates = enumerate_candidates::<F, O>(egraph, search_state);
    let weights = &egraph.analysis.weights;
    // Hoisted: fill once per cost call. `eclass_to_match_idx` depends only on
    // `search_state.matches`, which is the same across every candidate, so
    // repeating the fill per candidate was wasted work.
    scratch.rewrite.fill(search_state);
    let mut best: Option<CostSelection> = None;
    for candidate in candidates {
        let ho_arity: Vec<u32> = candidate.variable_indices.iter().map(|v| v.len() as u32).collect();
        let corpus = compute_size_for_candidate_prefilled(egraph, root, cache, scratch, search_state, check_slow, &candidate, &ho_arity);
        let body = compute_body_size_with_ho::<F, O>(&search_state.pattern, &ho_arity, weights);
        let cost = corpus + body;
        if best.as_ref().is_none_or(|b| cost < b.cost) {
            best = Some(CostSelection { cost, candidate });
        }
    }
    best.expect("enumerate_candidates returns at least one candidate")
}

/// Size of the abstraction's pattern body — the pattern AST counted under
/// the active weights. Each `?#k` is a 0-arity meta-var leaf; HO body apps
/// are not included here (use `compute_body_size_with_ho` for the
/// inclusive form).
pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, weights: &Weights) -> usize {
    let rec_expr: RecExpr<F::Apply<crate::lang::OpWithVar<O>>> = pattern.pattern.clone().into();
    compute_recexpr_size::<F::Apply<crate::lang::OpWithVar<O>>>(&rec_expr, (rec_expr.len() - 1).into(), weights)
}

/// Total body size including HO-app wrapping: `compute_pattern_size` plus,
/// for each *syntactic* occurrence of `?#k` with `ho_arity[k] > 0`, the cost
/// of the `(@ … (@ ?#k $0) … $(h-1))` wrapper — one `app_cost` + one
/// `sym_var_cost` per binder, per occurrence.
///
/// Uses `pattern.var_occurrences[k]`, which is maintained incrementally by
/// `expand`/`reuse` and counts each parent reference (matching
/// `compute_pattern_size`'s syntactic-walk semantics). Using `vars[k].len()`
/// here would charge once per unique RecExpr id and silently reward DAG-shared
/// constructions, making the same final pattern cost less depending on the
/// action sequence that built it.
pub fn compute_body_size_with_ho<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, ho_arity: &[u32], weights: &Weights) -> usize {
    let base = compute_pattern_size::<F, O>(pattern, weights);
    if ho_arity.iter().all(|&h| h == 0) {
        return base;
    }
    let per_app = weights.app_cost + weights.sym_var_cost;
    let ho_extra: u32 = (0..pattern.vars.len()).map(|k| pattern.var_occurrences[k] as u32 * ho_arity[k] * per_app).sum();
    base + ho_extra as usize
}

/// Tree-expanded size: DAG-shared subtrees are counted once per parent
/// reference (e.g. `(+ a a a)` counts `a` three times). Matches the
/// `var_occurrences` convention used throughout cost accounting.
pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Drives a `StitchAnalysisRunner` to fixed point. Match-root eclasses seed the
/// dirty set; as their sizes improve, parents are re-dirtied and reconsidered.
/// Compute the corpus size for a specific candidate. Only substs compatible
/// with the candidate's `variable_indices` (`S`) are considered for rewriting,
/// recovered per-factor via [`filter_factors_by_candidate`]; the body size is
/// implied by `S` and is *not* added here — callers add it via
/// `compute_body_size_with_ho`.
pub fn compute_size_for_candidate<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool, candidate: &CostCandidate) -> usize {
    scratch.rewrite.fill(search_state);
    let ho_arity: Vec<u32> = candidate.variable_indices.iter().map(|v| v.len() as u32).collect();
    compute_size_for_candidate_prefilled(egraph, root, cache, scratch, search_state, check_slow, candidate, &ho_arity)
}

/// Filters each factor of `m` to the rows compatible with candidate fv-sets
/// `S` (`variable_indices`): a row survives iff at every slot `k` it covers, the
/// captured pattern-internal fv of its arg (`fv ∩ [0, d_k)`) is a subset of
/// `S_k`. Returns `None` when any factor empties — the match keeps no subst.
/// `S_k` is sorted ascending (`enumerate_candidates`), enabling the binary
/// search.
pub(crate) fn filter_factors_by_candidate<L: StitchLanguage>(egraph: &StitchEgraph<L>, m: &MatchAtEClass, variable_indices: &[Vec<i32>], var_depth: &[u32]) -> Option<Vec<Factor>> {
    let mut out = Vec::with_capacity(m.factors.len());
    for f in &m.factors {
        let rows: Vec<Vec<Id>> = f
            .rows
            .iter()
            .filter(|row| {
                f.slots.iter().zip(row.iter()).all(|(&k, &v)| {
                    let d_k = var_depth[k];
                    d_k == 0 || egraph[v].data.fv.iter().all(|&i| i < 0 || (i as u32) >= d_k || variable_indices[k].binary_search(&i).is_ok())
                })
            })
            .cloned()
            .collect();
        out.push(Factor::new(f.slots.clone(), rows)?);
    }
    Some(out)
}

/// Same as [`compute_size_for_candidate`] but assumes `scratch.rewrite` is
/// already filled for the current `search_state` and the caller has already
/// computed `ho_arity`. `compute_cost_and_select` hoists both to amortise
/// across candidates.
#[allow(clippy::too_many_arguments)]
fn compute_size_for_candidate_prefilled<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool, candidate: &CostCandidate, ho_arity: &[u32]) -> usize {
    // On lambda-bearing domains the candidate keeps a subst iff its per-slot
    // captures fit `S` (`variable_indices`). That test is per-slot, hence
    // per-factor, so we just filter each factor's rows — the product stays
    // factored and the separable min applies unchanged. Lambda-free domains
    // (`var_depth` all 0) can capture nothing, so every subst is kept and no
    // filtering is needed.
    let var_depth = &search_state.pattern.var_depth;
    let lambda_free = var_depth.iter().all(|&d| d == 0);
    let filtered: Option<Vec<Option<Vec<Factor>>>> = (!lambda_free).then(|| search_state.matches.iter().map(|m| filter_factors_by_candidate(egraph, m, &candidate.variable_indices, var_depth)).collect());
    let analysis = RewriteAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
        arg_to_match_roots: &scratch.rewrite.arg_to_match_roots,
        ho_arity,
        kept: match &filtered {
            Some(f) => KeptArgs::Filtered(f),
            None => KeptArgs::AllFactored,
        },
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for (i, m) in search_state.matches.iter().enumerate() {
        // Skip seeding matches that keep no subst (a factor filtered empty), so
        // the dirty set agrees with what `best` will actually rewrite.
        if let Some(f) = &filtered
            && f[i].is_none()
        {
            continue;
        }
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    let final_size = sizes.get(root);
    if check_slow {
        let rewritten = build_rewritten_egraph::<F, O>(egraph.clone(), search_state, candidate, "inv_0");
        let slow_size = rewritten[root].data.size as i64;
        F::check_fast_vs_slow(final_size, slow_size);
        // Semantic guard: rewriting must preserve the free-variable set at the
        // root. A mismatch means `wrap_subst_args` is shifting captured args
        // incorrectly and the abstraction's call site no longer agrees with the
        // original program on outer-scope references.
        assert_eq!(egraph[root].data.fv, rewritten[root].data.fv, "free-variable set diverges after rewrite: original {:?} != rewritten {:?}", egraph[root].data.fv, rewritten[root].data.fv,);
    }
    final_size as usize
}

/// Optimistic analysis producing a lower bound on achievable size. At a match
/// root, the rewrite collapses to a single stub node plus the captured
/// arguments at frozen-var slots (those that are holes, `var_frozen[k]`) — those holes are
/// committed to staying, so their args appear verbatim at every call site and
/// must be paid for. Non-frozen vars can still be expanded into the body, so
/// they contribute nothing here. Min taken across substs.
pub struct LowerBoundAnalysis<'a, F: LanguageFamily, O: StitchOp> {
    pub search_state: &'a SearchState<F, O>,
    pub eclass_to_match_idx: &'a FxHashMap<Id, usize>,
    /// Inverse index: frozen-arg eclass → match-root eclasses whose rewrite
    /// size depends on it. The match-root's rewrite path reads arg sizes
    /// directly, bypassing the syntactic enode chain, so syntactic dirty
    /// propagation alone may not reach it when an arg improves.
    pub arg_to_match_roots: FxHashMap<Id, Vec<Id>>,
}

impl<'a, F: LanguageFamily, O: StitchOp> StitchAnalysis<F::Apply<O>> for LowerBoundAnalysis<'a, F, O> {
    fn best(sizes: &StitchAnalysisRunner<F::Apply<O>, Self>, eclass: Id) -> i64 {
        let mut best = sizes.min_enode_size(eclass);
        if let Some(&i) = sizes.analysis.eclass_to_match_idx.get(&eclass) {
            let var_frozen = &sizes.analysis.search_state.pattern.var_frozen;
            let weights = sizes.weights();
            let stub_size = F::stub_application_size(sizes.analysis.search_state.frozen_count(), weights) as i64;
            // Only frozen slots contribute; their arg sizes are additively
            // separable, so the min over the product factors into a sum of
            // per-factor minima (factors with no frozen slot add 0).
            let factors = &sizes.analysis.search_state.matches[i].factors;
            let rewrite_size = stub_size + factored_min(factors, |k, v| if var_frozen[k] { sizes.get(sizes.egraph.find(v)) } else { 0 });
            best = best.min(rewrite_size);
        }
        best
    }
    fn extra_parents(&self, id: Id) -> &[Id] {
        self.arg_to_match_roots.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Optimistic lower bound on the objective `corpus + body`. Corpus side: each
/// match contributes a 1-node stub plus the minimum total size of its frozen-var
/// arguments (those can no longer shrink via further expansion). Body side: the
/// current pattern size — monotone non-decreasing under expand/reuse and a lower
/// bound on the final `compute_body_size_with_ho` (the HO term is non-negative),
/// so it both tightens the bound and, being unbounded in pattern depth,
/// guarantees the search tree is finite. Reuses allocations in `scratch`.
pub fn compute_lower_bound<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>) -> usize {
    scratch.rewrite.fill(search_state);
    let var_frozen = &search_state.pattern.var_frozen;
    let mut arg_to_match_roots: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for m in &search_state.matches {
        let root = egraph.find(m.root_eclass);
        for f in &m.factors {
            for (p, &k) in f.slots.iter().enumerate() {
                if var_frozen[k] {
                    for row in &f.rows {
                        arg_to_match_roots.entry(egraph.find(row[p])).or_default().push(root);
                    }
                }
            }
        }
    }
    let analysis = LowerBoundAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
        arg_to_match_roots,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    sizes.get(root) as usize + compute_pattern_size::<F, O>(&search_state.pattern, &egraph.analysis.weights)
}

/// Outcome of the Version-B budget analysis at one search node: the match set
/// with budget-dead substitution *rows* removed (factoring preserved), plus
/// before/after subst counts.
pub struct BudgetPrune {
    /// Input matches with dead rows filtered out; matches whose factored product
    /// empties are dropped entirely. Subst-level, not just root-level.
    pub matches: Vec<MatchAtEClass>,
    /// The whole-node optimistic lower bound (identical to
    /// [`compute_lower_bound`]) computed off the same fixpoint, so callers can
    /// do the node-level prune without a second pass.
    pub lb_root: usize,
    /// Total substitutions across the input matches (`Σ num_substs`).
    pub substs_before: usize,
    /// Substitutions surviving the combined budget+local prune.
    pub substs_after: usize,
    /// Matches dropped entirely (every subst pruned).
    pub matches_emptied: usize,
}

/// Version B: top-down budget pruning of individual match roots.
///
/// Runs the same optimistic bottom-up `LowerBoundAnalysis` fixpoint as
/// [`compute_lower_bound`] to obtain a per-eclass lower bound `lb(e)`, then
/// sweeps root→leaf over the reverse postorder computing a budget `B(e)` — the
/// largest cost `e`'s subtree can realize while `e` is still on a plausible min
/// term. The bound discipline is asymmetric, which is exactly what makes this
/// sound on top of a *bound* rather than an exact cost:
///   - **competitors** (the other enodes at an ancestor) are charged their
///     ORIGINAL size (`data.size`, the largest they can be) — an alternative
///     branch may not wield its own optimistic compression to starve the
///     branch under consideration;
///   - **siblings** (the other children of the enode on the branch) are charged
///     `lb` (maximally compressed).
///
/// Both choices are pointwise-favorable to the branch, so the favorable
/// assignment dominates every real cost assignment: a match root whose stub
/// exceeds `B(root)` loses under every realizable extraction and is safe to drop
/// (and stays prunable in all descendants — `stub` is monotone ↑, `B` monotone
/// ↓ under specialization).
pub fn compute_budget_prunable<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>) -> BudgetPrune {
    // --- bottom-up: optimistic per-eclass lower bound (mirrors compute_lower_bound) ---
    scratch.rewrite.fill(search_state);
    let var_frozen = &search_state.pattern.var_frozen;
    let mut arg_to_match_roots: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for m in &search_state.matches {
        let r = egraph.find(m.root_eclass);
        for f in &m.factors {
            for (p, &k) in f.slots.iter().enumerate() {
                if var_frozen[k] {
                    for row in &f.rows {
                        arg_to_match_roots.entry(egraph.find(row[p])).or_default().push(r);
                    }
                }
            }
        }
    }
    let analysis = LowerBoundAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
        arg_to_match_roots,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    // Whole-node bound, identical to `compute_lower_bound`, off this fixpoint.
    let lb_root = sizes.get(root) as usize + compute_pattern_size::<F, O>(&search_state.pattern, &egraph.analysis.weights);

    // --- shared quantities ---
    let weights = &egraph.analysis.weights;
    let cap = cache.visit_order.iter().map(|&e| usize::from(e)).max().map_or(0, |m| m + 1);
    let orig = |e: Id| egraph[egraph.find(e)].data.size as i64;
    let lb = |e: Id| sizes.get(egraph.find(e));
    let stub_size = F::stub_application_size(search_state.frozen_count(), weights) as i64;

    // --- pass 1: per-match frozen-arg costs + bottom-up local cull ---
    // `frowcosts[f][r]` = Σ frozen-slot lb of row r (non-frozen slots cost 0);
    // `m_total` = Σ per-factor minima = the cheapest subst's stub-extra. A match
    // is locally dead when even that cheapest stub can't beat the eclass's own
    // ORIGINAL size — sound (smallest stub vs largest local alternative) and
    // observable here on the way up, no budget needed. Dead matches are settled
    // now and kept out of the top-down closure seed.
    struct MatchCost {
        frowcosts: Vec<Vec<i64>>,
        fmin: Vec<i64>,
        m_total: i64,
        locally_dead: bool,
    }
    let match_costs: Vec<MatchCost> = search_state
        .matches
        .iter()
        .map(|m| {
            let frowcosts: Vec<Vec<i64>> = m.factors.iter().map(|f| f.rows.iter().map(|row| f.slots.iter().enumerate().filter(|(_, k)| var_frozen[**k]).map(|(p, _)| lb(row[p])).sum()).collect()).collect();
            let fmin: Vec<i64> = frowcosts.iter().map(|rc| *rc.iter().min().expect("factor rows are non-empty")).collect();
            let m_total: i64 = fmin.iter().sum();
            let locally_dead = stub_size + m_total >= orig(m.root_eclass);
            MatchCost { frowcosts, fmin, m_total, locally_dead }
        })
        .collect();

    // --- top-down: budget B(e) over the reverse postorder (root before children) ---
    // i64::MIN marks an eclass not yet reached by any parent route; `max` over
    // routes keeps the most generous (largest) budget, since a match is kept if
    // any route keeps it.
    let mut budget = vec![i64::MIN; cap];
    let root = egraph.find(root);
    budget[usize::from(root)] = egraph[root].data.size as i64;
    // Restrict the sweep to the ancestor-closure of the *locally-alive* match
    // roots: only those budgets are ever read, and no non-ancestor is ever a
    // parent of an ancestor, so match-root budgets match a full sweep. Locally
    // dead roots are excluded — already pruned, no budget needed.
    let mut relevant = vec![false; cap];
    let mut stack: Vec<Id> = Vec::new();
    for (m, mc) in search_state.matches.iter().zip(&match_costs) {
        if mc.locally_dead {
            continue;
        }
        let r = egraph.find(m.root_eclass);
        let i = usize::from(r);
        if i < cap && !relevant[i] {
            relevant[i] = true;
            stack.push(r);
        }
    }
    while let Some(e) = stack.pop() {
        if let Some(parents) = cache.parents_of.get(&e) {
            for &p in parents {
                let pi = usize::from(p);
                if pi < cap && !relevant[pi] {
                    relevant[pi] = true;
                    stack.push(p);
                }
            }
        }
    }
    for &e in cache.visit_order.iter().rev() {
        if !relevant[usize::from(e)] {
            continue;
        }
        let be = budget[usize::from(e)];
        if be == i64::MIN {
            continue;
        }
        let enodes = &egraph[e].nodes;
        // Original full-subtree cost of each enode — the competitor charge.
        let orig_node: Vec<i64> = enodes.iter().map(|nd| nd.discriminant().intrinsic_size(weights) as i64 + nd.children().iter().map(|&c| orig(c)).sum::<i64>()).collect();
        // Two smallest original alternatives, so the cheapest *other* enode is
        // O(1) per enode below.
        let (mut min1, mut min2) = (i64::MAX, i64::MAX);
        for &v in &orig_node {
            if v < min1 {
                min2 = min1;
                min1 = v;
            } else if v < min2 {
                min2 = v;
            }
        }
        for (ei, nd) in enodes.iter().enumerate() {
            // Cheapest competing enode at ORIGINAL size; i64::MAX when this is
            // the only enode (no alternative constrains the branch).
            let competitor = if orig_node[ei] > min1 { min1 } else { min2 };
            // The enode's allowed cost: capped both by the room from above and
            // by needing to beat the cheapest original competitor.
            let cap_n = be.min(competitor);
            let intrinsic = nd.discriminant().intrinsic_size(weights) as i64;
            let children: Vec<Id> = nd.children().iter().map(|&c| egraph.find(c)).collect();
            let total_child_lb: i64 = children.iter().map(|&c| lb(c)).sum();
            for &c in &children {
                // Only relevant children's budgets are ever used; non-ancestors
                // are skipped (their siblings still count via `total_child_lb`).
                let slot = usize::from(c);
                if !relevant[slot] {
                    continue;
                }
                // Room for this child = enode budget − intrinsic − LB of its
                // siblings (a repeated child correctly leaves its other copies
                // as siblings).
                let cand = cap_n - intrinsic - (total_child_lb - lb(c));
                if cand > budget[slot] {
                    budget[slot] = cand;
                }
            }
        }
    }

    // --- pass 2: per-(match, factor, row) filter via the min-of-the-rest bound ---
    // Row `r` of factor `f` survives iff its smallest-possible stub (paired with
    // the cheapest row of every other factor) stays under
    //   T = min(B(c)+1, orig(c))
    // folding the path budget (stub > B) and the local rule (stub ≥ orig) into
    // one threshold. Locally-dead matches were already settled in pass 1.
    let mut out_matches: Vec<MatchAtEClass> = Vec::with_capacity(search_state.matches.len());
    let mut substs_before = 0usize;
    let mut substs_after = 0usize;
    let mut matches_emptied = 0usize;
    for (m, mc) in search_state.matches.iter().zip(&match_costs) {
        substs_before += m.num_substs();
        if mc.locally_dead {
            matches_emptied += 1;
            continue;
        }
        let r = egraph.find(m.root_eclass);
        let b = budget[usize::from(r)];
        // An unreached root keeps the path budget out of it (local cull already
        // applied), so use orig(c) alone.
        let t = if b == i64::MIN { orig(r) } else { (b + 1).min(orig(r)) };
        let mut kept: Vec<Factor> = Vec::with_capacity(m.factors.len());
        let mut prod = 1usize;
        let mut emptied = false;
        for (fi, f) in m.factors.iter().enumerate() {
            let keep_bound = t - stub_size - (mc.m_total - mc.fmin[fi]);
            let rows: Vec<Vec<Id>> = f.rows.iter().enumerate().filter(|&(ri, _)| mc.frowcosts[fi][ri] < keep_bound).map(|(_, row)| row.clone()).collect();
            if rows.is_empty() {
                emptied = true;
                break;
            }
            prod *= rows.len();
            kept.push(Factor { slots: f.slots.clone(), rows });
        }
        if emptied {
            matches_emptied += 1;
        } else {
            substs_after += prod;
            out_matches.push(MatchAtEClass { root_eclass: m.root_eclass, factors: kept });
        }
    }
    BudgetPrune {
        matches: out_matches,
        lb_root,
        substs_before,
        substs_after,
        matches_emptied,
    }
}

/// Consumes `egraph` and unions each match root with a `fn_name(args...)`
/// node, then rebuilds. Source of truth for the rewrite — used both by
/// `lib::apply_abstraction` (which extracts+reparses on top) and by
/// `compute_size_for_candidate`'s `check_slow` validation path.
///
/// Each captured eclass is fed through `wrap_subst_args` to re-index its
/// pattern-internal fv onto wrap-lam slots and shift above-pattern fv past
/// the wrap-lams, then wrapped under `variable_indices[k].len()` λs before
/// being passed in.
pub fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(mut egraph: StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>, candidate: &CostCandidate, fn_name: &str) -> StitchEgraph<F::Apply<O>> {
    let variable_indices = &candidate.variable_indices;
    let var_depth = &search_state.pattern.var_depth;
    // Defer unions until all shifts are done. A mid-loop `union` shrinks
    // `data.fv` on the unioned classes but leaves parent classes stale until
    // `rebuild`, and the next iteration's `shift_free_egraph` would then
    // read that stale fv and trip the intersection-fv assertion.
    let mut pending: Vec<(Id, Id)> = Vec::new();
    for m in &search_state.matches {
        // Restrict to the candidate-compatible substs (per-factor filter), then
        // materialise that filtered product — the rewrite genuinely needs each
        // joint assignment. This is the slow/validation + apply path, not hot.
        let Some(kept) = filter_factors_by_candidate(&egraph, m, variable_indices, var_depth) else {
            continue;
        };
        for vars in crate::factor::factors_product(&kept) {
            let wrapped = wrap_subst_args::<F, O>(&mut egraph, &vars, variable_indices, var_depth);
            let x = F::add_stub_application::<O>(fn_name, wrapped, &mut egraph);
            pending.push((x, m.root_eclass));
        }
    }
    for (x, root_eclass) in pending {
        egraph.union(x, root_eclass);
    }
    egraph.rebuild();
    egraph
}

/// Per-subst HO wrapping: for each captured arg `arg_id` at metavar slot `k`,
/// returns `λ^h. permuted_shift(arg_id, vis[k], d_k)`, where `h = vis[k].len()`
/// and `d_k = var_depth[k]`. Each pattern-internal `$i` with `i ∈ vis[k]` is
/// re-indexed to the wrap-lam slot that the body's η-app at `?#k` will rebind
/// it through, so β at the call site recovers the original `$i`. Above-pattern
/// free indices (`i ≥ d_k`) shift past the `h` wrap-lams. Used by both
/// `lib::apply_abstraction` and its `check_slow` validation path; `shift_memo`
/// is shared across calls so equivalent shifts are deduplicated.
pub(crate) fn wrap_subst_args<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, vars: &[Id], variable_indices: &[Vec<i32>], var_depth: &[u32]) -> Vec<Id> {
    vars.iter()
        .enumerate()
        .map(|(k, &arg_id)| {
            let vis = &variable_indices[k];
            let h = vis.len() as u32;
            let d_k = var_depth[k];
            let rank_map: FxHashMap<i32, u32> = vis.iter().enumerate().map(|(r, &i)| (i, r as u32)).collect();
            // Memo is per-slot: keying by (canonical, initial_depth) is only
            // valid for a single (d_k, h, rank_map) — sharing across slots
            // would conflate transformations.
            let mut shift_memo: FxHashMap<(Id, u32), Id> = FxHashMap::default();
            let shifted = shift_free_egraph::<F, O>(egraph, arg_id, d_k, &rank_map, h, 0, &mut shift_memo);
            if h == 0 { shifted } else { F::wrap_lams::<O>(shifted, h, egraph) }
        })
        .collect()
}

/// Computes the exact syntactic free-variable set at every position of `expr`,
/// indexed by `usize::from(Id)`. Shares its per-enode rule with
/// `StitchAnalysis::make` via `enode_fv`.
pub fn recexpr_fv<L: StitchLanguage>(expr: &RecExpr<L>) -> Vec<FxHashSet<i32>> {
    let nodes: &[L] = expr.as_ref();
    let mut fv: Vec<FxHashSet<i32>> = vec![FxHashSet::default(); nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        fv[i] = enode_fv(node, |c| &fv[usize::from(c)]);
    }
    fv
}

/// Asserts that the extracted term's actual syntactic fv matches the egraph
/// analysis's recorded fv. Under intersection-fv semantics + WeightedSize
/// extraction, the minimal-size representative is also the fv-minimal one,
/// so its fv should equal the intersection across reps — i.e. `expected`.
/// A mismatch in either direction means the assumption "min-size ⇒ min-fv"
/// failed for this extraction; downstream soundness checks that read
/// `data.fv` lose their guarantee.
pub fn check_fvs_are_as_expected<L: StitchLanguage>(expr: &RecExpr<L>, expected: &FxHashSet<i32>) {
    let fv = recexpr_fv(expr);
    let actual = fv.last().expect("non-empty RecExpr");
    assert_eq!(actual, expected, "extracted RecExpr fv {:?} differs from egraph analysis fv {:?}; intersection-fv assumption (min-size rep is fv-minimal) violated", actual, expected,);
}

/// Asserts the invariant `fv(c) = fv(MinTerm(c))` for every eclass: the analysis
/// fv `data.fv` must equal the syntactic fv of the class's size-minimal enode
/// (computed from its children's class fv). Downstream capture decisions read
/// `data.fv` and rely on it matching the extracted min term. Panics on the first
/// offending class.
pub fn assert_fv_matches_min_term<L: StitchLanguage>(egraph: &StitchEgraph<L>) {
    let weights = egraph.analysis.weights;
    for c in egraph.classes() {
        let rep = c.nodes.iter().min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&ch| egraph[ch].data.size as u64).sum::<u64>()).expect("non-empty eclass");
        let rep_fv = enode_fv(rep, |ch| &egraph[ch].data.fv);
        assert_eq!(
            &rep_fv, &c.data.fv,
            "eclass {:?} data.fv {:?} differs from its min-term node fv {:?}; the fv(c)=fv(MinTerm(c)) invariant (min-size rep is fv-minimal) is violated",
            c.id, c.data.fv, rep_fv
        );
    }
}
