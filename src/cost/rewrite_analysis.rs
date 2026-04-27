use super::{StitchAnalysis, StitchAnalysisRunner};
use crate::lang::{LanguageFamily, StitchOp};
use crate::search::SearchState;
use egg::Id;
use rustc_hash::FxHashMap;

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
