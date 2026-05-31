//! Unit tests for `io::rule_fv_condition_violation`, the load-time check for the
//! structural conditions behind `fv(c) = fv(MinTerm(c))`. The helper returns
//! `Some(reason)` for a non-conforming rule and `None` for a conforming one;
//! `io::parse` panics on `Some`.

use egg::{ENodeOrVar, RecExpr};
use egg_stitch::io::rule_fv_condition_violation;
use egg_stitch::lang::{LambdaCalcLanguage, Op, OpChildrenLanguage, OpDB, StitchLanguage};

type Flat = OpChildrenLanguage<Op>;
type Lam = LambdaCalcLanguage<OpDB<Op>>;

fn flat(s: &str) -> RecExpr<ENodeOrVar<Flat>> {
    <Flat as StitchLanguage>::parse_pattern_ast(s).expect("parse flat pattern")
}
fn lam(s: &str) -> RecExpr<ENodeOrVar<Lam>> {
    <Lam as StitchLanguage>::parse_pattern_ast(s).expect("parse lambda pattern")
}

#[test]
fn shared_variables_ok() {
    // Same metavariable set on both sides: conforming regardless of shape.
    assert_eq!(rule_fv_condition_violation::<Flat>(&flat("(+ ?x ?y)"), &flat("(+ ?y ?x)")), None);
    assert_eq!(rule_fv_condition_violation::<Flat>(&flat("(f ?x)"), &flat("?x")), None);
}

#[test]
fn dropping_a_variable_to_a_strictly_smaller_side_ok() {
    // `(* 0 ?x) => 0`: ?x only on the LHS, which is strictly larger. Conforming.
    assert_eq!(rule_fv_condition_violation::<Flat>(&flat("(* 0 ?x)"), &flat("0")), None);
}

#[test]
fn one_sided_variable_on_non_larger_side_flagged() {
    // `id_xf`-style tie: ?x only on the LHS, but both sides have equal node count.
    let v = rule_fv_condition_violation::<Flat>(&flat("(T ?x z)"), &flat("(T c z)"));
    assert!(v.is_some(), "expected a violation");
    assert!(v.unwrap().contains("only on the LHS"), "wrong reason");

    // Symmetric: variable introduced only on the (non-larger) RHS.
    let v = rule_fv_condition_violation::<Flat>(&flat("(T c z)"), &flat("(T ?x z)"));
    assert!(v.unwrap().contains("only on the RHS"));
}

#[test]
fn free_de_bruijn_leaf_flagged() {
    // `$0` is free at the top level of the RHS.
    let v = rule_fv_condition_violation::<Lam>(&lam("(f ?x)"), &lam("(g $0 ?x)"));
    assert!(v.unwrap().contains("free de Bruijn"));
}

#[test]
fn bound_de_bruijn_leaf_ok() {
    // `$0` under a binder is bound, contributes nothing to fv — conforming.
    assert_eq!(rule_fv_condition_violation::<Lam>(&lam("(map (lam $0) ?xs)"), &lam("?xs")), None);
}

#[test]
fn metavariable_under_binder_flagged() {
    // ?x sits beneath the `lam` binder on the LHS.
    let v = rule_fv_condition_violation::<Lam>(&lam("(lam ?x)"), &lam("(f ?x)"));
    assert!(v.unwrap().contains("beneath a binder"));
}
