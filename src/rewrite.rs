use crate::lang::{LanguageFamily, StitchEgraph, StitchOp,StitchLanguage};
use crate::search::SearchState;
use crate::cost::check_fvs_are_as_expected;
use egg::Language;

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> StitchEgraph<F::Apply<O>> {
    let mut egraph = egraph.clone();
    for m in &search_state.matches {
        for subst in &m.substs {
            let x = F::add_stub_application::<O>("inv_0", subst.vars.clone(), &mut egraph);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph
}

/// Extracts each program from the rewritten egraph, using `inv_0` where it reduces size.
pub fn extract_rewritten_programs<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, search_state: &SearchState<F, O>) -> Vec<String> {
    let rewritten = build_rewritten_egraph(egraph, search_state);
    let extractor = egg::Extractor::new(&rewritten, egg::AstSize);
    rewritten[root].nodes[0]
        .children()
        .iter()
        .map(|&child| {
            let (_, expr) = extractor.find_best(child);
            check_fvs_are_as_expected::<F::Apply<O>>(&expr, &rewritten[child].data.fv);
            <F::Apply<O> as StitchLanguage>::display_recexpr(&expr)
        })
        .collect()
}


