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

/// Sparse per-eclass size map with a fallback to the unrewritten AstSize (`egraph[id].data`).
/// Entries represent eclasses whose rewritten size is strictly smaller than the default.
struct Sizes<'a> {
    egraph: &'a StitchEgraph,
    cache: &'a CostCache,
    overrides: FxHashMap<Id, i64>,
}
impl Sizes<'_> {
    fn get(&self, id: Id) -> i64 {
        self.overrides.get(&id).copied().unwrap_or(self.original_size(id))
    }
    fn set(&mut self, id: Id, v: i64) {
        self.overrides.insert(id, v);
    }
    fn contains(&self, id: Id) -> bool {
        self.overrides.contains_key(&id)
    }
    /// Sum of `get` over a list of eclass ids.
    fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    fn original_size(&self, id: Id) -> i64 {
        self.egraph[id].data as i64
    }
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a postorder min-heap so children pop before parents. Initial entries are the
/// match-root eclasses; when an eclass's size strictly improves we write it into
/// `sizes` and push its parents so they can reconsider with the new child value.
pub(crate) fn compute_size(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let mut eclass_to_substs = FxHashMap::<Id, &Vec<Subst>>::default();
    let mut sizes = Sizes { egraph, cache, overrides: FxHashMap::default() };
    let mut work_queue = BinaryHeap::new();
    for m in &search_state.matches {
        eclass_to_substs.insert(m.root_eclass, &m.substs);
        work_queue.push(Reverse((sizes.cache.postorder[usize::from(m.root_eclass)].unwrap(), m.root_eclass)));
    }
    while let Some(Reverse((_, eclass))) = work_queue.pop() {
        if sizes.contains(eclass) {
            continue;
        }

        // size without rewriting self NOR any descendants
        let size_current = sizes.original_size(eclass);
        let mut best = size_current;

        // For every way we match at this eclass (if any), try all ways of rewriting it
        // (relies on postorder guaranteeing descendants (arguments) have sizes.get done)
        if let Some(substs) = eclass_to_substs.get(&eclass) {
            for subst in *substs {
                best = best.min(1 + sizes.sum(&subst.vars));
            }
        }

        // Try not rewriting self but YES allowing rewrites of descendants
        // (relies on postorder guaranteeing children have sizes.get done)
        for enode in &egraph[eclass].nodes {
            best = best.min(1 + sizes.sum(&enode.children));
        }

        // If we found a smaller size than the "no rewriting and no descendant rewriting" size, push
        // our parents to the queue to make sure they get updated
        if best < size_current {
            if let Some(parents) = sizes.cache.parents_of.get(&eclass) {
                for &parent in parents {
                    if let Some(po) = sizes.cache.postorder[usize::from(parent)] {
                        work_queue.push(Reverse((po, parent)));
                    }
                }
            }
            sizes.set(eclass, best);
        }
    }
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
