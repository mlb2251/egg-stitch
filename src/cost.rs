use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights, enode_fv};
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

/// Per-metavar higher-order arity. `ho_arity[k]` is the number of wrap-lams
/// each captured arg gets at slot `k` — equivalently, the number of distinct
/// pattern-internal DB indices referenced across all matches at this slot.
/// Zero means plain capture (no body wrapping needed).
pub fn compute_ho_arity<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<u32> {
    compute_variable_indices::<F, O>(egraph, search_state).into_iter().map(|v| v.len() as u32).collect()
}

/// Per-metavar sorted-ascending list of distinct pattern-internal DB indices
/// referenced by any match's captured arg. `variable_indices[k][j]` is a free
/// DB index `i` (0 ≤ i < d_k) appearing in `fv(arg_{m,k})` for some match `m`.
/// Symmetric to `compute_ho_arity` but returns the actual set of binder indices
/// referenced, not just their count.
pub fn compute_variable_indices<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<Vec<i32>> {
    let arity = search_state.pattern.var_depth.len();
    let var_depth = &search_state.pattern.var_depth;
    // No slot can capture pattern-internal binders → result is all-empty.
    // Skip the per-slot hashset allocations entirely; this is the common case
    // for domains without lambda/DB-var ops (e.g. dials).
    if var_depth.iter().all(|&d| d == 0) {
        return vec![Vec::new(); arity];
    }
    let mut sets: Vec<FxHashSet<i32>> = vec![FxHashSet::default(); arity];
    let mut seen_per_slot: Vec<FxHashSet<Id>> = vec![FxHashSet::default(); arity];
    for m in &search_state.matches {
        for subst in &m.substs {
            for (k, &arg_id) in subst.vars.iter().enumerate() {
                let d_k = var_depth[k];
                if d_k == 0 {
                    continue;
                }
                if !seen_per_slot[k].insert(arg_id) {
                    continue;
                }
                for &i in egraph[arg_id].data.fv.iter() {
                    if i >= 0 && (i as u32) < d_k {
                        sets[k].insert(i);
                    }
                }
            }
        }
    }
    sets.into_iter()
        .map(|s| {
            let mut v: Vec<i32> = s.into_iter().collect();
            v.sort();
            v
        })
        .collect()
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
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    /// Currently unused by `solve`, but kept for callers/inspection.
    #[allow(dead_code)]
    postorder: Vec<Option<u32>>,
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
                    parents_of.entry(child).or_default().push(class.id);
                }
            }
        }

        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut postorder = vec![None; max_id + 1];
        let mut visit_order: Vec<Id> = Vec::new();
        let mut order: u32 = 0;
        let mut stack: Vec<Result<Id, Id>> = vec![Err(root)]; // Err=enter, Ok=exit
        let mut on_stack = FxHashSet::<Id>::default();
        while let Some(state) = stack.pop() {
            match state {
                Err(id) => {
                    if postorder[usize::from(id)].is_some() || !on_stack.insert(id) {
                        continue;
                    }
                    stack.push(Ok(id));
                    for enode in &egraph[id].nodes {
                        for &child in enode.children() {
                            stack.push(Err(child));
                        }
                    }
                }
                Ok(id) => {
                    on_stack.remove(&id);
                    postorder[usize::from(id)] = Some(order);
                    visit_order.push(id);
                    order += 1;
                }
            }
        }

        Self { visit_order, postorder, parents_of }
    }
}

/// Reusable allocations for repeated cost computations. Build once with `new(egraph)`
/// and pass `&mut` to `compute_cost` / `compute_size` to avoid reallocating across calls.
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
            self.mark_dirty(p);
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
/// We store an index (not a `&Vec<Subst>`) so the map is `'static`-friendly and can
/// be reused across calls bound to different `SearchState`s.
#[derive(Default)]
pub struct RewriteScratch {
    pub eclass_to_match_idx: FxHashMap<Id, usize>,
}

impl RewriteScratch {
    /// Refills the index map from `search_state`. Clears first; retains capacity.
    pub fn fill<F: LanguageFamily, O: StitchOp>(&mut self, search_state: &SearchState<F, O>) {
        self.eclass_to_match_idx.clear();
        for (i, m) in search_state.matches.iter().enumerate() {
            self.eclass_to_match_idx.insert(m.root_eclass, i);
        }
    }
}

/// Default analysis: at each match root, rewriting via `inv_0(args...)` is allowed,
/// otherwise we fall back to the minimum enode size.
pub struct RewriteAnalysis<'a, F: LanguageFamily, O: StitchOp> {
    pub search_state: &'a SearchState<F, O>,
    pub eclass_to_match_idx: &'a FxHashMap<Id, usize>,
    pub ho_arity: &'a [u32],
}

impl<'a, F: LanguageFamily, O: StitchOp> StitchAnalysis<F::Apply<O>> for RewriteAnalysis<'a, F, O> {
    fn best(sizes: &StitchAnalysisRunner<F::Apply<O>, Self>, eclass: Id) -> i64 {
        // Try not rewriting self but YES allowing rewrites of descendants.
        let mut best = sizes.min_enode_size(eclass);
        // For every way we match at this eclass (if any), try all ways of rewriting it.
        if let Some(&i) = sizes.analysis.eclass_to_match_idx.get(&eclass) {
            let substs = &sizes.analysis.search_state.matches[i].substs;
            let weights = sizes.weights();
            let ho_arity = sizes.analysis.ho_arity;
            if let Some(rewrite_size) = substs
                .iter()
                .map(|subst| {
                    let stub_size = F::stub_application_size::<O>("inv_0", subst.vars.len(), weights) as i64;
                    let args_size: i64 = subst
                        .vars
                        .iter()
                        .enumerate()
                        .map(|(k, &v)| {
                            let h = ho_arity[k];
                            let wrap = if h > 0 { F::lams_cost(h, weights) as i64 } else { 0 };
                            wrap + sizes.get(v)
                        })
                        .sum();
                    stub_size + args_size
                })
                .min()
            {
                best = best.min(rewrite_size);
            }
        }
        best
    }
}

/// Returns the total cost: compressed corpus size plus the abstraction's own
/// pattern body size. Each `?#k` with `ho_arity[k] > 0` has its body uses
/// applied to the enclosing binders (`(@ … (@ ?#k $0) … $h-1)`), which adds
/// `h * (app_cost + sym_var_cost)` per occurrence.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let ho_arity = compute_ho_arity::<F, O>(egraph, search_state);
    let cost = compute_size(egraph, root, cache, scratch, search_state, check_slow, &ho_arity);
    let body_size = compute_body_size_with_ho::<F, O>(&search_state.pattern, &ho_arity, &egraph.analysis.weights);
    cost + body_size
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
    let pattern_size = compute_pattern_size::<F, O>(pattern, weights);
    if ho_arity.iter().all(|&h| h == 0) {
        return pattern_size;
    }
    let per_app = weights.app_cost + weights.sym_var_cost;
    let ho_extra: u32 = (0..pattern.vars.len()).map(|k| pattern.var_occurrences[k] as u32 * ho_arity[k] * per_app).sum();
    pattern_size + ho_extra as usize
}

pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Drives a `StitchAnalysisRunner` to fixed point. Match-root eclasses seed the
/// dirty set; as their sizes improve, parents are re-dirtied and reconsidered.
pub fn compute_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool, ho_arity: &[u32]) -> usize {
    scratch.rewrite.fill(search_state);
    let analysis = RewriteAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
        ho_arity,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    let final_size = sizes.get(root);
    if check_slow {
        let data = crate::shared::SharedData::new(egraph.clone(), root);
        let (rewritten, _programs) = crate::apply_abstraction::<F, O>(data, search_state, "inv_0", None);
        let slow_size = rewritten.egraph[rewritten.root].data.size as i64;
        F::check_fast_vs_slow(final_size, slow_size);
        // Semantic guard: rewriting must preserve the free-variable set at the
        // root. A mismatch means `wrap_subst_args` is shifting captured args
        // incorrectly and the abstraction's call site no longer agrees with the
        // original program on outer-scope references.
        assert_eq!(
            egraph[root].data.fv, rewritten.egraph[rewritten.root].data.fv,
            "free-variable set diverges after rewrite: original {:?} != rewritten {:?}",
            egraph[root].data.fv, rewritten.egraph[rewritten.root].data.fv,
        );
    }
    final_size as usize
}

/// Optimistic analysis producing a lower bound on achievable size. At a match
/// root, the rewrite collapses to a single stub node plus the captured
/// arguments at frozen-var slots (`?#0..?#(frozen_count-1)`) — those holes are
/// committed to staying, so their args appear verbatim at every call site and
/// must be paid for. Non-frozen vars can still be expanded into the body, so
/// they contribute nothing here. Min taken across substs.
pub struct LowerBoundAnalysis<'a, F: LanguageFamily, O: StitchOp> {
    pub search_state: &'a SearchState<F, O>,
    pub eclass_to_match_idx: &'a FxHashMap<Id, usize>,
}

impl<'a, F: LanguageFamily, O: StitchOp> StitchAnalysis<F::Apply<O>> for LowerBoundAnalysis<'a, F, O> {
    fn best(sizes: &StitchAnalysisRunner<F::Apply<O>, Self>, eclass: Id) -> i64 {
        let mut best = sizes.min_enode_size(eclass);
        if let Some(&i) = sizes.analysis.eclass_to_match_idx.get(&eclass) {
            let frozen = sizes.analysis.search_state.frozen_count.unwrap_or(0);
            let substs = &sizes.analysis.search_state.matches[i].substs;
            if let Some(rewrite_size) = substs.iter().map(|subst| 1 + subst.vars.iter().take(frozen).map(|&v| sizes.get(v)).sum::<i64>()).min() {
                best = best.min(rewrite_size);
            }
        }
        best
    }
}

/// Computes an optimistic lower bound on corpus size. Each match contributes a
/// 1-node stub plus the minimum total size of its frozen-var arguments (those
/// can no longer shrink via further expansion). Reuses allocations in `scratch`.
pub fn compute_lower_bound<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>) -> usize {
    scratch.rewrite.fill(search_state);
    let analysis = LowerBoundAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    sizes.get(root) as usize
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
