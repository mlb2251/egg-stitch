use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    postorder: Vec<Option<u32>>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    parents_of: FxHashMap<Id, Vec<Id>>,
}

impl CostCache {
    /// Builds the cache from the egraph rooted at `root`.
    pub fn new<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> Self {
        let mut parents_of = FxHashMap::<Id, Vec<Id>>::default();
        for class in egraph.classes() {
            for enode in &class.nodes {
                for &child in enode.children() {
                    parents_of.entry(child).or_default().push(class.id);
                }
            }
        }

        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut postorder = vec![None; max_id + 1];
        let mut order: u32 = 0;
        let mut stack: Vec<Result<Id, Id>> = vec![Err(root)]; // Err=enter, Ok=exit
        let mut on_stack = FxHashSet::<Id>::default();
        while let Some(state) = stack.pop() {
            match state {
                Err(id) => {
                    if postorder[usize::from(id)].is_some() || !on_stack.insert(id) {
                        continue;
                    }
                    stack.push(Ok(id));
                    for enode in &egraph[id].nodes {
                        for &child in enode.children() {
                            stack.push(Err(child));
                        }
                    }
                }
                Ok(id) => {
                    on_stack.remove(&id);
                    postorder[usize::from(id)] = Some(order);
                    order += 1;
                }
            }
        }

        Self { postorder, parents_of }
    }
}

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern, &egraph.analysis.weights);
    cost + pattern_size
}

pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, weights: &Weights) -> usize {
    let rec_expr: RecExpr<F::Apply<crate::lang::OpWithVar<O>>> = pattern.pattern.clone().into();
    compute_recexpr_size::<F::Apply<crate::lang::OpWithVar<O>>>(&rec_expr, (rec_expr.len() - 1).into(), weights)
}

pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a work-queue ordered by postorder (children before parents) so each
/// eclass is visited at most once.
pub(crate) fn compute_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let mut eclass_to_matches = FxHashMap::<Id, &Vec<Subst>>::default();
    for m in &search_state.matches {
        eclass_to_matches.insert(m.root_eclass, &m.substs);
    }

    let get_size = |eclass: Id, s_u_r: &FxHashMap<Id, i64>| -> i64 { s_u_r.get(&eclass).cloned().unwrap_or(egraph[eclass].data as i64) };

    let mut size_under_rewrite = FxHashMap::<Id, i64>::default();
    let mut work_queue = BinaryHeap::new();
    for m in &search_state.matches {
        work_queue.push(Reverse((cache.postorder[usize::from(m.root_eclass)].unwrap(), m.root_eclass)));
    }
    while let Some(Reverse((_, eclass))) = work_queue.pop() {
        if size_under_rewrite.contains_key(&eclass) {
            continue;
        }
        let size_current = get_size(eclass, &size_under_rewrite);
        let mut best = size_current;
        let weights = &egraph.analysis.weights;
        if let Some(substs) = eclass_to_matches.get(&eclass) {
            for subst in *substs {
                let stub_size = F::stub_application_size::<O>("inv_0", subst.vars.len(), weights) as i64;
                let size_new: i64 = stub_size + subst.vars.iter().map(|&v| get_size(v, &size_under_rewrite)).sum::<i64>();
                if size_new < best {
                    best = size_new;
                }
            }
        }
        for enode in &egraph[eclass].nodes {
            let size_no_rewrite: i64 = enode.discriminant().intrinsic_size(weights) as i64 + enode.children().iter().map(|&c| get_size(c, &size_under_rewrite)).sum::<i64>();
            if size_no_rewrite < best {
                best = size_no_rewrite;
            }
        }
        if best < size_current {
            if let Some(parents) = cache.parents_of.get(&eclass) {
                for &parent in parents {
                    if let Some(po) = cache.postorder[usize::from(parent)] {
                        work_queue.push(Reverse((po, parent)));
                    }
                }
            }
            size_under_rewrite.insert(eclass, best);
        }
    }
    let final_size = get_size(root, &size_under_rewrite);
    if check_slow {
        let slow_size = build_rewritten_egraph(egraph, search_state)[root].data as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub(crate) fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> StitchEgraph<F::Apply<O>> {
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
    rewritten[root].nodes[0].children().iter().map(|&child| <F::Apply<O> as StitchLanguage>::display_recexpr(&extractor.find_best(child).1)).collect()
}
