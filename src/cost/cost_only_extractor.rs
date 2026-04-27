use egg::{Analysis, CostFunction, EClass, EGraph, Id, Language};
use rustc_hash::FxHashMap;

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
