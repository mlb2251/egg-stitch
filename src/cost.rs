use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights, enode_fv};
use crate::matching::Subst;
use crate::pattern::{Pattern, PatternRecExpr};
use crate::rewrite::build_rewritten_egraph;
use crate::search::SearchState;
use egg::{Analysis, CostFunction, EClass, EGraph, Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Eclasses reachable from `root`, in postorder (children before parents).
    /// `solve` iterates this so child sizes settle before their parents reconsider.
    visit_order: Vec<Id>,
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    /// Currently unused by `solve`, but kept for callers/inspection.
    postorder: Vec<Option<u32>>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    /// Currently unused by `solve`, but kept for callers/inspection.
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
/// and pass `&mut` to `compute_cost`, `compute_size`, or `compute_lower_bound` to
/// avoid reallocating across calls.
pub struct CostScratch {
    pub runner: RunnerScratch,
    pub rewrite: RewriteScratch,
}

impl CostScratch {
    /// Builds the scratch space for a given egraph. The egraph's per-eclass AstSize
    /// is captured into `runner.original` here and reused across all subsequent calls.
    pub fn new<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> Self {
        Self {
            runner: RunnerScratch::new(egraph),
            rewrite: RewriteScratch::default(),
        }
    }
}

/// Allocations owned by `StitchAnalysisRunner` itself (independent of the analysis).
/// Two parallel dense vectors indexed by `usize::from(Id)`: `original` holds the
/// un-rewritten AstSize per eclass (built once at construction), `overrides` is the
/// working size table that `solve` relaxes downward. Both are sized to `max_id + 1`.
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

/// Dense per-eclass size table with a fallback to the unrewritten AstSize
/// (`egraph[id].data.size`). An entry is set only when the rewritten size beats the default.
pub struct StitchAnalysisRunner<'a, L: StitchLanguage, A: StitchAnalysis<L>> {
    egraph: &'a StitchEgraph<L>,
    cache: &'a CostCache,
    scratch: &'a mut RunnerScratch,
    pub analysis: A,
}
impl<'a, L: StitchLanguage, A: StitchAnalysis<L>> StitchAnalysisRunner<'a, L, A> {
    /// Allocates the override table sized to the egraph's eclasses.
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
    /// Marks `id` dirty so `solve` will (re)evaluate it.
    fn mark_dirty(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = true;
    }
    /// Marks `id` clean (will be skipped by `solve` until something re-dirties it).
    fn mark_clean(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = false;
    }
    fn is_dirty(&self, id: Id) -> bool {
        self.scratch.dirty[usize::from(id)]
    }
    fn any_dirty(&self) -> bool {
        self.scratch.dirty.iter().any(|&d| d)
    }
    /// Sum of `get` over a list of eclass ids.
    pub fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    pub fn original_size(&self, id: Id) -> i64 {
        self.scratch.original[usize::from(id)]
    }
    /// Minimum size over the enodes of `eclass`. Panics if the eclass has no enodes.
    pub fn min_enode_size(&self, eclass: Id) -> i64 {
        let weights = &self.egraph.analysis.weights;
        self.egraph[eclass].nodes.iter().map(|enode| enode.discriminant().intrinsic_size(weights) as i64 + self.sum(enode.children())).min().unwrap()
    }
    /// Cost weights carried on the underlying egraph's analysis.
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

/// Like `egg::Extractor`, but only computes the minimum cost per eclass — it
/// doesn't track the winning enode or reconstruct the `RecExpr`. Uses the same
/// fixpoint relaxation as egg's extractor: iterate until no eclass's best cost
/// improves, relying on `CostFunction`'s monotonicity for termination.
pub struct CostOnlyExtractor<'a, CF: CostFunction<L>, L: Language, N: Analysis<L>> {
    cost_function: CF,
    costs: FxHashMap<Id, CF::Cost>,
    egraph: &'a EGraph<L, N>,
}

impl<'a, CF, L, N> CostOnlyExtractor<'a, CF, L, N>
where
    CF: CostFunction<L>,
    L: Language,
    N: Analysis<L>,
{
    /// Builds the extractor and runs the cost-only fixpoint to completion.
    pub fn new(egraph: &'a EGraph<L, N>, cost_function: CF) -> Self {
        let mut this = Self { cost_function, costs: FxHashMap::default(), egraph };
        this.saturate();
        this
    }

    /// Returns the minimum cost for `eclass`, or `None` if no finite-cost term exists.
    pub fn cost(&self, eclass: Id) -> Option<CF::Cost> {
        self.costs.get(&self.egraph.find(eclass)).cloned()
    }

    /// Iteratively relaxes per-eclass costs until a full pass produces no improvement.
    fn saturate(&mut self) {
        let mut changed = true;
        while changed {
            changed = false;
            for class in self.egraph.classes() {
                if let Some(new) = self.best_cost(class) {
                    let improved = match self.costs.get(&class.id) {
                        None => true,
                        Some(old) => new.partial_cmp(old) == Some(std::cmp::Ordering::Less),
                    };
                    if improved {
                        self.costs.insert(class.id, new);
                        changed = true;
                    }
                }
            }
        }
    }

    /// Minimum cost over enodes in this eclass whose children all have known costs.
    fn best_cost(&mut self, class: &EClass<L, N::Data>) -> Option<CF::Cost> {
        class.iter().filter_map(|n| self.node_cost(n)).min_by(|a, b| a.partial_cmp(b).expect("CostFunction returned incomparable costs"))
    }

    /// Cost of a single enode if every child eclass already has a cost; else `None`.
    fn node_cost(&mut self, node: &L) -> Option<CF::Cost> {
        let eg = self.egraph;
        if node.all(|id| self.costs.contains_key(&eg.find(id))) {
            let costs = &self.costs;
            Some(self.cost_function.cost(node, |id| costs[&eg.find(id)].clone()))
        } else {
            None
        }
    }
}

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, scratch, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern, &egraph.analysis.weights);
    cost + pattern_size
}

/// Returns the AST size of the pattern, respecting each operator's `intrinsic_size()`.
/// Var nodes contribute via `OpWithVar::Var`'s `intrinsic_size = 1`, so the recursion
/// is uniform across var and non-var slots.
pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, weights: &Weights) -> usize {
    let rec_expr: RecExpr<F::Apply<OpWithVar<O>>> = PatternRecExpr::<F, O>::clone(&pattern.pattern).into();
    compute_recexpr_size::<F::Apply<OpWithVar<O>>>(&rec_expr, (rec_expr.len() - 1).into(), weights)
}

/// Recursive AST size of a `RecExpr<L>`, respecting `intrinsic_size()`.
pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a postorder min-heap so children pop before parents. Initial entries are the
/// match-root eclasses; when an eclass's size strictly improves we write it into
/// `sizes` and push its parents so they can reconsider with the new child value.
pub(crate) fn compute_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    scratch.rewrite.fill(search_state);
    let analysis = RewriteAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
    sizes.solve();
    let final_size = sizes.get(root);
    if check_slow {
        let rewritten = build_rewritten_egraph(egraph, search_state);
        let slow_size = rewritten[root].data.size as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
        let cost_only = CostOnlyExtractor::new(&rewritten, egg::AstSize);
        let cost_only_size = cost_only.cost(root).expect("root has no cost") as i64;
        assert_eq!(final_size, cost_only_size, "Fast rewrite size {} != CostOnlyExtractor size {}", final_size, cost_only_size);
    }
    final_size as usize
}

/// Optimistic analysis producing a lower bound on achievable size. Match-root
/// eclasses are seeded with size 1 directly into the runner's override table
/// before `solve` runs, so `best` only needs to return the minimum enode size —
/// the seeded `1`s persist because nothing improves on them.
pub struct LowerBoundAnalysis;
impl<L: StitchLanguage> StitchAnalysis<L> for LowerBoundAnalysis {
    fn best(sizes: &StitchAnalysisRunner<L, Self>, eclass: Id) -> i64 {
        sizes.min_enode_size(eclass)
    }
}

/// Computes an optimistic lower bound on corpus size by assuming every match collapses
/// to a single node. Reuses allocations in `scratch` across calls.
pub fn compute_lower_bound<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>) -> usize {
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, LowerBoundAnalysis);
    for m in &search_state.matches {
        sizes.set(m.root_eclass, 1);
    }
    sizes.solve();
    sizes.get(root) as usize
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
}
impl<'a, F: LanguageFamily, O: StitchOp> StitchAnalysis<F::Apply<O>> for RewriteAnalysis<'a, F, O> {
    fn best(sizes: &StitchAnalysisRunner<F::Apply<O>, Self>, eclass: Id) -> i64 {
        // Try not rewriting self but YES allowing rewrites of descendants
        // (technically we could just use sizes.original_size if we knew we weren't enqueued by a child)
        let mut best = sizes.min_enode_size(eclass);
        // For every way we match at this eclass (if any), try all ways of rewriting it
        if let Some(&i) = sizes.analysis.eclass_to_match_idx.get(&eclass) {
            let substs = &sizes.analysis.search_state.matches[i].substs;
            let weights = sizes.weights();
            if let Some(rewrite_size) = substs.iter().map(|subst| F::stub_application_size::<O>("inv_0", subst.vars.len(), weights) as i64 + sizes.sum(&subst.vars)).min() {
                best = best.min(rewrite_size);
            }
        }
        best
    }
}

/// Computes the exact syntactic free-variable set at every position of `expr`,
/// indexed by `usize::from(Id)`. Shares its per-enode rule with
/// `StitchAnalysis::make` via `enode_fv`.
pub fn recexpr_fv<L: StitchLanguage>(expr: &RecExpr<L>) -> Vec<FxHashSet<u32>> {
    let nodes: &[L] = expr.as_ref();
    let mut fv: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        fv[i] = enode_fv(node, |c| &fv[usize::from(c)]);
    }
    fv
}

/// Asserts that the extracted term's actual syntactic fv matches the egraph
/// analysis's recorded fv. Under intersection-fv semantics + AstSize
/// extraction, the minimal-size representative is also the fv-minimal one,
/// so its fv should equal the intersection across reps — i.e. `expected`.
/// A mismatch in either direction means the assumption "min-size ⇒ min-fv"
/// failed for this extraction; downstream soundness checks that read
/// `data.fv` lose their guarantee.
pub fn check_fvs_are_as_expected<L: StitchLanguage>(expr: &RecExpr<L>, expected: &FxHashSet<u32>) {
    let fv = recexpr_fv(expr);
    let actual = fv.last().expect("non-empty RecExpr");
    assert_eq!(actual, expected, "extracted RecExpr fv {:?} differs from egraph analysis fv {:?}; intersection-fv assumption (min-size rep is fv-minimal) violated", actual, expected,);
}