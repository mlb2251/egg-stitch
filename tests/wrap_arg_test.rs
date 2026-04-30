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
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 0, &[]);
    eg.rebuild();
    assert_eq!(eg.find(wrapped), eg.find(id));
}

#[test]
fn closed_arg_wraps_with_unchanged_body() {
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ 1 2)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 1, &[]);
    eg.rebuild();
    // No free vars in the captured subterm; one outer lam wraps over it
    // (representing a pattern-internal binder that's unused inside the arg).
    assert_eq!(extract(&eg, wrapped), "(lam (+ 1 2))");
}

#[test]
fn pattern_internal_var_stays_at_zero() {
    // `$0` references a pattern-internal binder; with depth=1 it gets bound
    // by the wrap-lam without any index shift (η-expansion, not shift+wrap).
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 7)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 1, &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (+ $0 7))");
}

#[test]
fn deep_pattern_wrap_keeps_indices_aligned() {
    // Both `$0` and `$1` are pattern-internal under depth=2; they keep their
    // original indices in the body of the doubly-wrapped lam.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $1)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 2, &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $0 $1)))");
}

#[test]
fn bound_vars_inside_arg_are_preserved() {
    // The arg is `(lam $0)` — its `$0` is bound *inside* the arg. Wrapping in
    // another lam shouldn't change the meaning.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(lam $0)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 1, &[]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam $0))");
}

#[test]
fn hoist_at_depth_zero_makes_arg_a_closed_function() {
    // depth=0, hoist=[0]: the captured `$0` is treated as an outer-context
    // ref. We η-expand by 1 hoist-lam binding it; result is `(lam $0)` —
    // closed.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "$0");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 0, &[0]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam $0)");
    // The wrapped form has empty fv (closed under the hoist).
    assert!(eg[wrapped].data.fv.is_empty());
}

#[test]
fn hoist_with_pattern_depth_orders_inner_pattern_outer_hoist() {
    // depth=1 (one pattern-internal binder), hoist=[0] (one outer ref to bind).
    // Captured `(+ $0 $1)` has fv {0, 1}; under d_k=1 the $0 is bound by the
    // inner pattern-lam, the $1 (post-wrap fv {0}) gets bound by the outer
    // hoist-lam.
    //
    // Wrap stack (outer→inner): [hoist_lam, pattern_lam]. Inside the body
    // (depth 2), $0 (pattern-internal) → still $0; $1 (hoisted) → $1.
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $1)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 1, &[0]);
    eg.rebuild();
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $0 $1)))");
    assert!(eg[wrapped].data.fv.is_empty());
}

#[test]
fn multiple_hoist_indices_ordered_smallest_outermost() {
    // depth=0, hoist=[0, 3]: two hoisted indices. Convention: outermost hoist
    // lam binds the smallest index (0), innermost binds the largest (3).
    // Captured = `(+ $0 $3)`: $0 → outermost hoist-lam (at index n-1=1 from
    // body), $3 → innermost (at index 0 from body).
    let mut eg = fresh_egraph();
    let id = add(&mut eg, "(+ $0 $3)");
    let wrapped = wrap_arg_for_abstraction::<LambdaCalc, OpDB<Op>>(&mut eg, id, 0, &[0, 3]);
    eg.rebuild();
    // Body (depth 2 inside two wrap-lams): $0 → $1 (outer hoist-lam),
    // $3 → $0 (inner hoist-lam). Result: (lam (lam (+ $1 $0))).
    assert_eq!(extract(&eg, wrapped), "(lam (lam (+ $1 $0)))");
    assert!(eg[wrapped].data.fv.is_empty());
}
