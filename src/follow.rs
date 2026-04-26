use crate::pattern::PatternStorage;
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use std::collections::HashMap;

/// Structural equality of two subtrees in the follow tree. Needed because
/// RecExpr doesn't hash-cons — repeated `?#0` nodes get distinct Ids.
fn follow_subtrees_equal<L: Language, P: PatternStorage<L>>(follow: &RevExpr<P>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    let (na, nb) = (&follow[a], &follow[b]);
    na.matches(nb) && na.children().iter().zip(nb.children().iter()).all(|(&ca, &cb)| follow_subtrees_equal::<L, P>(follow, ca, cb))
}

/// Checks whether a pattern is a valid prefix of a follow target.
/// Pattern Var matches any subtree (binding the var); pattern Node at a follow-Var
/// position is rejected.
pub fn check_follow<L: Language, P: PatternStorage<L>>(pattern: &RevExpr<P>, pid: Id, follow: &RevExpr<P>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    if let Some(v) = pattern[pid].as_var() {
        return match var_bindings.entry(v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal::<L, P>(follow, *e.get(), fid),
        };
    }
    if follow[fid].as_var().is_some() {
        return false;
    }
    let (pn, fnode) = (&pattern[pid], &follow[fid]);
    pn.matches(fnode) && pn.children().iter().zip(fnode.children().iter()).all(|(&pc, &fc)| check_follow::<L, P>(pattern, pc, follow, fc, var_bindings))
}
