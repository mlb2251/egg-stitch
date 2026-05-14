//! Unit tests for `build_shifted_variants` and `ShiftedVariants::get`.
//!
//! Build a tiny lambda-calc egraph by hand, run the variant builder, and
//! assert the public lookup API: identity at `s=0`, presence of variants up
//! to `max(fv)`, absence past it (variant-only classes are not built),
//! absence for fv-empty classes, and that returned ids are canonical
//! post-rebuild.

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
    // `(lam $0)` is fv-empty: the build pass must skip the root class, and
    // `get` returns `Some` only at `s = 0`.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg);
    let root = eg.find(root);
    assert_eq!(v.get(root, 0), Some(root));
    assert_eq!(v.get(root, 1), None);
}

#[test]
fn variants_built_up_to_max_fv() {
    // `$1` has fv {1}, so max(fv) = 1:
    //   shift s=1 → `$0`   (fv {0})    — still a positive free index
    //   s=2 would give `$-1` (a variant-only class with no original
    //   counterpart); not built — those would leak into extracted programs
    //   via captured metavar substitutions.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $1)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg);
    let leaf = eg.find(leaf);

    assert_eq!(v.get(leaf, 0), Some(leaf));
    let s1 = v.get(leaf, 1).expect("shift by 1 should be built");
    assert_eq!(fv_sorted(&eg, s1), vec![0]);
    assert_eq!(v.get(leaf, 2), None);
}

#[test]
fn fv_zero_only_gets_no_variants() {
    // A class with fv {0} would yield a variant-only class under any negative
    // shift (s=1 gives `$-1`), so no variants are built.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$0").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg);
    let leaf = eg.find(leaf);
    assert_eq!(eg[leaf].data.fv.iter().copied().collect::<Vec<_>>(), vec![0]);
    assert_eq!(v.get(leaf, 0), Some(leaf));
    assert_eq!(v.get(leaf, 1), None);
}

#[test]
fn get_returns_canonical_id() {
    // Documented contract: the recorded ids are canonicalized so callers
    // don't have to `find` them.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $1)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg);
    let leaf = eg.find(leaf);
    let s1 = v.get(leaf, 1).expect("shift by 1");
    assert_eq!(s1, eg.find(s1), "returned id must be canonical");
}

#[test]
fn fv_empty_subclass_gets_no_entry() {
    // The outer `lam` class in `(lam $0)` has fv {} — `build_shifted_variants`
    // must not record an entry for it. `get` at s≥1 returns None.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    let root = eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    eg.rebuild();
    let v = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg);
    let root = eg.find(root);
    assert!(eg[root].data.fv.is_empty());
    assert_eq!(v.get(root, 1), None);
    assert_eq!(v.get(root, 2), None);
}
