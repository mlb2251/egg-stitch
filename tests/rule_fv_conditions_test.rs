//! Unit tests for `io::rule_fv_verdict`, the load-time classifier for the
//! structural conditions behind `fv(c) = fv(MinTerm(c))`. It returns
//! `Ok`, `OkIfDeBruijnMoreExpensive` (safe only when de Bruijn variables are strictly
//! more expensive than symbols), or `Violation`; `io::parse` panics on a `Violation` and
//! on an unsatisfied `OkIfDeBruijnMoreExpensive`.

use egg::{ENodeOrVar, RecExpr};
use egg_stitch::io::{RuleFvVerdict, rule_fv_verdict};
use egg_stitch::lang::{LambdaCalcLanguage, Op, OpChildrenLanguage, OpDB, StitchLanguage, Weights};

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
    assert_eq!(rule_fv_verdict::<Flat>(&flat("(+ ?x ?y)"), &flat("(+ ?y ?x)")), RuleFvVerdict::Ok);
    assert_eq!(rule_fv_verdict::<Flat>(&flat("(f ?x)"), &flat("?x")), RuleFvVerdict::Ok);
}

#[test]
fn dropping_a_variable_to_a_strictly_smaller_side_ok() {
    // `(* 0 ?x) => 0`: ?x only on the LHS, which is strictly larger. Conforming.
    assert_eq!(rule_fv_verdict::<Flat>(&flat("(* 0 ?x)"), &flat("0")), RuleFvVerdict::Ok);
}

#[test]
fn variable_introduced_on_strictly_smaller_side_is_a_violation() {
    // ?x only on the LHS, and the LHS is strictly smaller — reducing toward it
    // introduces the variable into the reduced term.
    assert!(matches!(rule_fv_verdict::<Flat>(&flat("?x"), &flat("(f a a)")), RuleFvVerdict::Violation(_)));
}

#[test]
fn one_sided_variable_at_a_size_tie_is_conditional() {
    // `id_xf`-style tie: ?x only on the LHS, equal node count on both sides.
    let v = rule_fv_verdict::<Flat>(&flat("(T ?x z)"), &flat("(T c z)"));
    assert!(matches!(v, RuleFvVerdict::OkIfDeBruijnMoreExpensive(_)), "expected OkIfDeBruijnMoreExpensive, got {v:?}");
    // Symmetric on the RHS.
    let v = rule_fv_verdict::<Flat>(&flat("(T c z)"), &flat("(T ?x z)"));
    assert!(matches!(v, RuleFvVerdict::OkIfDeBruijnMoreExpensive(_)));
}

#[test]
fn free_de_bruijn_leaf_is_a_violation() {
    let v = rule_fv_verdict::<Lam>(&lam("(f ?x)"), &lam("(g $0 ?x)"));
    assert!(matches!(v, RuleFvVerdict::Violation(ref r) if r.contains("free de Bruijn")), "got {v:?}");
}

#[test]
fn bound_de_bruijn_leaf_ok() {
    // `$0` under a binder is bound, contributes nothing to fv — conforming.
    assert_eq!(rule_fv_verdict::<Lam>(&lam("(map (lam $0) ?xs)"), &lam("?xs")), RuleFvVerdict::Ok);
}

#[test]
fn metavariable_under_binder_is_a_violation() {
    let v = rule_fv_verdict::<Lam>(&lam("(lam ?x)"), &lam("(f ?x)"));
    assert!(matches!(v, RuleFvVerdict::Violation(ref r) if r.contains("beneath a binder")), "got {v:?}");
}

#[test]
fn language_level_de_bruijn_cost_check() {
    // Under the uniform leaf-cost model, every leaf costs `sym_var_cost`, so
    // de Bruijn variables are *not* strictly more expensive than symbols in a language
    // that has them (lambda-calc) — but the check holds vacuously for a language
    // without de Bruijn variables (op-children). This is what lets the `id_xf`
    // tie load for the (de-Bruijn-free) drawing domains but be rejected for a
    // de-Bruijn language under uniform weights.
    let w = Weights::default();
    assert!(<Flat as StitchLanguage>::de_bruijn_strictly_more_expensive_than_symbols(&w));
    assert!(!<Lam as StitchLanguage>::de_bruijn_strictly_more_expensive_than_symbols(&w));
}
