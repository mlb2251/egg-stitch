//! Unit tests for `build_shifted_variants` and `ShiftedVariants::get`.
//!
//! Build a tiny lambda-calc egraph by hand, run the variant builder, and
//! assert the public lookup API: identity at `s=0`, presence of variants up
//! to the observed max depth, absence past it, absence for fv-empty classes,
//! and that returned ids are canonical post-rebuild.

use egg::Id;
use egg_stitch::lang::{LambdaCalc, LambdaCalcLanguage, Op, OpDB, StitchAnalysis, StitchLanguage};
use egg_stitch::shifted::{ShiftedVariants, build_shifted_variants};

type LamLang = LambdaCalcLanguage<OpDB<Op>>;

/// Sorted fv of the canonical eclass containing `id`.
fn fv_sorted(eg: &egg::EGraph<LamLang, StitchAnalysis>, id: Id) -> Vec<i32> {
    let mut v: Vec<i32> = eg[eg.find(id)].data.fv.iter().copied().collect();
    v.sort();
    v
}

#[test]
fn default_is_empty() {
    let v = ShiftedVariants::default();
    // Identity holds for any id at shift 0 even without an entry.
    assert_eq!(v.get(Id::from(0usize), 0), Some(Id::from(0usize)));
    assert_eq!(v.get(Id::from(0usize), 1), None);
}

#[test]
fn no_variants_for_closed_term() {
    // `(lam $0)` is fv-empty: the build pass must skip every class under it,
    // and `get` returns `Some` only at `s = 0`.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, root);
    let root = eg.find(root);
    assert_eq!(v.get(root, 0), Some(root));
    assert_eq!(v.get(root, 1), None);
}

#[test]
fn variants_built_up_to_observed_depth() {
    // `(lam (lam $1))`: the `$1` leaf sits under two binders and has fv {1}.
    //   shift s=1 → `$0`   (fv {0})    — still a positive free index
    //   shift s=2 → `$-1`  (fv {-1})   — re-wrap slot
    //   s=3 is past observed depth, so no variant is built.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam (lam $1))").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, root);
    let leaf = eg.find(leaf);

    assert_eq!(v.get(leaf, 0), Some(leaf));
    let s1 = v.get(leaf, 1).expect("shift by 1 should be built");
    assert_eq!(fv_sorted(&eg, s1), vec![0]);
    let s2 = v.get(leaf, 2).expect("shift by 2 should be built");
    assert_eq!(fv_sorted(&eg, s2), vec![-1]);
    assert_eq!(v.get(leaf, 3), None);
}

#[test]
fn get_returns_canonical_id() {
    // Documented contract: the recorded ids are canonicalized so callers
    // don't have to `find` them.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam $1)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, root);
    let leaf = eg.find(leaf);
    let s1 = v.get(leaf, 1).expect("shift by 1");
    assert_eq!(s1, eg.find(s1), "returned id must be canonical");
}

#[test]
fn fv_empty_subclass_gets_no_entry() {
    // The outer `lam` class in `(lam $0)` has fv {} — `build_shifted_variants`
    // must not record an entry for it even though it sits at depth 0 (the
    // root) and its child reaches a DB-var. `get` at s≥1 returns None.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, root);
    let root = eg.find(root);
    assert!(eg[root].data.fv.is_empty());
    assert_eq!(v.get(root, 1), None);
    assert_eq!(v.get(root, 2), None);
}
