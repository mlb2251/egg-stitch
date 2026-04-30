//! Tests for `cost::wrap_arg_for_abstraction` — the extract→abstract+wrap step
//! that builds a closed `(λ … λ body)` envelope which, when called via the
//! stitch λ-form `(?#k $(d-1) … $0)` plus any hoist args, β-reduces back to
//! the original captured subterm at the call site.

use egg::Id;
use egg_stitch::cost::wrap_arg_for_abstraction;
use egg_stitch::lang::{LambdaCalc, LambdaCalcLanguage, Op, OpDB, StitchAnalysis, StitchLanguage};

type LamLang = LambdaCalcLanguage<OpDB<Op>>;

fn fresh_egraph() -> egg::EGraph<LamLang, StitchAnalysis> {
    egg::EGraph::default()
}

fn add(egraph: &mut egg::EGraph<LamLang, StitchAnalysis>, prog: &str) -> Id {
    let expr = LamLang::parse_program(prog).unwrap();
    egraph.add_expr(&expr)
}

fn extract(egraph: &egg::EGraph<LamLang, StitchAnalysis>, id: Id) -> String {
    let extractor = egg::Extractor::new(egraph, egg::AstSize);
    let (_, rec) = extractor.find_best(id);
    LamLang::display_recexpr(&rec)
}

#[test]
fn depth_zero_no_hoist_is_identity() {
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ 1 2)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[], &[]);
    eg.rebuild();
    assert_eq!(eg.find(wrapped), eg.find(id));
}

#[test]
fn unused_internal_lam_still_wraps() {
    // Captured `(+ 1 2)` is closed (no fv). Caller passes internal=[0] anyway
    // (e.g., because some other match for the same metavar references $0); the
    // wrap-lam gets emitted but binds nothing inside this arg.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ 1 2)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[0], &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (+ 1 2))");
}

#[test]
fn pattern_internal_var_stays_at_zero() {
    // `$0` references a pattern-internal binder; internal=[0] gets bound by
    // the inner-stack wrap-lam. The leaf stays as `$0` (formula gives e+d=0).
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 7)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[0], &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (+ $0 7))");
}

#[test]
fn deep_pattern_wrap_keeps_indices_aligned() {
    // Both `$0` and `$1` are pattern-internal under internal=[0, 1]; they keep
    // their original indices in the body of the doubly-wrapped lam.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $1)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[0, 1], &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $0 $1)))");
}

#[test]
fn bound_vars_inside_arg_are_preserved() {
    // The arg is `(lam $0)` — its `$0` is bound *inside* the arg. Wrapping in
    // another lam shouldn't change the meaning. Caller specifies internal=[0]
    // (unused for this match).
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(lam $0)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[0], &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam $0))");
}

#[test]
fn hoist_only_makes_arg_a_closed_function() {
    // No internals, single hoist over original index 0. Result wraps in one
    // hoist-lam binding the captured's free `$0`.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "$0");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[], &[0]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam $0)");
    assert!(eg[wrapped].data.fv.is_empty());
}

#[test]
fn internal_inner_hoist_outer() {
    // internal=[0], hoist=[1] (original-frame index 1 hoisted). Captured
    // `(+ $0 $1)`: $0 bound by inner-stack lam (formula: depth + p = 0); $1
    // bound by outer hoist-stack lam (formula: depth + (m+n-1-q) = 1).
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $1)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[0], &[1]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $0 $1)))");
    assert!(eg[wrapped].data.fv.is_empty());
}

#[test]
fn multiple_hoist_indices_ordered_smallest_outermost() {
    // hoist=[0, 3]: two hoisted indices. Convention: outermost hoist-lam binds
    // the smallest (0), innermost binds the largest (3). Captured = `(+ $0 $3)`:
    // $0 → outer hoist-lam (body-frame depth m+n-1-q = 1, leaf depth 0 → $1);
    // $3 → inner (body-frame depth 0, leaf depth 0 → $0). Result `(+ $1 $0)`.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $3)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, &[], &[0, 3]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $1 $0)))");
    assert!(eg[wrapped].data.fv.is_empty());
}
