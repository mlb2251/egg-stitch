use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use std::collections::HashMap;

/// Strips the `ENodeOrVar` wrapper from a parsed follow pattern, panicking if
/// any node is a `Var`. Call this once at parse time so all downstream code
/// can work with `RevExpr<StitchLang>` and never worry about Vars.
pub fn strip_vars(expr: RevExpr<ENodeOrVar<StitchLang>>) -> RevExpr<StitchLang> {
    RevExpr::new(expr.nodes.into_iter().map(|node| match node {
        ENodeOrVar::ENode(inner) => inner,
        ENodeOrVar::Var(v) => panic!("follow pattern contains Var node {v}; only concrete enodes are supported"),
    }).collect())
}

/// Checks whether a pattern is a valid prefix of a concrete follow target.
/// Pattern variables bind to follow subtree ids; every occurrence of the same
/// pattern variable must bind to the same id (which is sufficient for equality
/// because RevExpr preserves egg's hash-consing — structurally equal subtrees
/// always share the same Id).
pub fn check_follow(pattern: &RevExpr<ENodeOrVar<StitchLang>>, pid: Id, follow: &RevExpr<StitchLang>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    match &pattern[pid] {
        ENodeOrVar::Var(v) => match var_bindings.entry(*v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => *e.get() == fid,
        },
        ENodeOrVar::ENode(p_node) => {
            let f_node = &follow[fid];
            p_node.matches(f_node) && p_node.children.iter().zip(f_node.children.iter()).all(|(&pc, &fc)| check_follow(pattern, pc, follow, fc, var_bindings))
        }
    }
}
