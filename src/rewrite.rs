use crate::lang::{LanguageFamily, StitchEgraph, StitchOp,StitchLanguage};
use crate::search::SearchState;
use crate::cost::check_fvs_are_as_expected;
use egg::Language;

/// Clones the egraph and unions each match root with an `inv_0(args...)`
/// node, then rebuilds. Source of truth for the rewrite — `compute_size`'s
/// fast path is validated against this via `check_slow`.
///
/// For each k with `ho_arity[k] > 0`, the captured eclass is shifted (fv
/// `≥ d_k` up by `ho_arity[k]`) and wrapped under `ho_arity[k]` λs before
/// being passed in.
pub fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>, ho_arity: &[u32]) -> StitchEgraph<F::Apply<O>> {
    let mut egraph = egraph.clone();
    let var_depth = &search_state.pattern.var_depth;
    let mut shift_memo: FxHashMap<(Id, u32), Id> = FxHashMap::default();
    for m in &search_state.matches {
        for subst in &m.substs {
            let wrapped = wrap_subst_args::<F, O>(&mut egraph, &subst.vars, ho_arity, var_depth, &mut shift_memo);
            let x = F::add_stub_application::<O>("inv_0", wrapped, &mut egraph);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph
}

/// Extracts each program from the rewritten egraph, using `inv_0` where it reduces size.
pub fn extract_rewritten_programs<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, search_state: &SearchState<F, O>) -> Vec<String> {
    let ho_arity = compute_ho_arity::<F, O>(egraph, search_state);
    let rewritten = build_rewritten_egraph(egraph, search_state, &ho_arity);
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


