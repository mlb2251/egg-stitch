//! Direct tests for `LambdaCalc::parse_follow_pattern`.
//!
//! The follow pattern is whatever `display_pattern_as_lambda` / `display_recexpr`
//! would emit for an in-progress lambda-calc pattern: flat-form sexps that may
//! have a `?#k` var head (e.g. `(?#0 a b)`). egg's stock pattern parser rejects
//! both list-headed shapes and var-headed apps, so `LambdaCalc` overrides
//! `parse_follow_pattern` to go through `parse_program` at `OpWithVar<O>`,
//! which routes `?#k` atoms to `Var(v)`.

use egg::RecExpr;
use egg_stitch::lang::{LambdaCalc, LambdaCalcLanguage, LanguageFamily, Op, OpWithVar, StitchLanguage};

type LangPat = LambdaCalcLanguage<OpWithVar<Op>>;

fn round_trip(s: &str) -> String {
    let parsed = LambdaCalc::parse_follow_pattern::<Op>(s).unwrap();
    let rec: RecExpr<LangPat> = parsed.into();
    LangPat::display_recexpr(&rec)
}

#[test]
fn flat_app_no_vars() {
    assert_eq!(round_trip("(f a b c)"), "(f a b c)");
}

#[test]
fn var_in_arg_position() {
    assert_eq!(round_trip("(f ?#0 b)"), "(f ?#0 b)");
}

#[test]
fn var_headed_application() {
    // `(?#0 a b)` — head is a pattern var. egg's stock `RecExpr` parser
    // refuses this because `?#0` isn't a registered op name; lambda-calc's
    // override handles it via `parse_program` + `OpWithVar::from_name`.
    assert_eq!(round_trip("(?#0 a b)"), "(?#0 a b)");
}

#[test]
fn multiple_vars() {
    assert_eq!(round_trip("(f ?#0 (g ?#1) ?#0)"), "(f ?#0 (g ?#1) ?#0)");
}

#[test]
fn var_inside_lam() {
    assert_eq!(round_trip("(lam (?#0 $0))"), "(lam (?#0 $0))");
}

#[test]
fn nested_lam_with_var_head() {
    assert_eq!(round_trip("(lam (lam (?#0 $0 $1)))"), "(lam (lam (?#0 $0 $1)))");
}

#[test]
fn list_headed_application_curries() {
    // `((f x) y)` — egg's stock parser rejects list-in-head; the override
    // curries it into `App(App(f, x), y)` and the unappified display flattens
    // back to `(f x y)`.
    assert_eq!(round_trip("((f x) y)"), "(f x y)");
}

#[test]
fn bare_var() {
    assert_eq!(round_trip("?#0"), "?#0");
}

#[test]
fn invalid_lam_arity_errors() {
    // Errors bubble through as `Err`, not panic — `setup_search` panics on
    // the outer side, but the parser itself must surface a Result.
    assert!(LambdaCalc::parse_follow_pattern::<Op>("(lam)").is_err());
    assert!(LambdaCalc::parse_follow_pattern::<Op>("(lam x y)").is_err());
}
