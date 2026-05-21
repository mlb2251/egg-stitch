//! Tests for `follow::follow_unify`: the prefix/unification check that drives
//! the `--follow` constraint. Uses `OpChildren` over `OpWithVar<Op>` so trees
//! can be built by hand without going through any parser.

use egg::Id;
use egg_stitch::follow::follow_unify;
use egg_stitch::lang::{Op, OpChildren, OpChildrenLanguage, OpWithVar, StitchOp};
use egg_stitch::revexpr::RevExpr;

type Node = OpChildrenLanguage<OpWithVar<Op>>;

fn node(name: &str, children: Vec<usize>) -> Node {
    OpChildrenLanguage {
        op: OpWithVar::Node(Op::from_name(name)),
        children: children.into_iter().map(Id::from).collect(),
    }
}

fn var(k: u32) -> Node {
    OpChildrenLanguage {
        op: OpWithVar::Var(egg::Var::from(k)),
        children: vec![],
    }
}

fn rev(nodes: Vec<Node>) -> RevExpr<Node> {
    RevExpr::new(nodes)
}

#[test]
fn single_var_matches_any_subtree() {
    // pattern: `?#0`
    // follow:  `(f a b)`
    // Expect: binding ?#0 → root of follow (id 0).
    let pat = rev(vec![var(0)]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("b", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("single var should unify");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[&egg::Var::from(0u32)], Id::from(0));
}

#[test]
fn exact_structural_match_with_no_vars() {
    // pattern == follow == `(f a)`
    let pat = rev(vec![node("f", vec![1]), node("a", vec![])]);
    let fol = rev(vec![node("f", vec![1]), node("a", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("identical trees should unify");
    assert!(bindings.is_empty());
}

#[test]
fn head_op_mismatch_fails() {
    // pattern: `(f a)`  vs  follow: `(g a)`
    let pat = rev(vec![node("f", vec![1]), node("a", vec![])]);
    let fol = rev(vec![node("g", vec![1]), node("a", vec![])]);
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn pattern_node_against_follow_var_fails() {
    // pattern: `(f a)`  vs  follow: `?#0`. A concrete pattern node can't sit
    // at a follow-Var position — the follow is more abstract than the pattern.
    let pat = rev(vec![node("f", vec![1]), node("a", vec![])]);
    let fol = rev(vec![var(0)]);
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn pattern_var_at_inner_position_binds_subtree() {
    // pattern: `(f ?#0)`  follow: `(f (g a))`
    // Expect: ?#0 → the inner `(g a)` subtree (follow id 1).
    let pat = rev(vec![node("f", vec![1]), var(0)]);
    let fol = rev(vec![node("f", vec![1]), node("g", vec![2]), node("a", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("inner var should unify");
    assert_eq!(bindings[&egg::Var::from(0u32)], Id::from(1));
}

#[test]
fn repeated_pattern_var_with_equal_subtrees_succeeds() {
    // pattern: `(f ?#0 ?#0)`  follow: `(f a a)`
    // Both `?#0` occurrences see structurally-equal follow subtrees (two
    // distinct `a` ids — `follow_subtrees_equal` collapses them).
    let pat = rev(vec![node("f", vec![1, 1]), var(0)]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("a", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("consistent repeat should unify");
    assert_eq!(bindings.len(), 1);
}

#[test]
fn repeated_pattern_var_with_unequal_subtrees_fails() {
    // pattern: `(f ?#0 ?#0)`  follow: `(f a b)` — ?#0 can't be both `a` and `b`.
    let pat = rev(vec![node("f", vec![1, 1]), var(0)]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("b", vec![])]);
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}

#[test]
fn distinct_pattern_vars_bind_independently() {
    // pattern: `(f ?#0 ?#1)`  follow: `(f a b)`
    let pat = rev(vec![node("f", vec![1, 2]), var(0), var(1)]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("b", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("distinct vars should unify");
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[&egg::Var::from(0u32)], Id::from(1));
    assert_eq!(bindings[&egg::Var::from(1u32)], Id::from(2));
}

#[test]
fn distinct_pattern_vars_can_bind_equal_subtrees() {
    // pattern: `(f ?#0 ?#1)`  follow: `(f a a)`
    // Distinct pattern vars aren't required to bind distinct follow subtrees —
    // `follow_unify` is a one-way prefix check, not injective.
    let pat = rev(vec![node("f", vec![1, 2]), var(0), var(1)]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("a", vec![])]);
    let bindings = follow_unify::<OpChildren, Op>(&pat, &fol).expect("distinct vars on equal subtrees should unify");
    assert_eq!(bindings.len(), 2);
}

#[test]
fn arity_mismatch_fails() {
    // pattern: `(f a)`  follow: `(f a b)`. Same op, different arity.
    let pat = rev(vec![node("f", vec![1]), node("a", vec![])]);
    let fol = rev(vec![node("f", vec![1, 2]), node("a", vec![]), node("b", vec![])]);
    assert!(follow_unify::<OpChildren, Op>(&pat, &fol).is_none());
}
