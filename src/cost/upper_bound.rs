use super::cost_only_extractor::CostOnlyExtractor;
use super::rewrite_analysis::{build_eclass_to_substs, RewriteAnalysis};
use super::{CostCache, StitchAnalysisRunner};
use crate::lang::{StitchEgraph, StitchLang};
use crate::pattern::Pattern;
use crate::search::SearchState;

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern);
    cost + pattern_size
}

/// Returns the AST size of the pattern
pub fn compute_pattern_size(pattern: &Pattern) -> usize {
    pattern.pattern.size()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a postorder min-heap so children pop before parents. Initial entries are the
/// match-root eclasses; when an eclass's size strictly improves we write it into
/// `sizes` and push its parents so they can reconsider with the new child value.
pub(crate) fn compute_size(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let eclass_to_substs = build_eclass_to_substs(search_state);
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, RewriteAnalysis::new(&eclass_to_substs));
    sizes.solve();
    let final_size = sizes.get(root);
    if check_slow {
        let rewritten = build_rewritten_egraph(egraph, search_state);
        let slow_size = rewritten[root].data as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
        let cost_only = CostOnlyExtractor::new(&rewritten, egg::AstSize);
        let cost_only_size = cost_only.cost(root).expect("root has no cost") as i64;
        assert_eq!(final_size, cost_only_size, "Fast rewrite size {} != CostOnlyExtractor size {}", final_size, cost_only_size);
    }
    final_size as usize
}

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub(crate) fn build_rewritten_egraph(egraph: &StitchEgraph, search_state: &SearchState) -> StitchEgraph {
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
