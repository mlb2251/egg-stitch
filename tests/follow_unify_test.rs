//! Tests for `follow::follow_unify`: the prefix/unification check that drives
//! the `--follow` constraint. Patterns and follow trees are built via the
//! `RevExpr` parser (egg's `RecExpr` parser routes `?…` atoms through
//! `OpWithVar::from_name`, which detects them as Vars).
//!
//! `bindings` flattens the returned `HashMap` to a `Vec<(Var, Id)>` sorted by
//! the var name so assertions compare against an explicit ordered list.

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

/// Run `follow_unify` and flatten the result to a `Vec<(Var, Id)>` sorted by
/// the var's display name, so tests can `assert_eq!` against an ordered list.
fn bindings(pat: &str, fol: &str) -> Option<Vec<(egg::Var, Id)>> {
    let pat = parse(pat);
    let fol = parse(fol);
    follow_unify::<OpChildren, Op>(&pat, &fol).map(|m| {
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by_key(|(k, _)| k.to_string());
        v
    })
}

#[test]
fn single_var_matches_any_subtree() {
    // `?#0` captures the whole follow — root is RevExpr id 0.
    assert_eq!(bindings("?#0", "(f a b)"), Some(vec![(v(0), Id::from(0))]));
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
    // Fol `(f (g a))` reverses to [f([1]), g([2]), a] — `?#0` captures the `g`
    // subtree at id 1.
    assert_eq!(bindings("(f ?#0)", "(f (g a))"), Some(vec![(v(0), Id::from(1))]));
}

#[test]
fn repeated_pattern_var_with_equal_subtrees_succeeds() {
    // Fol `(f a a)` reverses to [f([2,1]), a, a]. The pattern's two `?#0`
    // children land on fids 2 then 1; the second visit hits the
    // `follow_subtrees_equal` branch and accepts because both `a` leaves are
    // structurally equal. The recorded id is the first one bound (fid 2).
    assert_eq!(bindings("(f ?#0 ?#0)", "(f a a)"), Some(vec![(v(0), Id::from(2))]));
}

#[test]
fn repeated_pattern_var_with_unequal_subtrees_fails() {
    assert_eq!(bindings("(f ?#0 ?#0)", "(f a b)"), None);
}

#[test]
fn distinct_pattern_vars_bind_independently() {
    // Fol `(f a b)` reverses to [f([2,1]), b, a]. `?#0` lands at fid 2 (a),
    // `?#1` lands at fid 1 (b).
    assert_eq!(bindings("(f ?#0 ?#1)", "(f a b)"), Some(vec![(v(0), Id::from(2)), (v(1), Id::from(1))]));
}

#[test]
fn distinct_pattern_vars_can_bind_equal_subtrees() {
    // `follow_unify` is a one-way prefix check, not injective on vars — both
    // `?#0` and `?#1` are allowed to capture (different ids of) the same `a`.
    assert_eq!(bindings("(f ?#0 ?#1)", "(f a a)"), Some(vec![(v(0), Id::from(2)), (v(1), Id::from(1))]));
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
    // `?#0` captures a multi-node subtree — fol `(f (g a b) c)` reverses to
    // [f([2,1]), c, g([4,3]), b, a]; the `g` subtree sits at id 2.
    assert_eq!(bindings("(f ?#0 c)", "(f (g a b) c)"), Some(vec![(v(0), Id::from(2))]));
}
