//! Tests for `follow::follow_unify`: the prefix/unification check that drives
//! the `--follow` constraint. Patterns and follow trees are built via the
//! `RevExpr` parser (egg's `RecExpr` parser routes `?…` atoms through
//! `OpWithVar::from_name`, which detects them as Vars).

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

#[test]
fn single_var_matches_any_subtree() {
    let pat = parse("?#0");
    let fol = parse("(f a b)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("single var should unify");
    assert_eq!(bindings.len(), 1);
    // The whole follow is bound: ?#0 → follow root (id 0 in RevExpr).
    assert_eq!(bindings[&v(0)], Id::from(0));
}

#[test]
fn exact_structural_match_with_no_vars() {
    let pat = parse("(f a)");
    let fol = parse("(f a)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("identical trees should unify");
    assert!(bindings.is_empty());
}

#[test]
fn head_op_mismatch_fails() {
    let pat = parse("(f a)");
    let fol = parse("(g a)");
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn pattern_node_against_follow_var_fails() {
    // The follow is more abstract than the pattern.
    let pat = parse("(f a)");
    let fol = parse("?#0");
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn pattern_var_at_inner_position_binds_subtree() {
    let pat = parse("(f ?#0)");
    let fol = parse("(f (g a))");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("inner var should unify");
    // Round-trip the captured id back to a string to avoid relying on the
    // RevExpr's internal indexing.
    let captured = &fol.nodes[usize::from(bindings[&v(0)])];
    assert_eq!(captured.to_string(), "g");
    assert_eq!(captured.children.len(), 1);
}

#[test]
fn repeated_pattern_var_with_equal_subtrees_succeeds() {
    // Both occurrences of `?#0` see structurally-equal `a` subtrees — distinct
    // RevExpr ids, collapsed by `follow_subtrees_equal`.
    let pat = parse("(f ?#0 ?#0)");
    let fol = parse("(f a a)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("consistent repeat should unify");
    assert_eq!(bindings.len(), 1);
}

#[test]
fn repeated_pattern_var_with_unequal_subtrees_fails() {
    let pat = parse("(f ?#0 ?#0)");
    let fol = parse("(f a b)");
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn distinct_pattern_vars_bind_independently() {
    let pat = parse("(f ?#0 ?#1)");
    let fol = parse("(f a b)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("distinct vars should unify");
    assert_eq!(bindings.len(), 2);
    assert_eq!(fol.nodes[usize::from(bindings[&v(0)])].to_string(), "a");
    assert_eq!(fol.nodes[usize::from(bindings[&v(1)])].to_string(), "b");
}

#[test]
fn distinct_pattern_vars_can_bind_equal_subtrees() {
    // `follow_unify` is a one-way prefix check, not injective on Vars.
    let pat = parse("(f ?#0 ?#1)");
    let fol = parse("(f a a)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("distinct vars on equal subtrees should unify");
    assert_eq!(bindings.len(), 2);
}

#[test]
fn arity_mismatch_fails() {
    // Same head op, different arity — `OpChildrenLanguage::matches` checks
    // both, so the prefix-style child zip never gets a chance to swallow the
    // dropped trailing argument.
    let pat = parse("(f a)");
    let fol = parse("(f a b)");
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn nested_var_binding() {
    // ?#0 captures a multi-node subtree, not just a leaf.
    let pat = parse("(f ?#0 c)");
    let fol = parse("(f (g a b) c)");
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("multi-node capture should unify");
    let captured = &fol.nodes[usize::from(bindings[&v(0)])];
    assert_eq!(captured.to_string(), "g");
    assert_eq!(captured.children.len(), 2);
}
