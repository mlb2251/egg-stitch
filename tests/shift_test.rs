use egg::ENodeOrVar;
use egg_stitch::lang::StitchLang;
use egg_stitch::revexpr::{RevExpr, shift_free};

fn parse(s: &str) -> RevExpr<ENodeOrVar<StitchLang>> {
    s.parse().unwrap()
}

fn render(e: &RevExpr<ENodeOrVar<StitchLang>>) -> String {
    e.to_string()
}

#[test]
fn shift_bare_var_up() {
    let mut e = parse("$0");
    shift_free(&mut e, egg::Id::from(0), 1, 0);
    assert_eq!(render(&e), "$1");
}

#[test]
fn shift_under_lam_does_not_touch_bound() {
    let mut e = parse("(lam $0)");
    shift_free(&mut e, egg::Id::from(0), 5, 0);
    // $0 is bound by the lam, so unchanged.
    assert_eq!(render(&e), "(lam $0)");
}

#[test]
fn shift_under_lam_touches_free() {
    let mut e = parse("(lam $1)");
    shift_free(&mut e, egg::Id::from(0), 2, 0);
    // $1 inside lam = free var 0 outside; shifted to free 2 → $3 inside lam.
    assert_eq!(render(&e), "(lam $3)");
}

#[test]
fn shift_mixed() {
    let mut e = parse("(+ $0 (lam $2))");
    shift_free(&mut e, egg::Id::from(0), 1, 0);
    // Top-level $0 → $1. Inside lam, $2 is free (depth=1, 2>=1), shifts to $3.
    assert_eq!(render(&e), "(+ $1 (lam $3))");
}

#[test]
fn meta_vars_untouched() {
    let mut e = parse("(+ ?#0 $0)");
    shift_free(&mut e, egg::Id::from(0), 1, 0);
    assert_eq!(render(&e), "(+ ?#0 $1)");
}

#[test]
fn initial_depth_treats_root_as_under_binders() {
    let mut e = parse("$0");
    // If the splicing point already sits under 1 binder, $0 is bound (< 1), don't shift.
    shift_free(&mut e, egg::Id::from(0), 5, 1);
    assert_eq!(render(&e), "$0");
}
