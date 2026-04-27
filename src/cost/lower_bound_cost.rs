use super::{CostCache, CostScratch, StitchAnalysis, StitchAnalysisRunner};
use crate::lang::{LanguageFamily, StitchEgraph, StitchLanguage, StitchOp};
use crate::search::SearchState;
use egg::Id;

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
