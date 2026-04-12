use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use std::collections::HashMap;

/// Validates a parsed follow pattern: panics if any enode has an op starting
/// with `#`, which likely means the user wrote `#0` instead of `?0`.
pub fn validate_follow(expr: &RevExpr<ENodeOrVar<StitchLang>>) {
    for node in &expr.nodes {
        if let ENodeOrVar::ENode(n) = node {
            assert!(
                !n.op.as_str().starts_with('#'),
                "follow pattern contains enode with op '{}'; use ?-prefixed variables (e.g. ?0) instead of #-prefixed literals",
                n.op,
            );
        }
    }
}

/// Checks whether a pattern is a valid prefix of a follow target.
///
/// Pattern variables bind to follow subtree ids; every occurrence of the same
/// pattern variable must bind to the same id (sufficient for structural
/// equality because RevExpr preserves egg's hash-consing).
///
/// Follow variables act as wildcards: any pattern node at a follow-Var
/// position is accepted. This lets the search explore freely at those
/// positions — particles that expand "wrong" there naturally have higher
/// cost and get outcompeted during resampling, rather than being killed
/// immediately (which collapses diversity too fast for the simple SMC loop).
pub fn check_follow(pattern: &RevExpr<ENodeOrVar<StitchLang>>, pid: Id, follow: &RevExpr<ENodeOrVar<StitchLang>>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    match (&pattern[pid], &follow[fid]) {
        (ENodeOrVar::Var(v), _) => match var_bindings.entry(*v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => *e.get() == fid,
        },
        (_, ENodeOrVar::Var(_)) => true,
        (ENodeOrVar::ENode(p_node), ENodeOrVar::ENode(f_node)) => {
            p_node.matches(f_node) && p_node.children.iter().zip(f_node.children.iter()).all(|(&pc, &fc)| check_follow(pattern, pc, follow, fc, var_bindings))
        }
    }
}
