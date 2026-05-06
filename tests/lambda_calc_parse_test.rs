//! Tests for `LambdaCalcLanguage::parse_program`.
//!
//! The parser accepts arbitrary s-expressions (including ones with a list in
//! operator position, e.g. `((f x) y)`) and curries them into `App` chains.
//! These tests exercise list-headed forms specifically — egg's default
//! `RecExpr::from_str` rejects them because `from_op` takes a string head.

use egg_stitch::lang::{LambdaCalcLanguage, Op, StitchLanguage};

type Lang = LambdaCalcLanguage<Op>;

fn round_trip(prog: &str) -> String {
    let parsed = Lang::parse_program(prog).unwrap();
    Lang::display_recexpr(&parsed)
}

#[test]
fn flat_application() {
    assert_eq!(round_trip("(f a b c)"), "(f a b c)");
}

#[test]
fn explicit_at_application() {
    // Explicit `(@ f a)` is the binary `App` form. Display unappifies back to
    // flat n-ary, so `(@ (@ f a) b)` should round-trip to `(f a b)`.
    assert_eq!(round_trip("(@ f a)"), "(f a)");
    assert_eq!(round_trip("(@ (@ f a) b)"), "(f a b)");
}

#[test]
fn list_in_operator_position_curries() {
    // egg's default parser rejects head-as-list; ours treats it as
    // `App(App(f, x), y)` and the unappified display flattens to `(f x y)`.
    assert_eq!(round_trip("((f x) y)"), "(f x y)");
}

#[test]
fn deeply_nested_list_head() {
    // `(((g x) y) z)` parses as a fully-curried 3-ary application.
    assert_eq!(round_trip("(((g x) y) z)"), "(g x y z)");
}

#[test]
fn list_head_inside_lam() {
    // Same shape buried inside a lambda body.
    assert_eq!(round_trip("(lam ((f a) b))"), "(lam (f a b))");
}

#[test]
fn list_head_with_lam_arg() {
    // List-headed application where one of the args is itself a lambda.
    assert_eq!(round_trip("((f (lam x)) y)"), "(f (lam x) y)");
}

#[test]
fn list_head_at_top_with_complex_args() {
    // The head's arity-1 sub-application produces a function value that gets
    // applied to two more args. After currying display normalises to flat form.
    assert_eq!(round_trip("((compose f) (compose g) h)"), "(compose f (compose g) h)");
}

#[test]
fn programs_root_preserved() {
    // `(programs …)` is the corpus-wide multi-child root; it shouldn't curry.
    assert_eq!(round_trip("(programs a b c)"), "(programs a b c)");
}

#[test]
fn programs_with_list_headed_body() {
    // Multi-program corpus where some program has a list-headed body. Each
    // program independently curries.
    assert_eq!(round_trip("(programs (f a) ((g x) y))"), "(programs (f a) (g x y))");
}

#[test]
fn nested_lams_with_list_head() {
    // A common shape: `(lam (lam ((f x) y)))`.
    assert_eq!(round_trip("(lam (lam ((f x) y)))"), "(lam (lam (f x y)))");
}

#[test]
fn bare_atom() {
    assert_eq!(round_trip("foo"), "foo");
}

#[test]
fn lam_with_bare_body() {
    assert_eq!(round_trip("(lam x)"), "(lam x)");
}

#[test]
fn invalid_lam_arity_errors() {
    assert!(Lang::parse_program("(lam)").is_err());
    assert!(Lang::parse_program("(lam x y)").is_err());
}

#[test]
fn invalid_at_arity_errors() {
    assert!(Lang::parse_program("(@ f)").is_err());
    assert!(Lang::parse_program("(@ f a b)").is_err());
}

#[test]
fn lambda_alias_for_lam() {
    // Babble's dreamcoder corpus and DSRs use `lambda` (and `λ`) as the binder
    // keyword; we accept either as a synonym for `lam` so those inputs parse
    // verbatim. Display always emits the canonical `lam`.
    assert_eq!(round_trip("(lambda x)"), "(lam x)");
    assert_eq!(round_trip("(λ x)"), "(lam x)");
    assert_eq!(round_trip("(@ (@ map (lambda $0)) xs)"), "(map (lam $0) xs)");
}
