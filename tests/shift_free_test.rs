//! Tests for `revexpr::shift_free` on `LambdaCalcLanguage<OpDB<Op>>`.
//!
//! `shift_free` mutates a `RevExpr` in place, rewriting every free `$n` leaf
//! (those at index ≥ the running depth) to `$(n + by)`. Bound vars (those at
//! `n < depth`) and non-DB nodes are left alone.

use egg::Id;
use egg_stitch::lang::{LambdaCalcLanguage, Op, OpDB, StitchLanguage};
use egg_stitch::revexpr::{RevExpr, shift_free};

type LamLang = LambdaCalcLanguage<OpDB<Op>>;

/// Parse a program and convert to a `RevExpr` so we can call `shift_free`.
fn parse(s: &str) -> RevExpr<LamLang> {
    LamLang::parse_program(s).unwrap().into()
}

/// Render the `RevExpr` back to its s-expression display form (which goes
/// through `display_recexpr`, i.e. unappified).
fn show(expr: &RevExpr<LamLang>) -> String {
    let recexpr: egg::RecExpr<LamLang> = expr.clone().into();
    LamLang::display_recexpr(&recexpr)
}

/// Apply `shift_free` rooted at the top of `expr`. Roots in a `RevExpr` are
/// always at index 0 (it stores nodes in reverse DFS order).
fn shift_top(expr: &mut RevExpr<LamLang>, by: i32, initial_depth: u32) {
    shift_free(expr, Id::from(0), by, initial_depth);
}

#[test]
fn shift_bare_var_at_depth_zero() {
    let mut e = parse("$3");
    shift_top(&mut e, 1, 0);
    assert_eq!(show(&e), "$4");
}

#[test]
fn shift_zero_is_noop() {
    let mut e = parse("(+ $0 (lam $1))");
    let before = show(&e);
    shift_top(&mut e, 0, 0);
    assert_eq!(show(&e), before);
}

#[test]
fn lam_stops_shift_at_bound_var() {
    // `(lam $0)` — the `$0` is bound by the lam, not free. Shift is a no-op.
    let mut e = parse("(lam $0)");
    shift_top(&mut e, 5, 0);
    assert_eq!(show(&e), "(lam $0)");
}

#[test]
fn lam_shifts_only_truly_free_indices() {
    // `(lam $1)` — the `$1` is free (one beyond the lam's binding). Shifting
    // by 1 turns it into `$2`.
    let mut e = parse("(lam $1)");
    shift_top(&mut e, 1, 0);
    assert_eq!(show(&e), "(lam $2)");
}

#[test]
fn nested_lams_track_depth() {
    // `(lam (lam (+ $0 $1 $2)))`: under two lams, `$0` and `$1` are bound, `$2`
    // is free. Shift by 3 should leave `$0`/`$1` and turn `$2` into `$5`.
    let mut e = parse("(lam (lam (+ $0 $1 $2)))");
    shift_top(&mut e, 3, 0);
    assert_eq!(show(&e), "(lam (lam (+ $0 $1 $5)))");
}

#[test]
fn initial_depth_treats_outer_indices_as_bound() {
    // `(+ $0 $1)` viewed as if it sits under one outer binder: `$0` is bound
    // by that outer binder, `$1` is free. Shifting by 2 with initial_depth=1
    // leaves `$0` alone and turns `$1` into `$3`.
    let mut e = parse("(+ $0 $1)");
    shift_top(&mut e, 2, 1);
    assert_eq!(show(&e), "(+ $0 $3)");
}

#[test]
fn negative_shift_is_allowed_when_safe() {
    // `(lam $5)`: the inner $5 is free (depth 1, 5 ≥ 1). Shift by -2 → $3.
    let mut e = parse("(lam $5)");
    shift_top(&mut e, -2, 0);
    assert_eq!(show(&e), "(lam $3)");
}

#[test]
#[should_panic(expected = "negative index")]
fn negative_shift_panics_when_underflow() {
    // `$0` shifted by -1 would yield `$-1` — must panic.
    let mut e = parse("$0");
    shift_top(&mut e, -1, 0);
}

#[test]
fn non_var_nodes_are_untouched() {
    // No DB vars at all — shift is a no-op even with by != 0.
    let mut e = parse("(+ 1 (* 2 3))");
    let before = show(&e);
    shift_top(&mut e, 7, 0);
    assert_eq!(show(&e), before);
}
