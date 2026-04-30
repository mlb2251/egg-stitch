use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::matching::Subst;
use crate::revexpr::abstract_with_hoist;
use crate::search::SearchState;
use egg::{Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Per-metavar hoist sets for a pattern.
///
/// `per_metavar[k]` lists the post-pattern-wrap free indices that need to be
/// hoisted into extra `fn_n` parameters at metavar `?#k`. These are computed
/// as the union, across all matches, of `{i - d_k | i ∈ captured.fv, i ≥ d_k}`.
#[derive(Debug, Clone)]
pub struct HoistSets {
    pub per_metavar: Vec<Vec<u32>>,
}

impl HoistSets {
    /// Sum over all metavars of `(1 + |hoist[k]|)` — the number of args
    /// `fn_n` actually receives at each call site under hoisting.
    pub fn arity_after_hoist(&self) -> usize {
        self.per_metavar.iter().map(|h| 1 + h.len()).sum()
    }
}

/// Compute the per-metavar hoist set as the union of post-pattern-wrap free
/// indices across every subst in the search state. Sorted ascending.
pub fn compute_hoist_sets<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, state: &SearchState<F, O>) -> HoistSets {
    let var_depth = &state.pattern.var_depth;
    let num_meta = var_depth.len();
    let mut acc: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); num_meta];
    for m in &state.matches {
        for subst in &m.substs {
            for (k, &arg_id) in subst.vars.iter().enumerate() {
                let d_k = var_depth[k];
                for &i in &egraph[arg_id].data.fv {
                    if i >= d_k {
                        acc[k].insert(i - d_k);
                    }
                }
            }
        }
    }
    let per_metavar = acc
        .into_iter()
        .map(|s| {
            let mut v: Vec<u32> = s.into_iter().collect();
            v.sort();
            v
        })
        .collect();
    HoistSets { per_metavar }
}

/// Returns true iff every metavar's post-pattern-wrap fv in `subst` matches
/// `hoists.per_metavar[k]` exactly. Substs that disagree (smaller fv set) are
/// rejected: a single `fn_n` signature can't accommodate variable hoist arity
/// across call sites.
pub fn subst_compatible_with<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, var_depth: &[u32], subst: &Subst, hoists: &HoistSets) -> bool {
    for (k, &arg_id) in subst.vars.iter().enumerate() {
        let d_k = var_depth[k];
        let arg_fv = &egraph[arg_id].data.fv;
        let mut post: Vec<u32> = arg_fv.iter().filter_map(|&i| if i >= d_k { Some(i - d_k) } else { None }).collect();
        post.sort();
        if post != hoists.per_metavar[k] {
            return false;
        }
    }
    true
}

/// Build the call-site form of one match-site arg.
///
/// At depth 0 with no hoists the arg is used as-is. Otherwise we extract a
/// smallest representative from `arg_id`'s e-class, run `abstract_with_hoist`
/// to substitute pattern-internal indices and hoist indices to positional
/// refs in `d_k + |hoist|` wrap-lams, and add the result back to the egraph.
///
/// The wrapped subtree gets its own e-class — we don't union it with the
/// original arg, because the original isn't equivalent to the wrapped form
/// (different binder context). The caller hands the wrapped Id off to
/// `add_stub_application` along with literal `Var(h + d_k)` enodes for each
/// hoist value, and β-reduction of `fn_n` at the call site restores equality
/// with the original program.
pub fn wrap_arg_for_abstraction<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, arg_id: Id, depth: u32, hoist: &[u32]) -> Id {
    if depth == 0 && hoist.is_empty() {
        return arg_id;
    }
    let rec: RecExpr<F::Apply<O>> = {
        let extractor = egg::Extractor::new(egraph, egg::AstSize);
        extractor.find_best(arg_id).1
    };
    let root = (rec.as_ref().len() - 1).into();
    let wrapped = abstract_with_hoist(&rec, root, depth, hoist);
    egraph.add_expr(&wrapped)
}

/// Add a literal `$h` De Bruijn leaf to the egraph and return its Id. Used at
/// call sites to supply the hoisted positional value.
pub fn add_db_var<L: StitchLanguage>(egraph: &mut StitchEgraph<L>, h: u32) -> Id {
    let leaf = L::from_op(&format!("${h}"), vec![]).expect("from_op DB var");
    egraph.add(leaf)
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
/// rendered-body size (with each `?#k` expanded into its stitch λ-form
/// application — see `Pattern::body_with_hoists`).
pub fn compute_cost<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, root: egg::Id, cache: &CostCache, search_state: &SearchState<F, O>, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, search_state, check_slow);
    let pattern_size = compute_pattern_size(egraph, search_state, &egraph.analysis.weights);
    cost + pattern_size
}

/// Size of the abstraction body actually emitted as `fn_n`'s definition: the
/// pattern with each `?#k` replaced by its hoist-and-binder application form.
/// This is what gets displayed in the library output, so the cost figure and
/// the printed body always agree on token count.
pub fn compute_pattern_size<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>, weights: &Weights) -> usize {
    let hoists = compute_hoist_sets::<F, O>(egraph, search_state);
    let rec_expr = search_state.pattern.body_with_hoists(&hoists.per_metavar);
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

    // Hoist analysis: per metavar, the union of post-pattern-wrap fv across all
    // matches. Substs that don't match this signature exactly get filtered —
    // a single `fn_n` can't have variable hoist arity across call sites.
    //
    // Per-match overhead: each arg `k` gets wrapped in `d_k + |hoist_k|` lam
    // enodes (the abstract_with_hoist envelope), and the call site additionally
    // passes |hoist_k| literal `$h` enodes as extra args. So the per-match cost
    // beyond `Σ arg_size` is `Σ_k ((d_k + |hoist_k|) * lam_cost + |hoist_k| *
    // sym_var_cost)`, plus `stub_application_size` over the post-hoist arity.
    let var_depth = &search_state.pattern.var_depth;
    let hoists = compute_hoist_sets::<F, O>(egraph, search_state);

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
        let arity_after_hoist = hoists.arity_after_hoist();
        let wrap_overhead: i64 = (0..var_depth.len())
            .map(|k| {
                let n = hoists.per_metavar[k].len() as i64;
                let d = var_depth[k] as i64;
                (d + n) * weights.lam_cost as i64 + n * weights.sym_var_cost as i64
            })
            .sum();
        if let Some(substs) = eclass_to_matches.get(&eclass) {
            for subst in *substs {
                if !subst_compatible_with::<F, O>(egraph, var_depth, subst, &hoists) {
                    continue;
                }
                let stub_size = F::stub_application_size::<O>("inv_0", arity_after_hoist, weights) as i64;
                let size_new: i64 = stub_size + wrap_overhead + subst.vars.iter().map(|&v| get_size(v, &size_under_rewrite)).sum::<i64>();
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

/// Clones the egraph and unions each match root with an `inv_0(wrapped_args...)`
/// node, then rebuilds. Used for validating `compute_size` and for extracting
/// rewritten programs.
///
/// Each arg is wrapped via `wrap_arg_for_abstraction` so the slow path produces
/// the same per-match cost (stub + Σ d_k lams + Σ arg sizes) that
/// `compute_size` computes analytically.
pub(crate) fn build_rewritten_egraph<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> StitchEgraph<F::Apply<O>> {
    let mut egraph = egraph.clone();
    let var_depth = &search_state.pattern.var_depth;
    let hoists = compute_hoist_sets::<F, O>(&egraph, search_state);
    for m in &search_state.matches {
        for subst in &m.substs {
            if !subst_compatible_with::<F, O>(&egraph, var_depth, subst, &hoists) {
                continue;
            }
            let mut all_args: Vec<Id> = Vec::new();
            for (k, &arg_id) in subst.vars.iter().enumerate() {
                let d_k = var_depth[k];
                let hoist_k = &hoists.per_metavar[k];
                all_args.push(wrap_arg_for_abstraction::<F, O>(&mut egraph, arg_id, d_k, hoist_k));
                for &h_post in hoist_k {
                    all_args.push(add_db_var::<F::Apply<O>>(&mut egraph, h_post + d_k));
                }
            }
            let x = F::add_stub_application::<O>("inv_0", all_args, &mut egraph);
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
