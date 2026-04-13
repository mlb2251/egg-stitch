use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use std::collections::{HashMap, hash_map::Entry};

/// Checks structural equality of two subtrees in a follow RevExpr.
fn follow_subtrees_equal(follow: &RevExpr<ENodeOrVar<StitchLang>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    match (&follow[a], &follow[b]) {
        (ENodeOrVar::Var(va), ENodeOrVar::Var(vb)) => va == vb,
        (ENodeOrVar::ENode(na), ENodeOrVar::ENode(nb)) => na.matches(nb) && na.children.iter().zip(nb.children.iter()).all(|(&ca, &cb)| follow_subtrees_equal(follow, ca, cb)),
        _ => false,
    }
}

/// Recursively checks whether a pattern is a valid prefix of the follow target.
pub fn check_follow(pattern: &RevExpr<ENodeOrVar<StitchLang>>, pid: Id, follow: &RevExpr<ENodeOrVar<StitchLang>>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    match &pattern[pid] {
        ENodeOrVar::Var(v) => match var_bindings.entry(*v) {
            Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            Entry::Occupied(e) => follow_subtrees_equal(follow, *e.get(), fid),
        },
        ENodeOrVar::ENode(p_node) => match &follow[fid] {
            ENodeOrVar::Var(_) => false,
            ENodeOrVar::ENode(f_node) => p_node.matches(f_node) && p_node.children.iter().zip(f_node.children.iter()).all(|(&pc, &fc)| check_follow(pattern, pc, follow, fc, var_bindings)),
        },
    }
}
