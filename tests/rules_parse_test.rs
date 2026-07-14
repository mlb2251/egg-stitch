//! Tests for the rewrite-rule parser's arrow handling (`io::parse`).
//!
//! A rule uses `=>` for a one-directional rewrite or `<=>` for a bidirectional
//! equivalence, which expands to the forward rule plus a `<name>-rev` rule with
//! the sides swapped.
//!
//! Regression: `<=>` previously "worked" only by accident — `split_once("=>")`
//! matched the `=>` inside `<=>`, leaving a stray `<` on the lhs that the
//! pattern parser silently dropped, so a `<=>` rule quietly became a
//! forward-only `=>` (and the reverse direction was never created). The parser
//! now splits on `<=>` first and emits both directions explicitly.

use egg_stitch::{
    io,
    lang::{Op, OpChildrenLanguage, StitchAnalysis},
};

type Lang = OpChildrenLanguage<Op>;

fn rule_names(src: &str) -> Vec<String> {
    io::parse::<Lang, StitchAnalysis>(src).expect("parse rules").iter().map(|r| r.name.to_string()).collect()
}

/// `<=>` expands to the forward rule plus a `<name>-rev` reverse rule.
#[test]
fn bidirectional_arrow_expands_to_forward_and_reverse() {
    let names = rule_names("dup: (+ 2 2) <=> 4");
    assert_eq!(names.len(), 2, "<=> should expand to two rules; got {names:?}");
    assert!(names.contains(&"dup".to_string()), "missing forward rule; got {names:?}");
    assert!(names.contains(&"dup-rev".to_string()), "missing reverse rule; got {names:?}");
}

/// `=>` stays a single one-directional rule.
#[test]
fn unidirectional_arrow_is_a_single_rule() {
    let names = rule_names("fwd: (+ 2 2) => 4");
    assert_eq!(names, vec!["fwd".to_string()]);
}

/// Multiple rules, mixed arrows, with comments and blank lines.
#[test]
fn mixed_arrows_and_comments() {
    let src = "\
        // a comment line\n\
        a: (f ?x) => ?x\n\
        \n\
        b: (g ?x) <=> ?x  // trailing comment\n";
    let names = rule_names(src);
    assert_eq!(names.len(), 3, "expected a, b, b-rev; got {names:?}");
    assert!(names.contains(&"a".to_string()));
    assert!(names.contains(&"b".to_string()));
    assert!(names.contains(&"b-rev".to_string()));
}
