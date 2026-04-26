use crate::lang::{LanguageFamily, OpWithVar, StitchOp};
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use std::collections::HashMap;

/// Structural equality of two subtrees in the follow tree. Needed because
/// RecExpr doesn't hash-cons — repeated `?#0` nodes get distinct Ids.
fn follow_subtrees_equal<F: LanguageFamily, O: StitchOp>(follow: &RevExpr<F::Apply<OpWithVar<O>>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    match (follow[a].discriminant(), follow[b].discriminant()) {
        (OpWithVar::Var(va), OpWithVar::Var(vb)) => va == vb,
        (OpWithVar::Node(_), OpWithVar::Node(_)) => follow[a].matches(&follow[b]) && follow[a].children().iter().zip(follow[b].children().iter()).all(|(&ca, &cb)| follow_subtrees_equal::<F, O>(follow, ca, cb)),
        _ => false,
    }
}

/// Checks whether a pattern is a valid prefix of a follow target.
/// Pattern ENode at a follow-Var position is rejected.
pub fn check_follow<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, pid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    match (pattern[pid].discriminant(), follow[fid].discriminant()) {
        (OpWithVar::Var(v), _) => match var_bindings.entry(v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal::<F, O>(follow, *e.get(), fid),
        },
        (OpWithVar::Node(_), OpWithVar::Var(_)) => false,
        (OpWithVar::Node(_), OpWithVar::Node(_)) => pattern[pid].matches(&follow[fid]) && pattern[pid].children().iter().zip(follow[fid].children().iter()).all(|(&pc, &fc)| check_follow::<F, O>(pattern, pc, follow, fc, var_bindings)),
    }
}
