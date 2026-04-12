use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use std::collections::HashMap;

/// Checks structural equality of two enode subtrees in a follow RevExpr.
/// Follow patterns in this codebase are fully concrete (no `Var` nodes — `#0`
/// etc. parse as enodes), so only the `(ENode, ENode)` case is reachable.
fn follow_subtrees_equal(follow: &RevExpr<ENodeOrVar<StitchLang>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    let (ENodeOrVar::ENode(na), ENodeOrVar::ENode(nb)) = (&follow[a], &follow[b]) else {
        unreachable!("follow patterns should not contain Var nodes")
    };
    na.matches(nb) && na.children.iter().zip(nb.children.iter()).all(|(&ca, &cb)| follow_subtrees_equal(follow, ca, cb))
}

/// Recursively checks whether a pattern is a valid prefix of the follow target.
/// Pattern variables bind to subtrees of the follow; every occurrence of the
/// same pattern variable must bind to structurally equal follow subtrees.
/// The follow itself must be fully concrete (no `Var` nodes).
pub fn check_follow(pattern: &RevExpr<ENodeOrVar<StitchLang>>, pid: Id, follow: &RevExpr<ENodeOrVar<StitchLang>>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    match &pattern[pid] {
        ENodeOrVar::Var(v) => match var_bindings.entry(*v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal(follow, *e.get(), fid),
        },
        ENodeOrVar::ENode(p_node) => {
            let ENodeOrVar::ENode(f_node) = &follow[fid] else {
                unreachable!("follow patterns should not contain Var nodes")
            };
            p_node.matches(f_node) && p_node.children.iter().zip(f_node.children.iter()).all(|(&pc, &fc)| check_follow(pattern, pc, follow, fc, var_bindings))
        }
    }
}
