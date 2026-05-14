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
    let (v, _) = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, 4);
    let root = eg.find(root);
    assert_eq!(v.get(root, 0), Some(root));
    assert_eq!(v.get(root, 1), None);
}

#[test]
fn variants_built_up_to_max_shift() {
    // `$1` has fv {1}. With max_shift=4 the builder produces variants at
    // s=1..4: s=1 → `$0` (fv {0}, still a positive free index); s=2..4 cross
    // the original-fv boundary and produce re-wrap slots `$-1`, `$-2`, `$-3`.
    // ho-arity uses those negatives at capture-time depth lookups.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $1)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let (v, _) = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, 4);
    let leaf = eg.find(leaf);

    assert_eq!(v.get(leaf, 0), Some(leaf));
    let s1 = v.get(leaf, 1).expect("shift by 1 should be built");
    assert_eq!(fv_sorted(&eg, s1), vec![0]);
    let s2 = v.get(leaf, 2).expect("shift by 2 should be built (re-wrap slot)");
    assert_eq!(fv_sorted(&eg, s2), vec![-1]);
    let s3 = v.get(leaf, 3).expect("shift by 3 should be built");
    assert_eq!(fv_sorted(&eg, s3), vec![-2]);
    assert_eq!(v.get(leaf, 5), None);
}

#[test]
fn fv_zero_only_gets_negative_variants() {
    // A class with fv {0} yields re-wrap-slot variants under negative shifts:
    // s=1 → `$-1`, s=2 → `$-2`. These exist now so capture-time lookups can
    // find them.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $0)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$0").unwrap());
    eg.rebuild();
    let (v, _) = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, 3);
    let leaf = eg.find(leaf);
    assert_eq!(eg[leaf].data.fv.iter().copied().collect::<Vec<_>>(), vec![0]);
    assert_eq!(v.get(leaf, 0), Some(leaf));
    let s1 = v.get(leaf, 1).expect("shift by 1 should be built (re-wrap slot)");
    assert_eq!(fv_sorted(&eg, s1), vec![-1]);
}

#[test]
fn get_returns_canonical_id() {
    // Documented contract: the recorded ids are canonicalized so callers
    // don't have to `find` them.
    let mut eg: egg::EGraph<LamLang, StitchAnalysis> = egg::EGraph::default();
    eg.add_expr(&LamLang::parse_program("(lam $1)").unwrap());
    let leaf = eg.add_expr(&LamLang::parse_program("$1").unwrap());
    eg.rebuild();
    let (v, _) = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, 4);
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
    let (v, _) = build_shifted_variants::<LambdaCalc, OpDB<Op>>(&mut eg, 4);
    let root = eg.find(root);
    assert!(eg[root].data.fv.is_empty());
    assert_eq!(v.get(root, 1), None);
    assert_eq!(v.get(root, 2), None);
}
