use crate::lang::StitchLang;
use crate::search::SearchState;
use crate::smc::StitchEgraph;

/// Slow but simple rewrite cost computation via egraph cloning.
/// Useful for debugging / validating the fast `compute_size` implementation.
pub fn rewrite_slow(
    egraph: &StitchEgraph,
    root: egg::Id,
    search_state: &SearchState,
) -> usize {
    let mut egraph = egraph.clone(); // todo be smarter

    for m in &search_state.matches {
        for subst in &m.substs {
            let node: StitchLang = StitchLang {
                op: "inv_0".into(),
                children: subst.vars.clone(),
            };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph[root].data as usize
}
