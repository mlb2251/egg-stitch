//! Regression tests for `LambdaCalc::unwrap_pattern_db_apps` — the inverse of
//! `wrap_pattern_with_db_apps` used by `follow::matches_follow_serialized` to
//! collapse the optimiser's η-wrap so the follow check is independent of which
//! `vis` ordering the displayer happened to pick.
//!
//! Each round-trip test wraps a fresh `?#0` metavar head with a known
//! `db_args` and asserts that `unwrap` returns the same inner head id.
//! `ho_arity ≥ 2` is the case the original implementation got wrong (the
//! direction comparison was inverted), so it gets first-class coverage here.

use egg::RecExpr;
use egg_stitch::lang::{LambdaCalc, LambdaCalcLanguage, LanguageFamily, Op, OpDB, OpWithVar, StitchOp};

type Lang = LambdaCalcLanguage<OpWithVar<OpDB<Op>>>;

fn metavar_head(r: &mut RecExpr<Lang>, k: u32) -> egg::Id {
    r.add(LambdaCalcLanguage::Leaf(OpWithVar::Var(egg::Var::from(k))))
}

fn round_trip(db_args: &[i32]) {
    let mut r: RecExpr<Lang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let wrapped = LambdaCalc::wrap_pattern_with_db_apps::<OpDB<Op>>(&mut r, head, db_args);
    assert_eq!(LambdaCalc::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), wrapped), head, "unwrap should recover the metavar head for db_args = {db_args:?}");
}

#[test]
fn round_trip_ho_arity_1() {
    round_trip(&[0]);
}

#[test]
fn round_trip_ho_arity_2_canonical() {
    // `vis = [0, 1]` (sorted ascending) → `db_args = vis.iter().rev() = [1, 0]`.
    round_trip(&[1, 0]);
}

#[test]
fn round_trip_ho_arity_3_canonical() {
    round_trip(&[2, 1, 0]);
}

#[test]
fn round_trip_ho_arity_2_sparse() {
    // Non-contiguous vis like `[0, 2]` → `db_args = [2, 0]`.
    round_trip(&[2, 0]);
}

#[test]
fn binary_op_chain_with_non_metavar_head_is_left_alone() {
    // `(f $1 $0)` parses curried to `App(App(f, $1), $0)` — the same shape an
    // η-wrap takes, but the head is a regular op, not a metavar. The guard in
    // `unwrap` should leave it untouched so genuine binary-op chains aren't
    // collapsed.
    let mut r: RecExpr<Lang> = RecExpr::default();
    let f = r.add(LambdaCalcLanguage::Leaf(OpWithVar::Node(OpDB::<Op>::from_name("f"))));
    let v1 = r.add(LambdaCalcLanguage::Leaf(OpWithVar::Node(OpDB::Var(1))));
    let app1 = r.add(LambdaCalcLanguage::App([f, v1]));
    let v0 = r.add(LambdaCalcLanguage::Leaf(OpWithVar::Node(OpDB::Var(0))));
    let app2 = r.add(LambdaCalcLanguage::App([app1, v0]));
    assert_eq!(LambdaCalc::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), app2), app2);
}

#[test]
fn metavar_alone_is_left_alone() {
    // A bare `?#0` (no surrounding App) should pass through unchanged — the
    // function only peels when there's at least one DB-arg App to remove.
    let mut r: RecExpr<Lang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    assert_eq!(LambdaCalc::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), head), head);
}
