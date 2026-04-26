use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Analysis, CostFunction, EClass, EGraph, Id, Language};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    postorder: Vec<Option<u32>>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    parents_of: FxHashMap<Id, Vec<Id>>,
}

impl CostCache {
    /// Builds the cache from the egraph rooted at `root`.
    pub fn new(egraph: &StitchEgraph, root: Id) -> Self {
        let mut parents_of = FxHashMap::<Id, Vec<Id>>::default();
        for class in egraph.classes() {
            for enode in &class.nodes {
                for &child in &enode.children {
                    parents_of.entry(child).or_default().push(class.id);
                }
            }
        }

        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut postorder = vec![None; max_id + 1];
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
                        for &child in &enode.children {
                            stack.push(Err(child));
                        }
                    }
                }
                Ok(id) => {
                    on_stack.remove(&id);
                    postorder[usize::from(id)] = Some(order);
                    order += 1;
                }
            }
        }

        Self { postorder, parents_of }
    }
}

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern);
    cost + pattern_size
}

/// Returns the AST size of the pattern (counting each node and edge once).
pub fn compute_pattern_size(pattern: &Pattern) -> usize {
    1 + pattern.pattern.nodes.iter().map(|node| node.children().len()).sum::<usize>()
}

/// Pluggable per-eclass relaxation rule. The analysis decides which eclasses seed the
/// work queue and how to compute a candidate size for an eclass given the current
/// `StitchAnalysisRunner` state. Implemented as associated functions (no `&self`) so the solver can
/// pass `&StitchAnalysisRunner<Self>` without conflicting borrows; analysis-owned data is reached
/// via `sizes.analysis`, while shared match info lives on `sizes.eclass_to_substs`.
pub trait StitchAnalysis: Sized {
    /// Eclasses to push onto the work queue when the solver starts.
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id>;
    /// Candidate size for `eclass` given currently known sizes.
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64;
}

/// Default analysis: at each match root, rewriting via `inv_0(args...)` is allowed,
/// otherwise we fall back to the minimum enode size.
pub struct RewriteAnalysis;
impl StitchAnalysis for RewriteAnalysis {
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id> {
        sizes.eclass_to_substs.keys().copied().collect()
    }
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64 {
        // Try not rewriting self but YES allowing rewrites of descendants
        // (technically we could just use sizes.original_size if we knew we weren't enqueued by a child)
        let mut best = sizes.min_enode_size(eclass);
        // For every way we match at this eclass (if any), try all ways of rewriting it
        if let Some(substs) = sizes.eclass_to_substs.get(&eclass) {
            if let Some(rewrite_size) = substs.iter().map(|subst| 1 + sizes.sum(&subst.vars)).min() {
                best = best.min(rewrite_size);
            }
        }
        best
    }
}

/// Optimistic lower-bound analysis: if any subst applies at this eclass, assume the
/// rewrite collapses it to a single node (size 1); otherwise fall back to the minimum
/// enode size. Useful as a cheap upper bound on achievable compression.
pub struct UpperBoundAnalysis;
impl StitchAnalysis for UpperBoundAnalysis {
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id> {
        sizes.eclass_to_substs.keys().copied().collect()
    }
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64 {
        if let Some(substs) = sizes.eclass_to_substs.get(&eclass) {
            debug_assert!(!substs.is_empty());
            1
        } else {
            sizes.min_enode_size(eclass)
        }
    }
}

/// Sparse per-eclass size map with a fallback to the unrewritten AstSize (`egraph[id].data`).
/// Entries represent eclasses whose rewritten size is strictly smaller than the default.
/// `eclass_to_substs` is shared state available to every analysis.
pub struct StitchAnalysisRunner<'a, A: StitchAnalysis> {
    egraph: &'a StitchEgraph,
    cache: &'a CostCache,
    overrides: FxHashMap<Id, i64>,
    work_queue: BinaryHeap<Reverse<(u32, Id)>>,
    pub eclass_to_substs: FxHashMap<Id, &'a Vec<Subst>>,
    pub analysis: A,
}
impl<'a, A: StitchAnalysis> StitchAnalysisRunner<'a, A> {
    /// Builds an empty size table seeded with the analysis's chosen eclasses.
    fn new(egraph: &'a StitchEgraph, cache: &'a CostCache, search_state: &'a SearchState, analysis: A) -> Self {
        let mut eclass_to_substs = FxHashMap::default();
        for m in &search_state.matches {
            eclass_to_substs.insert(m.root_eclass, &m.substs);
        }
        let mut sizes = StitchAnalysisRunner {
            egraph,
            cache,
            overrides: FxHashMap::default(),
            work_queue: BinaryHeap::new(),
            eclass_to_substs,
            analysis,
        };
        for id in A::init(&sizes) {
            sizes.work_queue.push(Reverse((cache.postorder[usize::from(id)].unwrap(), id)));
        }
        sizes
    }
    pub fn get(&self, id: Id) -> i64 {
        self.overrides.get(&id).copied().unwrap_or(self.original_size(id))
    }
    fn set(&mut self, id: Id, v: i64) {
        self.overrides.insert(id, v);
    }
    fn contains(&self, id: Id) -> bool {
        self.overrides.contains_key(&id)
    }
    /// Sum of `get` over a list of eclass ids.
    pub fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    pub fn original_size(&self, id: Id) -> i64 {
        self.egraph[id].data as i64
    }
    /// Minimum size over the enodes of `eclass`. Panics if the eclass has no enodes.
    pub fn min_enode_size(&self, eclass: Id) -> i64 {
        self.egraph[eclass].nodes.iter().map(|enode| 1 + self.sum(&enode.children)).min().unwrap()
    }
    /// If `new` improves on the current size of `eclass`, record it and enqueue parents for re-relaxation.
    fn update(&mut self, eclass: Id, new: i64) {
        if new < self.get(eclass) {
            self.notify_parents(eclass);
            self.set(eclass, new);
        }
    }
    /// Runs the postorder relaxation until the work queue drains.
    fn solve(&mut self) {
        while let Some(Reverse((_, eclass))) = self.work_queue.pop() {
            if self.contains(eclass) {
                continue;
            }
            let best = A::best(self, eclass);
            self.update(eclass, best);
        }
    }
    /// Re-enqueues every parent of `eclass` so they reconsider the new child size.
    fn notify_parents(&mut self, eclass: Id) {
        if let Some(parents) = self.cache.parents_of.get(&eclass) {
            for &parent in parents {
                if let Some(po) = self.cache.postorder[usize::from(parent)] {
                    self.work_queue.push(Reverse((po, parent)));
                }
            }
        }
    }
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a postorder min-heap so children pop before parents. Initial entries are the
/// match-root eclasses; when an eclass's size strictly improves we write it into
/// `sizes` and push its parents so they can reconsider with the new child value.
pub(crate) fn compute_size(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, search_state, RewriteAnalysis);
    sizes.solve();
    let final_size = sizes.get(root);
    if check_slow {
        let rewritten = build_rewritten_egraph(egraph, search_state);
        let slow_size = rewritten[root].data as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
        let cost_only = CostOnlyExtractor::new(&rewritten, egg::AstSize);
        let cost_only_size = cost_only.cost(root).expect("root has no cost") as i64;
        assert_eq!(final_size, cost_only_size, "Fast rewrite size {} != CostOnlyExtractor size {}", final_size, cost_only_size);
    }
    final_size as usize
}

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub(crate) fn build_rewritten_egraph(egraph: &StitchEgraph, search_state: &SearchState) -> StitchEgraph {
    let mut egraph = egraph.clone();
    for m in &search_state.matches {
        for subst in &m.substs {
            let node = StitchLang { op: "inv_0".into(), children: subst.vars.clone() };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph
}

/// Extracts each program from the rewritten egraph, using `inv_0` where it reduces size.
pub fn extract_rewritten_programs(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState) -> Vec<String> {
    let rewritten = build_rewritten_egraph(egraph, search_state);
    let extractor = egg::Extractor::new(&rewritten, egg::AstSize);
    rewritten[root].nodes[0].children.iter().map(|&child| extractor.find_best(child).1.to_string()).collect()
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
        class
            .iter()
            .filter_map(|n| self.node_cost(n))
            .min_by(|a, b| a.partial_cmp(b).expect("CostFunction returned incomparable costs"))
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
