use super::{CostCache, CostScratch, StitchAnalysis, StitchAnalysisRunner};
use crate::lang::StitchEgraph;
use crate::search::SearchState;
use egg::Id;
use rustc_hash::FxHashSet;

/// Reusable set of match-root eclasses for the lower-bound analysis.
#[derive(Default)]
pub struct LowerScratch {
    pub match_eclasses: FxHashSet<Id>,
}

impl LowerScratch {
    /// Refills the set from `search_state`. Clears first; retains capacity.
    pub fn fill(&mut self, search_state: &SearchState) {
        self.match_eclasses.clear();
        for m in &search_state.matches {
            self.match_eclasses.insert(m.root_eclass);
        }
    }
}

/// Optimistic analysis producing a lower bound on achievable size: if any subst
/// applies at this eclass, assume the rewrite collapses it to a single node (size 1);
/// otherwise fall back to the minimum enode size. Only needs the *set* of match-root
/// eclasses, not the substs themselves.
pub struct LowerBoundAnalysis<'a> {
    pub match_eclasses: &'a FxHashSet<Id>,
}
impl<'a> StitchAnalysis for LowerBoundAnalysis<'a> {
    fn init(&self, out: &mut Vec<Id>) {
        out.extend(self.match_eclasses.iter().copied());
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
/// to a single node. Reuses allocations in `scratch` across calls.
pub fn compute_lower_bound(egraph: &StitchEgraph, root: Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState) -> usize {
    scratch.lower.fill(search_state);
    let analysis = LowerBoundAnalysis { match_eclasses: &scratch.lower.match_eclasses };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    sizes.solve();
    sizes.get(root) as usize
}
