//! Unit tests for `revexpr::abstract_with_hoist`.
//!
//! `abstract_with_hoist(expr, root, d_k, hoist)` wraps `expr` in `d_k + n` lams
//! (where `n = hoist.len()`), substituting De Bruijn indices to positional refs:
//! - inner `d_k` lams bind pattern-internal indices `0..d_k-1`
//! - outer `n` lams bind hoisted post-pattern-wrap indices in `hoist` (sorted
//!   ascending; outermost lam binds the smallest)

use egg_stitch::lang::{LambdaCalcLanguage, Op, OpDB, StitchLanguage};
use egg_stitch::revexpr::abstract_with_hoist;

type LamLang = LambdaCalcLanguage<OpDB<Op>>;

fn run(prog: &str, d_k: u32, hoist: &[u32]) -> String {
    let expr = LamLang::parse_program(prog).unwrap();
    let root = (expr.as_ref().len() - 1).into();
    let wrapped = abstract_with_hoist::<LamLang>(&expr, root, d_k, hoist);
    LamLang::display_recexpr(&wrapped)
}

#[test]
fn no_wrap_no_hoist_returns_original_term_only_via_zero_lams() {
    // d_k=0, hoist=[]: zero lams added, body unchanged. (Not really useful in
    // practice — exercises the empty case.)
    assert_eq!(run("(+ 1 2)", 0, &[]), "(+ 1 2)");
}

#[test]
fn pattern_wrap_only_keeps_indices_aligned() {
    // Captured `(a b c $0 f)` with d_k=2: $0 is bound by inner pattern-lam.
    // Body indices unchanged because formula gives e + d = 0 at depth 0.
    assert_eq!(run("(a b c $0 f)", 2, &[]), "(lam (lam (a b c $0 f)))");
}

#[test]
fn open_var_at_depth_zero_with_hoist_zero_yields_lambda_identity() {
    // Captured `$0`, d_k=0, hoist=[0]: one outer hoist-lam binds the index.
    // Result is `(lam $0)` — identity function.
    assert_eq!(run("$0", 0, &[0]), "(lam $0)");
}

#[test]
fn pattern_internal_and_hoist_compose() {
    // Captured `(+ $0 $1)` with d_k=1, hoist=[0]: $0 is pattern-internal
    // (stays $0); $1 maps to post-wrap index 0, which is hoisted at position 0
    // of one outer hoist-lam → ends up as $1 in the body.
    assert_eq!(run("(+ $0 $1)", 1, &[0]), "(lam (lam (+ $0 $1)))");
}

#[test]
fn hoist_smallest_index_outermost() {
    // hoist=[0, 3]: outermost binds 0, innermost binds 3.
    // Body indices (depth 2): $0 → outer hoist-lam (index n-1-p=1), $3 → inner (0).
    assert_eq!(run("(+ $0 $3)", 0, &[0, 3]), "(lam (lam (+ $1 $0)))");
}

#[test]
fn three_hoist_indices_ordered() {
    // hoist=[1, 2, 5]: outermost binds 1, middle binds 2, innermost binds 5.
    // Inside body (depth 3): $1 → $2 (n-1-0=2), $2 → $1 (n-1-1=1), $5 → $0.
    assert_eq!(run("(+ $1 (+ $2 $5))", 0, &[1, 2, 5]), "(lam (lam (lam (+ $2 (+ $1 $0)))))");
}

#[test]
fn body_internal_lams_track_depth() {
    // `(lam (a $0 $1))` — inner `$0` is bound by the captured's own lam, `$1`
    // refers to one above (i.e., to whatever encloses the captured). With
    // d_k=1, the $1 (post-wrap index 0) is bound by the inner pattern-lam.
    // Total wrap = 1 (the captured's own lam stays in the body).
    assert_eq!(run("(lam (a $0 $1))", 1, &[]), "(lam (lam (a $0 $1)))");
}

#[test]
fn hoist_threads_through_internal_binders() {
    // `(lam $1)`: inside the captured's lam, $1 references one above (post-wrap
    // index 0 once d_k=0). Hoist=[0] → one outer hoist-lam.
    // Body of the captured `lam` has $1; from outside the hoist-lam wrap, $1
    // points at the hoist-lam (at body-frame depth 0; under captured's lam at
    // depth 1, that's index 1). So result body keeps `$1`.
    assert_eq!(run("(lam $1)", 0, &[0]), "(lam (lam $1))");
}

#[test]
#[should_panic(expected = "not in hoist set")]
fn unspecified_free_index_panics() {
    // `(+ $5 1)` has fv {5}; hoist is empty, so the wrap can't close it.
    run("(+ $5 1)", 0, &[]);
}

#[test]
fn dag_sharing_visits_shared_subterm_once_per_depth() {
    // `(+ (a $0) (a $0))` shares e-classes for a leaf if parsed via egraph,
    // but RecExpr from parsing may have separate nodes. Either way the result
    // is the same body with $0 substituted consistently.
    // d_k=1, hoist=[]: $0 stays $0 in the body.
    assert_eq!(run("(+ (a $0) (a $0))", 1, &[]), "(lam (+ (a $0) (a $0)))");
}
