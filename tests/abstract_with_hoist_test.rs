//! Unit tests for `revexpr::abstract_with_hoist`.
//!
//! `abstract_with_hoist(expr, root, internal, hoist)` wraps `expr` in
//! `internal.len() + hoist.len()` lams and rewrites every free DB-var leaf to
//! a positional ref bound by one of the wrap-lams. `internal` and `hoist` are
//! both sorted-ascending original-frame indices and disjoint.
//!
//! Conventions on the wrap stack (deepest = closest to the body):
//! - innermost-of-internal binds `internal[0]` (smallest internal).
//! - outermost-of-internal binds `internal[m-1]` (largest internal).
//! - innermost-of-hoist binds `hoist[n-1]` (largest hoist).
//! - outermost-of-hoist binds `hoist[0]` (smallest hoist).

use egg_stitch::lang::{LambdaCalcLanguage, Op, OpDB, StitchLanguage};
use egg_stitch::revexpr::abstract_with_hoist;

type LamLang = LambdaCalcLanguage<OpDB<Op>>;

fn run(prog: &str, internal: &[u32], hoist: &[u32]) -> String {
    let expr = LamLang::parse_program(prog).unwrap();
    let root = (expr.as_ref().len() - 1).into();
    let wrapped = abstract_with_hoist::<LamLang>(&expr, root, internal, hoist);
    LamLang::display_recexpr(&wrapped)
}

#[test]
fn empty_internal_and_hoist_returns_original_unchanged() {
    // No wrapping at all. (Not really useful in practice — exercises the
    // zero case.)
    assert_eq!(run("(+ 1 2)", &[], &[]), "(+ 1 2)");
}

#[test]
fn pattern_internal_only_keeps_indices_aligned() {
    // Captured `(a b c $0 f)` with internal=[0]: $0 bound by the inner-stack
    // wrap-lam. Formula at depth 0 with p=0 gives new_i = 0 → stays $0.
    assert_eq!(run("(a b c $0 f)", &[0], &[]), "(lam (a b c $0 f))");
}

#[test]
fn two_internals_keep_indices_aligned() {
    // internal=[0, 1] over `(+ $0 $1)`. $0 → p=0 → $0; $1 → p=1 → $1.
    assert_eq!(run("(+ $0 $1)", &[0, 1], &[]), "(lam (lam (+ $0 $1)))");
}

#[test]
fn hoist_only_makes_arg_a_closed_function() {
    // Captured `$0`, hoist=[0]: one outer hoist-lam binds the index. Result
    // is `(lam $0)` — identity-like.
    assert_eq!(run("$0", &[], &[0]), "(lam $0)");
}

#[test]
fn internal_and_hoist_compose() {
    // Captured `(+ $0 $1)` with internal=[0], hoist=[1]. $0 (internal at p=0)
    // → $0; $1 (hoist at q=0; m=1, n=1) → depth + (m+n-1-q) = 0 + 1 = $1.
    assert_eq!(run("(+ $0 $1)", &[0], &[1]), "(lam (lam (+ $0 $1)))");
}

#[test]
fn hoist_smallest_index_outermost() {
    // hoist=[0, 3]: outermost binds 0, innermost binds 3.
    // $0 (q=0; m=0, n=2) → 0 + (0+2-1-0) = $1; $3 (q=1) → 0 + (0+2-1-1) = $0.
    assert_eq!(run("(+ $0 $3)", &[], &[0, 3]), "(lam (lam (+ $1 $0)))");
}

#[test]
fn three_hoist_indices_ordered() {
    // hoist=[1, 2, 5]: outermost→1, middle→2, innermost→5.
    // $1 → q=0, n-1-q=2 → $2; $2 → q=1 → $1; $5 → q=2 → $0.
    assert_eq!(run("(+ $1 (+ $2 $5))", &[], &[1, 2, 5]), "(lam (lam (lam (+ $2 (+ $1 $0)))))");
}

#[test]
fn body_internal_lams_track_depth() {
    // `(lam (a $0 $1))` — inner `$0` is bound by captured's own lam, `$1`
    // is free at depth 1. internal=[0] → $1's effective e = 1-1 = 0, p=0,
    // new_i = depth+0 = 1 → stays $1.
    assert_eq!(run("(lam (a $0 $1))", &[0], &[]), "(lam (lam (a $0 $1)))");
}

#[test]
fn hoist_threads_through_internal_binders() {
    // `(lam $1)`: inside the captured's lam (depth 1), $1 has e = 0 — free
    // at the captured's outer frame. With hoist=[0]: q=0; m=0, n=1.
    // new_i = depth + (m+n-1-q) = 1 + 0 = $1.
    assert_eq!(run("(lam $1)", &[], &[0]), "(lam (lam $1))");
}

#[test]
#[should_panic(expected = "not in internal")]
fn unspecified_free_index_panics() {
    // `(+ $5 1)` has fv {5}; neither set covers it.
    run("(+ $5 1)", &[], &[]);
}

#[test]
fn dag_sharing_visits_shared_subterm_once_per_depth() {
    // `(+ (a $0) (a $0))` — internal=[0]: $0 stays $0 in the body.
    assert_eq!(run("(+ (a $0) (a $0))", &[0], &[]), "(lam (+ (a $0) (a $0)))");
}

#[test]
fn unused_internal_lam_is_emitted() {
    // Captured `(+ 1 2)` is closed but caller passes internal=[0] — the wrap
    // happens; the lam binds nothing, but the body shape is preserved.
    assert_eq!(run("(+ 1 2)", &[0], &[]), "(lam (+ 1 2))");
}
