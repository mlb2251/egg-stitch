use super::{CostCache, StitchAnalysis, StitchAnalysisRunner};
use crate::lang::StitchEgraph;
use egg::Id;
use rustc_hash::FxHashSet;

/// Optimistic analysis producing a lower bound on achievable size: if any subst
/// applies at this eclass, assume the rewrite collapses it to a single node (size 1);
/// otherwise fall back to the minimum enode size. Only needs the *set* of match-root
/// eclasses, not the substs themselves.
pub struct LowerBoundAnalysis<'a> {
    pub match_eclasses: &'a FxHashSet<Id>,
}
impl<'a> StitchAnalysis for LowerBoundAnalysis<'a> {
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id> {
        sizes.analysis.match_eclasses.iter().copied().collect()
    }
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64 {
        if sizes.analysis.match_eclasses.contains(&eclass) {
            1
        } else {
            sizes.min_enode_size(eclass)
        }
    }
}

/// Computes an optimistic lower bound on corpus size by assuming every match collapses
/// to a single node.
pub fn compute_lower_bound(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, match_eclasses: &FxHashSet<Id>) -> usize {
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, LowerBoundAnalysis { match_eclasses });
    sizes.solve();
    sizes.get(root) as usize
}
