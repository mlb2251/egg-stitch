use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// True iff every metavar in `subst` has a captured e-class whose fv contains
/// no pattern-internal binder index (i.e. all fv `≥ d_k`). Substs that fail
/// this can't be soundly emitted as `(fn_n captures…)` under stitch's plain-
/// substitution convention, but they're still *kept* in match sets during
/// search so further expansion can refine them into closed-prefix captures.
pub fn subst_is_sound<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, var_depth: &[u32], subst: &Subst) -> bool {
    subst.vars.iter().enumerate().all(|(k, &arg_id)| {
        let d_k = var_depth[k];
        egraph[arg_id].data.fv.iter().all(|&i| i >= d_k)
    })
}

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

    let get_size = |eclass: Id, s_u_r: &FxHashMap<Id, i64>| -> i64 { s_u_r.get(&eclass).cloned().unwrap_or(egraph[eclass].data.size as i64) };

    // Substs whose captures still have pattern-internal-bound fv aren't yet
    // sound; we skip them here (they're kept in `state.matches` so the search
    // can refine them, but they don't yet realize compression).
    let var_depth = &search_state.pattern.var_depth;

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
                if !subst_is_sound::<F, O>(egraph, var_depth, subst) {
                    continue;
                }
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
        let slow_size = build_rewritten_egraph(egraph, search_state)[root].data.size as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub(crate) fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> StitchEgraph<F::Apply<O>> {
    let mut egraph = egraph.clone();
    let var_depth = &search_state.pattern.var_depth;
    for m in &search_state.matches {
        for subst in &m.substs {
            if !subst_is_sound::<F, O>(&egraph, var_depth, subst) {
                continue;
            }
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
    let var_depth = &search_state.pattern.var_depth;
    rewritten[root].nodes[0].children().iter().map(|&child| {
        let (_, expr) = extractor.find_best(child);
        check_fvs_are_as_expected(&expr, var_depth);
        <F::Apply<O> as StitchLanguage>::display_recexpr(&expr)
    }).collect()
}

/// Computes the exact syntactic free-variable set at every position of `expr`,
/// indexed by `usize::from(Id)`. Mirrors the per-enode rule used in
/// `StitchAnalysis::make`, but on a concrete tree (no over-/under-approximation).
fn recexpr_fv<L: StitchLanguage>(expr: &RecExpr<L>) -> Vec<FxHashSet<u32>> {
    let nodes: &[L] = expr.as_ref();
    let mut fv: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        let disc = node.discriminant();
        let mut s = FxHashSet::default();
        if let Some(n) = disc.de_bruijn_index() {
            s.insert(n);
        }
        for (j, &c) in node.children().iter().enumerate() {
            let child_fv = &fv[usize::from(c)];
            if disc.binds_child(j) {
                s.extend(child_fv.iter().filter_map(|&k| if k >= 1 { Some(k - 1) } else { None }));
            } else {
                s.extend(child_fv.iter().copied());
            }
        }
        fv[i] = s;
    }
    fv
}

/// If `id` is the head of a fully-saturated `name(arg1, …, argN)` stub
/// application in `expr`, returns `Some(args)`. Handles both flat
/// (children-on-head, e.g. `OpChildrenLanguage`) and curried-`@` forms
/// (e.g. `LambdaCalcLanguage`). For curried chains, only the outermost
/// (fully-applied) node returns `Some` with the complete arg list; inner
/// `@` nodes return `None` from this check via the arity filter at the
/// call site.
fn match_stub_application<L: StitchLanguage>(expr: &RecExpr<L>, id: Id, name: &str) -> Option<Vec<Id>> {
    let node = &expr[id];
    let disc_name = node.discriminant().to_string();
    if disc_name == name {
        return Some(node.children().to_vec());
    }
    if disc_name == "@" && node.children().len() == 2 {
        let kids = node.children();
        if let Some(mut args) = match_stub_application(expr, kids[0], name) {
            args.push(kids[1]);
            return Some(args);
        }
    }
    None
}

/// Walks `expr` and, for every fully-applied `inv_0(arg1, …, argN)`,
/// asserts that arg `k`'s actual syntactic fv is `⊆ [var_depth[k], ∞)` —
/// i.e., none of the pattern-internal binder indices are free in the
/// extracted representative. Catches cases where the eclass-level
/// soundness filter (`subst_is_sound`) admitted a subst because the
/// intersection-fv analysis allowed it, but the cost extractor then
/// picked a representative whose actual fv violates the bound.
pub fn check_fvs_are_as_expected<L: StitchLanguage>(expr: &RecExpr<L>, var_depth: &[u32]) {
    let nodes: &[L] = expr.as_ref();
    let fv = recexpr_fv(expr);
    for i in 0..nodes.len() {
        if let Some(args) = match_stub_application::<L>(expr, Id::from(i), "inv_0") {
            if args.len() != var_depth.len() {
                continue;
            }
            for (k, &arg) in args.iter().enumerate() {
                let d_k = var_depth[k];
                let arg_fv = &fv[usize::from(arg)];
                assert!(
                    arg_fv.iter().all(|&j| j >= d_k),
                    "inv_0 arg {k} has fv {:?} containing index < d_k={d_k}; extractor picked an unsound representative",
                    arg_fv,
                );
            }
        }
    }
}
