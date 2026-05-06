use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights, enode_fv};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Per-metavar higher-order arity. `ho_arity[k]` is the number of
/// pattern-internal binders we abstract over when emitting `?#k`'s captured
/// argument. Zero means plain capture (no body wrapping needed).
///
/// Computed as `max over matches m of needed(m, k)`, where
/// `needed(m, k) = max{i + 1 : i ∈ fv(arg_{m,k}), i < d_k}` (or 0 if no such i).
/// Taking the max ensures all call sites of `inv_0` agree on the body's
/// `(@ … (@ ?#k $0) …)` shape.
pub fn compute_ho_arity<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<u32> {
    let arity = search_state.pattern.var_depth.len();
    let mut out = vec![0u32; arity];
    let var_depth = &search_state.pattern.var_depth;
    for m in &search_state.matches {
        for subst in &m.substs {
            for (k, &arg_id) in subst.vars.iter().enumerate() {
                let d_k = var_depth[k];
                let needed = egraph[arg_id].data.fv.iter().filter(|&&i| i < d_k).map(|&i| i + 1).max().unwrap_or(0);
                if needed > out[k] {
                    out[k] = needed;
                }
            }
        }
    }
    out
}

/// Build a copy of `eclass` in `egraph` with every free DB index `≥ initial_depth`
/// shifted by `+by`, so it can sit under `by` newly-introduced binders without
/// changing meaning. Picks the size-minimal enode per visited eclass (using the
/// analysis's `data.size`, which is the same quantity AstSize would minimize)
/// so the shifted witness is as small as possible.
///
/// Memoized per `(eclass, initial_depth)` for the lifetime of `memo`. Note `by`
/// is fixed per top-level call, so it isn't part of the key.
pub(crate) fn shift_free_egraph<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, eclass: Id, by: u32, initial_depth: u32, memo: &mut FxHashMap<(Id, u32), Id>) -> Id {
    let canonical = egraph.find(eclass);
    if let Some(&cached) = memo.get(&(canonical, initial_depth)) {
        return cached;
    }
    // If no fv `≥ initial_depth` is present in this class, the shift is a no-op
    // — return the original eclass to preserve sharing.
    if egraph[canonical].data.fv.iter().all(|&i| i < initial_depth) {
        memo.insert((canonical, initial_depth), canonical);
        return canonical;
    }
    // Pick the size-minimal enode by recomputing the analysis's `make` formula
    // over the current class. Done inline so mid-recursion `egraph.add`s can't
    // make the choice stale.
    let weights = egraph.analysis.weights;
    let rep = egraph[canonical]
        .nodes
        .iter()
        .min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&c| egraph[c].data.size as u64).sum::<u64>())
        .expect("non-empty eclass")
        .clone();
    // Under intersection-fv semantics the size-minimal rep is also fv-minimal,
    // so its syntactic fv should match the eclass's analysis fv. Mirrors the
    // assertion in `check_fvs_are_as_expected` for the extracted-RecExpr path.
    let rep_fv = enode_fv(&rep, |c| &egraph[c].data.fv);
    assert_eq!(
        &rep_fv, &egraph[canonical].data.fv,
        "shift_free_egraph rep fv {:?} differs from eclass data.fv {:?}; intersection-fv assumption (min-size rep is fv-minimal) violated",
        rep_fv, egraph[canonical].data.fv
    );
    let disc = rep.discriminant();
    if let Some(n) = disc.de_bruijn_index() {
        // Free DB-var leaf: rebuild with shifted index. (Bound vars `< initial_depth`
        // were already short-circuited by the fv check above.)
        let new_disc = F::map_discriminant(disc, |_| O::make_db_var(n + by).expect("higher-order capture requires a DB-var-bearing leaf op"));
        let new_id = egraph.add(F::make(new_disc, vec![]));
        memo.insert((canonical, initial_depth), new_id);
        return new_id;
    }
    let new_children: Vec<Id> = rep
        .children()
        .iter()
        .enumerate()
        .map(|(j, &c)| {
            let child_depth = initial_depth + if disc.binds_child(j) { 1 } else { 0 };
            shift_free_egraph::<F, O>(egraph, c, by, child_depth, memo)
        })
        .collect();
    let new_id = egraph.add(F::make(disc, new_children));
    memo.insert((canonical, initial_depth), new_id);
    new_id
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

/// Returns the total cost: compressed corpus size plus the abstraction's own
/// pattern body size. Each `?#k` with `ho_arity[k] > 0` has its body uses
/// applied to the enclosing binders (`(@ … (@ ?#k $0) … $h-1)`), which adds
/// `h * (app_cost + sym_var_cost)` per occurrence.
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let ho_arity = compute_ho_arity::<F, O>(egraph, search_state);
    let cost = compute_size(egraph, root, cache, search_state, check_slow, &ho_arity);
    let body_size = compute_body_size_with_ho::<F, O>(&search_state.pattern, &ho_arity, &egraph.analysis.weights);
    cost + body_size
}

/// Size of the abstraction's pattern body — the pattern AST counted under
/// the active weights. Each `?#k` is a 0-arity meta-var leaf; HO body apps
/// are not included here (use `compute_body_size_with_ho` for the
/// inclusive form).
pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, weights: &Weights) -> usize {
    let rec_expr: RecExpr<F::Apply<crate::lang::OpWithVar<O>>> = pattern.pattern.clone().into();
    compute_recexpr_size::<F::Apply<crate::lang::OpWithVar<O>>>(&rec_expr, (rec_expr.len() - 1).into(), weights)
}

/// Total body size including HO-app wrapping: `compute_pattern_size` plus,
/// for each occurrence of `?#k` with `ho_arity[k] > 0`, the cost of the
/// `(@ … (@ ?#k $0) … $(h-1))` wrapper — one `app_cost` + one
/// `sym_var_cost` per binder, per occurrence.
pub fn compute_body_size_with_ho<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, ho_arity: &[u32], weights: &Weights) -> usize {
    let pattern_size = compute_pattern_size::<F, O>(pattern, weights);
    if ho_arity.iter().all(|&h| h == 0) {
        return pattern_size;
    }
    let per_app = weights.app_cost + weights.sym_var_cost;
    let ho_extra: u32 = (0..pattern.vars.len()).map(|k| pattern.vars[k].len() as u32 * ho_arity[k] * per_app).sum();
    pattern_size + ho_extra as usize
}

pub fn compute_recexpr_size<L: StitchLanguage>(rec_expr: &RecExpr<L>, ptr: Id, weights: &Weights) -> usize {
    let node = &rec_expr[ptr];
    node.discriminant().intrinsic_size(weights) as usize + node.children().iter().map(|&child| compute_recexpr_size::<L>(rec_expr, child, weights)).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a work-queue ordered by postorder (children before parents) so each
/// eclass is visited at most once.
pub fn compute_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, search_state: &SearchState<F, O>, check_slow: bool, ho_arity: &[u32]) -> usize {
    let mut eclass_to_matches = FxHashMap::<Id, &Vec<Subst>>::default();
    for m in &search_state.matches {
        eclass_to_matches.insert(m.root_eclass, &m.substs);
    }

    let get_size = |eclass: Id, s_u_r: &FxHashMap<Id, i64>| -> i64 { s_u_r.get(&eclass).cloned().unwrap_or(egraph[eclass].data.size as i64) };

    let arity = search_state.pattern.var_depth.len();

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
                let stub_size = F::stub_application_size::<O>("inv_0", arity, weights) as i64;
                let args_size: i64 = subst
                    .vars
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| {
                        let h = ho_arity[k];
                        let wrap = if h > 0 { F::lams_cost(h, weights) as i64 } else { 0 };
                        wrap + get_size(v, &size_under_rewrite)
                    })
                    .sum();
                let size_new: i64 = stub_size + args_size;
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
        let slow_size = build_rewritten_egraph(egraph, search_state, ho_arity)[root].data.size as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

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

/// Per-subst HO wrapping: for each captured arg `arg_id` at metavar slot `k`,
/// returns either `arg_id` unchanged (when `ho_arity[k] == 0`) or
/// `λ^h. shift_free(arg_id, +h, var_depth[k])` (otherwise). Used by both
/// `build_rewritten_egraph` and `lib::apply_abstraction`; `shift_memo` is
/// shared across calls so equivalent shifts are deduplicated.
pub(crate) fn wrap_subst_args<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, vars: &[Id], ho_arity: &[u32], var_depth: &[u32], shift_memo: &mut FxHashMap<(Id, u32), Id>) -> Vec<Id> {
    vars.iter()
        .enumerate()
        .map(|(k, &arg_id)| {
            let h = ho_arity[k];
            if h == 0 {
                arg_id
            } else {
                let shifted = shift_free_egraph::<F, O>(egraph, arg_id, h, var_depth[k], shift_memo);
                F::wrap_lams::<O>(shifted, h, egraph)
            }
        })
        .collect()
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

/// Computes the exact syntactic free-variable set at every position of `expr`,
/// indexed by `usize::from(Id)`. Shares its per-enode rule with
/// `StitchAnalysis::make` via `enode_fv`.
pub fn recexpr_fv<L: StitchLanguage>(expr: &RecExpr<L>) -> Vec<FxHashSet<u32>> {
    let nodes: &[L] = expr.as_ref();
    let mut fv: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        fv[i] = enode_fv(node, |c| &fv[usize::from(c)]);
    }
    fv
}

/// Asserts that the extracted term's actual syntactic fv matches the egraph
/// analysis's recorded fv. Under intersection-fv semantics + AstSize
/// extraction, the minimal-size representative is also the fv-minimal one,
/// so its fv should equal the intersection across reps — i.e. `expected`.
/// A mismatch in either direction means the assumption "min-size ⇒ min-fv"
/// failed for this extraction; downstream soundness checks that read
/// `data.fv` lose their guarantee.
pub fn check_fvs_are_as_expected<L: StitchLanguage>(expr: &RecExpr<L>, expected: &FxHashSet<u32>) {
    let fv = recexpr_fv(expr);
    let actual = fv.last().expect("non-empty RecExpr");
    assert_eq!(actual, expected, "extracted RecExpr fv {:?} differs from egraph analysis fv {:?}; intersection-fv assumption (min-size rep is fv-minimal) violated", actual, expected,);
}
