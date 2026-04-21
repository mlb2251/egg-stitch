use egg::Id;
use egg_stitch::lang::{Op, StitchEgraph, StitchLang};
use egg_stitch::search::{SearchState, SharedSearchData, compute_usage_counts};

fn build(programs: &[&str]) -> (StitchEgraph, Id) {
    let mut eg: StitchEgraph = egg::EGraph::default();
    let ids: Vec<Id> = programs.iter().map(|s| eg.add_expr(&s.parse().unwrap())).collect();
    let root = eg.add(StitchLang { op: Op::Sym(egg::Symbol::from("programs")), children: ids });
    eg.rebuild();
    (eg, root)
}

fn shared_from(eg: StitchEgraph, root: Id) -> SharedSearchData {
    let usage_counts = compute_usage_counts(&eg, root);
    SharedSearchData { egraph: eg, root, follow: None, weight_by_usage: false, usage_counts, p_reuse: 0.0, check_slow: false }
}

/// Open e-classes (non-empty fv) are valid match roots — `$n` is just an opaque 0-ary op
/// as far as the pattern search is concerned. In a corpus of `(lam $0)`, the body e-class
/// for `$0` has fv={0} but is still a legitimate match.
#[test]
fn identity_matches_include_open_eclasses() {
    let (eg, root) = build(&["(lam $0)"]);
    let shared = shared_from(eg, root);
    let s = SearchState::new(&shared);
    // Expect at least one open e-class in the initial matches (the `$0` class itself).
    let has_open = s.matches.iter().any(|m| !shared.egraph[m.root_eclass].data.fv.is_empty());
    assert!(has_open, "open e-classes should be valid initial matches");
}

/// Expanding `?#0` with the target enode `$0` is allowed even at pattern depth 0.
/// `$0` is treated as an opaque 0-ary op; the pattern just matches e-classes holding `$0`.
#[test]
fn expand_with_free_var_is_allowed() {
    let (eg, root) = build(&["(lam $0)", "(lam (lam $1))"]);
    let shared = shared_from(eg, root);
    let mut s = SearchState::new(&shared);
    let target = StitchLang { op: Op::Var(0), children: vec![] };
    s.expand(0, &target, &shared);
    assert_eq!(s.pattern.to_string(), "$0");
    assert!(!s.matches.is_empty(), "`$0` is a valid pattern; e-classes for `$0` should still match");
}

/// End-to-end: `(lam $0)` can be discovered as a pattern.
#[test]
fn can_discover_identity_lambda_pattern() {
    let (eg, root) = build(&["(lam $0)", "(lam $0)", "(+ 1 2)"]);
    let shared = shared_from(eg, root);
    let mut s = SearchState::new(&shared);
    let lam = StitchLang { op: Op::Lam, children: vec![Id::from(0)] };
    s.expand(0, &lam, &shared);
    assert_eq!(s.pattern.to_string(), "(lam ?#0)");
    assert_eq!(s.pattern.var_depth, vec![1]);
    assert!(!s.matches.is_empty());
    let v0 = StitchLang { op: Op::Var(0), children: vec![] };
    s.expand(0, &v0, &shared);
    assert_eq!(s.pattern.to_string(), "(lam $0)");
    assert!(!s.matches.is_empty(), "(lam $0) should match the two identity lambdas in the corpus");
}
