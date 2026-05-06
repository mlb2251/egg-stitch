//! Pattern binder-depth tests for `LambdaCalcLanguage<OpDB<Op>>`.
//!
//! Mirrors the structural tests in `pattern.rs` but exercises the
//! `var_depth` invariant introduced by binding-aware expansion: `Lam` bumps
//! the depth of the meta-var that lands in its body; `App`/`Leaf` don't.

use egg::Id;
use egg_stitch::lang::{LambdaCalc, LambdaCalcDisc, LambdaCalcLanguage, LanguageFamily, Op, OpDB, StitchOp};
use egg_stitch::pattern::Pattern;

type LamLang = LambdaCalcLanguage<OpDB<Op>>;
type Pat = Pattern<LambdaCalc, OpDB<Op>>;

/// Lam enode shape (1 child). The placeholder Id is overwritten by `expand`.
fn lam() -> LamLang {
    <LambdaCalc as LanguageFamily>::make::<OpDB<Op>>(LambdaCalcDisc::Lam, vec![Id::from(0)])
}

/// App enode shape (2 children).
fn app() -> LamLang {
    <LambdaCalc as LanguageFamily>::make::<OpDB<Op>>(LambdaCalcDisc::App, vec![Id::from(0); 2])
}

/// 0-arity Leaf enode for a named symbol.
fn leaf(name: &str) -> LamLang {
    <LambdaCalc as LanguageFamily>::make::<OpDB<Op>>(LambdaCalcDisc::Leaf(OpDB::<Op>::from_name(name)), vec![])
}

#[test]
fn single_var_depth_is_zero() {
    let p: Pat = Pattern::single_var();
    assert_eq!(p.var_depth, vec![0]);
}

#[test]
fn lam_bumps_child_depth() {
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &lam());
    assert_eq!(p.to_string(), "(lam ?#0)");
    assert_eq!(p.var_depth, vec![1]);
    p.expand(0, &lam());
    assert_eq!(p.to_string(), "(lam (lam ?#0))");
    assert_eq!(p.var_depth, vec![2]);
}

#[test]
fn app_does_not_bump_depth() {
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &app()); // (@ ?#0 ?#1) at depths [0, 0]
    assert_eq!(p.var_depth, vec![0, 0]);
}

#[test]
fn lam_then_app_inherits_outer_depth() {
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &lam()); // (lam ?#0), depths [1]
    p.expand(0, &app()); // (lam (@ ?#0 ?#1)), both children inherit depth 1
    assert_eq!(p.var_depth, vec![1, 1]);
}

#[test]
fn mixed_depths_are_independent() {
    // Build (@ (lam ?#0) ?#1): only the meta-var inside `lam` gets bumped.
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &app()); // (@ ?#0 ?#1), depths [0, 0]
    p.expand(0, &lam()); // (@ (lam ?#0) ?#1), depths [1, 0]
    assert_eq!(p.var_depth, vec![1, 0]);
}

#[test]
fn reuse_at_equal_depth_ok() {
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &app()); // depths [0, 0]
    p.reuse(0, 1);
    assert_eq!(p.var_depth, vec![0]);
}

#[test]
fn reuse_at_unequal_depth_takes_max() {
    // Cross-depth reuse is allowed: the merged metavar adopts the strictest
    // (max) depth. The captures filter (`subset_matches_reuse`) enforces that
    // both occurrences' kept e-class fits the new max-depth invariant; this
    // structural test only checks the depth-bookkeeping side.
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &app()); // depths [0, 0]
    p.expand(0, &lam()); // (@ (lam ?#0) ?#1), depths [1, 0]
    p.reuse(0, 1);
    assert_eq!(p.var_depth, vec![1]);
}

#[test]
fn leaf_expansion_drops_var() {
    // Filling a hole with a 0-arity leaf removes that meta-var entirely; depth
    // bookkeeping shouldn't trip on the empty insert.
    let mut p: Pat = Pattern::single_var();
    p.expand(0, &lam()); // (lam ?#0), depths [1]
    p.expand(0, &leaf("foo")); // (lam foo), no remaining holes
    assert!(p.var_depth.is_empty());
    assert!(p.vars.is_empty());
}
