use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use std::collections::HashMap;

/// Structural equality of two subtrees in the follow tree. Needed because
/// RecExpr doesn't hash-cons — repeated `?#0` nodes get distinct Ids.
fn follow_subtrees_equal<F: LanguageFamily, O: StitchOp>(follow: &RevExpr<F::Apply<OpWithVar<O>>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    let (na, nb) = (&follow[a], &follow[b]);
    match (na.discriminant().as_var(), nb.discriminant().as_var()) {
        (Some(va), Some(vb)) => va == vb,
        (None, None) => na.matches(nb) && na.children().iter().zip(nb.children().iter()).all(|(&ca, &cb)| follow_subtrees_equal::<F, O>(follow, ca, cb)),
        _ => false,
    }
}

/// Unifies a pattern against a follow target, returning the substitution that
/// makes the pattern a structural prefix of the follow — pattern Vars map to
/// whatever follow subtree they cover; everything else must match exactly.
/// `None` means the pattern is not a prefix of the follow.
pub fn follow_unify<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, follow: &RevExpr<F::Apply<OpWithVar<O>>>) -> Option<HashMap<egg::Var, Id>> {
    let mut bindings = HashMap::new();
    walk::<F, O>(pattern, Id::from(0), follow, Id::from(0), &mut bindings).then_some(bindings)
}

fn walk<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, pid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, bindings: &mut HashMap<egg::Var, Id>) -> bool {
    let (pn, fn_) = (&pattern[pid], &follow[fid]);
    if let Some(v) = pn.discriminant().as_var() {
        return match bindings.entry(v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal::<F, O>(follow, *e.get(), fid),
        };
    }
    if fn_.discriminant().as_var().is_some() {
        return false;
    }
    pn.matches(fn_) && pn.children().iter().zip(fn_.children().iter()).all(|(&pc, &fc)| walk::<F, O>(pattern, pc, follow, fc, bindings))
}

/// True iff the state's HO-arity-decorated body is alpha-equivalent to the
/// follow target. Equivalent to: η-wrap the pattern with `variable_indices` so
/// it has the same surface shape the search displays, then `follow_unify`
/// against the follow; alpha-equivalence is the case where every captured
/// binding is a bare follow Var and the pattern→follow Var mapping is
/// injective.
pub fn matches_follow_serialized<F: LanguageFamily, O: StitchOp>(state: &crate::search::SearchState<F, O>, follow: &RevExpr<F::Apply<OpWithVar<O>>>, egraph: &crate::lang::StitchEgraph<F::Apply<O>>) -> bool {
    let vis = crate::cost::compute_variable_indices::<F, O>(egraph, state);
    let wrapped: RevExpr<F::Apply<OpWithVar<O>>> = state.pattern.build_with_ho(&vis).into();
    let Some(bindings) = follow_unify::<F, O>(&wrapped, follow) else { return false };
    let mut seen = std::collections::HashSet::new();
    bindings.values().all(|&fid| follow[fid].discriminant().as_var().is_some_and(|v| seen.insert(v)))
}
