//! Tests for `follow::binding_as_exact_var`: the per-binding witness that an
//! `unify` result is alpha-equivalent to the follow rather than merely a
//! prefix. Built against `LambdaCalc` since `OpChildren::wrap_pattern_with_db_apps`
//! panics — higher-order display is a lambda-calc concept.

use egg::Id;
use egg_stitch::follow::binding_as_exact_var;
use egg_stitch::lang::{LambdaCalc, LanguageFamily, Op, OpDB};

type Leaf = OpDB<Op>;

fn parse(s: &str) -> egg_stitch::revexpr::RevExpr<<LambdaCalc as LanguageFamily>::Apply<egg_stitch::lang::OpWithVar<Leaf>>> {
    LambdaCalc::parse_follow_pattern::<Leaf>(s).unwrap_or_else(|e| panic!("parse {s:?}: {e:?}"))
}

fn check(follow: &str, vis: &[i32]) -> Option<u32> {
    let fol = parse(follow);
    binding_as_exact_var::<LambdaCalc, Leaf>(&fol, Id::from(0), vis).map(|v| {
        // `egg::Var` displays as `?<name>`; for our `from(k)` vars that's `?#k`.
        v.to_string().trim_start_matches("?#").parse::<u32>().expect("?#k name")
    })
}

#[test]
fn empty_vis_at_bare_var_returns_that_var() {
    assert_eq!(check("?#5", &[]), Some(5));
}

#[test]
fn empty_vis_at_non_var_returns_none() {
    assert_eq!(check("(f a)", &[]), None);
}

#[test]
fn arity_one_eta_wrap_returns_head_var() {
    // `wrap_pattern_with_db_apps(?v, [0])` is `(app ?v $0)`. Lambda-calc's
    // surface form for that is the var-headed app `(?#3 $0)`.
    assert_eq!(check("(?#3 $0)", &[0]), Some(3));
}

#[test]
fn arity_two_eta_wrap_returns_head_var() {
    // `vis=[0, 1]` reverses to `db_args=[1, 0]`, producing
    // `(app (app ?v $1) $0)` → surface `(?#7 $1 $0)`.
    assert_eq!(check("(?#7 $1 $0)", &[0, 1]), Some(7));
}

#[test]
fn wrong_db_index_rejected() {
    // Expected `(?#0 $0)` for vis=[0]; this has `$1` instead.
    assert_eq!(check("(?#0 $1)", &[0]), None);
}

#[test]
fn no_var_at_head_rejected() {
    // Same shape as the eta-wrap but with a concrete op at the head.
    assert_eq!(check("(f $0)", &[0]), None);
}

#[test]
fn extra_structure_under_args_rejected() {
    // The eta-wrap inserts no other Var nodes; if a follow has Vars elsewhere
    // (here, in place of `$0`) it isn't a pure eta-wrap of a Var head.
    assert_eq!(check("(?#0 ?#1)", &[0]), None);
}

#[test]
fn arity_mismatch_rejected() {
    // follow has one db arg but we asked for arity 2.
    assert_eq!(check("(?#0 $0)", &[0, 1]), None);
    // follow has two but we asked for arity 1.
    assert_eq!(check("(?#0 $1 $0)", &[0]), None);
}
