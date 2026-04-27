use super::cost_only_extractor::CostOnlyExtractor;
use super::rewrite_analysis::RewriteAnalysis;
use super::{CostCache, CostScratch, StitchAnalysisRunner};
use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::pattern::{Pattern, PatternRecExpr};
use crate::rewrite::build_rewritten_egraph;
use crate::search::SearchState;
use egg::RecExpr;

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, scratch, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern, &egraph.analysis.weights);
    cost + pattern_size
}

/// Returns the AST size of the pattern, respecting each operator's `intrinsic_size()`.
/// Var nodes contribute via `OpWithVar::Var`'s `intrinsic_size = 1`, so the recursion
/// is uniform across var and non-var slots.
pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, weights: &Weights) -> usize {
    let rec_expr: RecExpr<F::Apply<crate::lang::OpWithVar<O>>> = PatternRecExpr::<F, O>::clone(&pattern.pattern).into();
    compute_recexpr_size::<F::Apply<crate::lang::OpWithVar<O>>>(&rec_expr, (rec_expr.len() - 1).into(), weights)
}

/// Recursive AST size of a `RecExpr<L>`, respecting `intrinsic_size()`.
pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: egg::Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a postorder min-heap so children pop before parents. Initial entries are the
/// match-root eclasses; when an eclass's size strictly improves we write it into
/// `sizes` and push its parents so they can reconsider with the new child value.
pub(crate) fn compute_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, scratch: &mut CostScratch, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    scratch.rewrite.fill(search_state);
    let analysis = RewriteAnalysis {
        search_state,
        eclass_to_match_idx: &scratch.rewrite.eclass_to_match_idx,
    };
    let mut sizes = StitchAnalysisRunner::new(egraph, cache, &mut scratch.runner, analysis);
    for m in &search_state.matches {
        sizes.mark_dirty(m.root_eclass);
    }
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
