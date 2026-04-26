use crate::lang::{StitchEgraph, StitchLang};
use crate::search::SearchState;

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub fn build_rewritten_egraph(egraph: &StitchEgraph, search_state: &SearchState) -> StitchEgraph {
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
