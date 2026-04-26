use super::{StitchAnalysis, StitchAnalysisRunner};
use crate::matching::Subst;
use crate::search::SearchState;
use egg::Id;
use rustc_hash::FxHashMap;

/// Map from match-root eclass to the substitutions that apply there. Build once
/// (via `build_eclass_to_substs`) and pass by reference to every analysis run.
pub type EclassToSubsts<'a> = FxHashMap<Id, &'a Vec<Subst>>;

/// Builds the eclass→substs map from a `SearchState`. Reuse across analyses.
pub fn build_eclass_to_substs(search_state: &SearchState) -> EclassToSubsts<'_> {
    let mut m = FxHashMap::default();
    for match_ in &search_state.matches {
        m.insert(match_.root_eclass, &match_.substs);
    }
    m
}

/// Default analysis: at each match root, rewriting via `inv_0(args...)` is allowed,
/// otherwise we fall back to the minimum enode size. Holds the full subst map
/// because it needs the per-subst arg lists to size the rewrite.
pub struct RewriteAnalysis<'a> {
    pub eclass_to_substs: &'a EclassToSubsts<'a>,
}
impl<'a> RewriteAnalysis<'a> {
    pub fn new(eclass_to_substs: &'a EclassToSubsts<'a>) -> Self {
        Self { eclass_to_substs }
    }
}
impl<'a> StitchAnalysis for RewriteAnalysis<'a> {
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id> {
        sizes.analysis.eclass_to_substs.keys().copied().collect()
    }
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64 {
        // Try not rewriting self but YES allowing rewrites of descendants
        // (technically we could just use sizes.original_size if we knew we weren't enqueued by a child)
        let mut best = sizes.min_enode_size(eclass);
        // For every way we match at this eclass (if any), try all ways of rewriting it
        if let Some(substs) = sizes.analysis.eclass_to_substs.get(&eclass) {
            if let Some(rewrite_size) = substs.iter().map(|subst| 1 + sizes.sum(&subst.vars)).min() {
                best = best.min(rewrite_size);
            }
        }
        best
    }
}
