//! Tests for the free-variable analysis on `StitchAnalysis`.
//!
//! Covers both language families (`OpChildrenLanguage` and `LambdaCalcLanguage`)
//! parameterised by `OpDB<Op>`, plus the no-binders / no-DB-vars sanity case
//! using plain `Op` (where `fv` should always be empty).

use egg::Id;
use egg_stitch::lang::{LambdaCalcLanguage, Op, OpChildrenLanguage, OpDB, StitchAnalysis, StitchLanguage};

type LamLang = LambdaCalcLanguage<OpDB<Op>>;
type OcLang = OpChildrenLanguage<OpDB<Op>>;

fn fv<L: StitchLanguage>(prog: &str) -> Vec<u32> {
    let expr = L::parse_program(prog).unwrap();
    let mut eg: egg::EGraph<L, StitchAnalysis> = egg::EGraph::default();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    let mut v: Vec<u32> = eg[id].data.fv.iter().copied().collect();
    v.sort();
    v
}

// ---- OpChildrenLanguage<OpDB<Op>>: De Bruijn leaves but no binders ----

#[test]
fn oc_var_leaf_carries_index() {
    assert_eq!(fv::<OcLang>("$0"), vec![0]);
    assert_eq!(fv::<OcLang>("$3"), vec![3]);
}

#[test]
fn oc_no_binders_so_fv_is_union() {
    // OpChildrenLanguage has no binding nodes, so `lam` here is just an opaque
    // 1-ary symbol — the `$0` underneath stays free.
    assert_eq!(fv::<OcLang>("(lam $0)"), vec![0]);
    assert_eq!(fv::<OcLang>("(+ $0 $2)"), vec![0, 2]);
    assert_eq!(fv::<OcLang>("(+ $0 $0)"), vec![0]);
}

#[test]
fn oc_plain_symbols_have_empty_fv() {
    assert_eq!(fv::<OcLang>("(+ 1 2)"), Vec::<u32>::new());
}

// ---- LambdaCalcLanguage<OpDB<Op>>: binders + DB vars ----

#[test]
fn lam_var_leaf_carries_index() {
    assert_eq!(fv::<LamLang>("$0"), vec![0]);
    assert_eq!(fv::<LamLang>("$5"), vec![5]);
}

#[test]
fn lam_binds_zero() {
    assert_eq!(fv::<LamLang>("(lam $0)"), Vec::<u32>::new());
    assert_eq!(fv::<LamLang>("(lam $1)"), vec![0]);
    assert_eq!(fv::<LamLang>("(lam (lam $1))"), Vec::<u32>::new());
    assert_eq!(fv::<LamLang>("(lam (lam $2))"), vec![0]);
}

#[test]
fn lam_union_across_app_chain() {
    // (+ $0 $2) appifies to (@ (@ + $0) $2); fv unions over the spine.
    assert_eq!(fv::<LamLang>("(+ $0 $2)"), vec![0, 2]);
}

#[test]
fn lam_partial_binder_keeps_outer_free() {
    // `(lam (+ $0 $1))` binds $0; $1 stays free at index 0 after the shift.
    assert_eq!(fv::<LamLang>("(lam (+ $0 $1))"), vec![0]);
}

#[test]
fn lam_plain_program_has_empty_fv() {
    assert_eq!(fv::<LamLang>("(+ 1 2)"), Vec::<u32>::new());
}

// ---- Plain Op (no DB-var slot) is unaffected ----

#[test]
fn plain_op_fv_always_empty() {
    type Plain = OpChildrenLanguage<Op>;
    let expr = Plain::parse_program("(foo a b c)").unwrap();
    let mut eg: egg::EGraph<Plain, StitchAnalysis> = egg::EGraph::default();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    assert!(eg[id].data.fv.is_empty());
    // `$0` parses as a plain symbol when the leaf type doesn't admit DB vars.
    let expr = Plain::parse_program("$0").unwrap();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    assert!(eg[id].data.fv.is_empty());
}

// ---- Size still matches the previous semantics ----

#[test]
fn size_is_unchanged_by_fv_addition() {
    let expr = LamLang::parse_program("(lam (+ $0 1))").unwrap();
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    // Default weights are all 1. Curried form of (+ $0 1) is (@ (@ + $0) 1):
    // 5 leaf/structural nodes (`+`, `$0`, `@`, `1`, `@`) + 1 outer `lam` = 6.
    assert_eq!(eg[id].data.size, 6);
}

// ---- Merge takes intersection of fv ----

#[test]
fn merge_intersects_fv() {
    // `$0` has fv {0}, `$1` has fv {1}. Intersection is empty: in the merged
    // class, neither index is free in *every* representative.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let v0 = eg.add_expr(&LamLang::parse_program("$0").unwrap());
    let v1 = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.union(v0, v1);
    eg.rebuild();
    let merged: Id = eg.find(v0);
    let got: Vec<u32> = eg[merged].data.fv.iter().copied().collect();
    assert_eq!(got, Vec::<u32>::new());
}

// ---- Annihilator: merging fv-dropping rewrites refines fv to the intersection ----
//
// With the rule `(* 0 x) ↔ 0`, the eclass `{0, (* 0 $1)}` denotes 0
// regardless of x. Intersection-based fv reports `{}`, matching the
// semantic fv. Soundness of using this for `inv_0` capture relies on
// extraction (AstSize) picking the fv-minimal representative; see
// `check_fvs_are_as_expected` for the runtime guard.
#[test]
fn merge_with_annihilator_should_not_inherit_dropped_fv() {
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    // `0` has empty fv; `(* 0 $1)` has fv {1}. With the rule `(* 0 x) ↔ 0`,
    // these are semantically equal — the eclass denotes 0 regardless of `x`.
    let zero = eg.add_expr(&LamLang::parse_program("0").unwrap());
    let mul = eg.add_expr(&LamLang::parse_program("(* 0 $1)").unwrap());
    eg.union(zero, mul);
    eg.rebuild();
    let merged: Id = eg.find(zero);
    let got: Vec<u32> = eg[merged].data.fv.iter().copied().collect();
    assert_eq!(got, Vec::<u32>::new());
}
