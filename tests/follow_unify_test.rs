//! Tests for `follow::follow_unify`: the prefix/unification check that drives
//! the `--follow` constraint. Patterns and follow trees are built via the
//! `RevExpr` parser (egg's `RecExpr` parser routes `?…` atoms through
//! `OpWithVar::from_name`, which detects them as Vars).
//!
//! `bindings` flattens the returned `HashMap` to a `Vec<(Var, String)>` sorted
//! by var name, with each captured subtree rendered back to its surface form
//! so assertions read as `(?#0, "a")` rather than relying on RevExpr's
//! internal id layout.

use egg::Id;
use egg_stitch::follow::follow_unify;
use egg_stitch::lang::{Op, OpChildren, OpChildrenLanguage, OpWithVar};
use egg_stitch::revexpr::RevExpr;

type Tree = RevExpr<OpChildrenLanguage<OpWithVar<Op>>>;

fn parse(s: &str) -> Tree {
    s.parse().unwrap_or_else(|e| panic!("parse {s:?}: {e:?}"))
}

fn v(k: u32) -> egg::Var {
    egg::Var::from(k)
}

/// Render the subtree rooted at `id` in `tree` as a parenthesised sexp,
/// matching the surface form the parser consumed.
fn render(tree: &Tree, id: Id) -> String {
    let n = &tree.nodes[usize::from(id)];
    if n.children.is_empty() {
        n.op.to_string()
    } else {
        let kids: Vec<String> = n.children.iter().map(|&c| render(tree, c)).collect();
        format!("({} {})", n.op, kids.join(" "))
    }
}

/// Run `follow_unify` and flatten the result to a `Vec<(Var, String)>` sorted
/// by the var's display name, with each captured id replaced by the rendered
/// subtree it points to.
fn bindings(pat: &str, fol: &str) -> Option<Vec<(egg::Var, String)>> {
    let pat = parse(pat);
    let fol = parse(fol);
    follow_unify::<OpChildren, Op>(&pat, &fol).map(|m| {
        let mut v: Vec<_> = m.into_iter().map(|(k, id)| (k, render(&fol, id))).collect();
        v.sort_by_key(|(k, _)| k.to_string());
        v
    })
}

#[test]
fn single_var_matches_any_subtree() {
    assert_eq!(bindings("?#0", "(f a b)"), Some(vec![(v(0), "(f a b)".into())]));
}

#[test]
fn exact_structural_match_with_no_vars() {
    assert_eq!(bindings("(f a)", "(f a)"), Some(vec![]));
}

#[test]
fn head_op_mismatch_fails() {
    assert_eq!(bindings("(f a)", "(g a)"), None);
}

#[test]
fn pattern_node_against_follow_var_fails() {
    // Pattern is more concrete than the follow — fails.
    assert_eq!(bindings("(f a)", "?#0"), None);
}

#[test]
fn pattern_var_at_inner_position_binds_subtree() {
    assert_eq!(bindings("(f ?#0)", "(f (g a))"), Some(vec![(v(0), "(g a)".into())]));
}

#[test]
fn repeated_pattern_var_with_equal_subtrees_succeeds() {
    // Both `?#0` occurrences see structurally-equal `a` leaves — distinct
    // RevExpr ids, collapsed by `follow_subtrees_equal`. The recorded id is
    // the first one bound, which renders as just `a`.
    assert_eq!(bindings("(f ?#0 ?#0)", "(f a a)"), Some(vec![(v(0), "a".into())]));
}

#[test]
fn repeated_pattern_var_with_unequal_subtrees_fails() {
    assert_eq!(bindings("(f ?#0 ?#0)", "(f a b)"), None);
}

#[test]
fn distinct_pattern_vars_bind_independently() {
    assert_eq!(bindings("(f ?#0 ?#1)", "(f a b)"), Some(vec![(v(0), "a".into()), (v(1), "b".into())]));
}

#[test]
fn distinct_pattern_vars_can_bind_equal_subtrees() {
    // `follow_unify` is a one-way prefix check, not injective on vars — both
    // `?#0` and `?#1` may capture (different ids of) the same `a`.
    assert_eq!(bindings("(f ?#0 ?#1)", "(f a a)"), Some(vec![(v(0), "a".into()), (v(1), "a".into())]));
}

#[test]
fn arity_mismatch_fails() {
    // Same head op, different arity — `OpChildrenLanguage::matches` checks
    // both, so the prefix-style child zip doesn't get to swallow the trailing
    // argument.
    assert_eq!(bindings("(f a)", "(f a b)"), None);
}

#[test]
fn nested_var_binding() {
    // `?#0` captures a multi-node subtree.
    assert_eq!(bindings("(f ?#0 c)", "(f (g a b) c)"), Some(vec![(v(0), "(g a b)".into())]));
}
