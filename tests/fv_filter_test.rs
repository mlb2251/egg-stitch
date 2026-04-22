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

/// Initial matches exclude open e-classes: at `?#0` (depth 0), any free var in a candidate
/// subterm would be dangling at the call site of the emitted λ-abstraction — `apply_abstraction`
/// would reject those matches, so we prune them upfront to keep cost estimation honest.
#[test]
fn identity_matches_exclude_open_eclasses() {
    let (eg, root) = build(&["(lam $0)"]);
    let shared = shared_from(eg, root);
    let s = SearchState::new(&shared);
    for m in &s.matches {
        assert!(shared.egraph[m.root_eclass].data.fv.is_empty(), "open e-class {:?} should not be an initial match", m.root_eclass);
    }
    // The closed `(lam $0)` e-class is still a valid match root.
    assert!(!s.matches.is_empty());
}

/// Expanding `?#0` at depth 0 with the target `$0` produces the pattern `$0`, whose match
/// sites would be open e-classes for `$0`. Since those were already filtered from
/// `identity_matches`, the resulting match set is empty.
#[test]
fn expand_to_bare_var_leaves_no_matches() {
    let (eg, root) = build(&["(lam $0)", "(lam (lam $1))"]);
    let shared = shared_from(eg, root);
    let mut s = SearchState::new(&shared);
    let target = StitchLang { op: Op::Var(0), children: vec![] };
    s.expand(0, &target, &shared);
    assert_eq!(s.pattern.to_string(), "$0");
    assert!(s.matches.is_empty(), "`$0` as a depth-0 pattern cannot bind `$0`; no sound matches");
}

/// Pattern `(lam ?#0)` puts the hole at depth 1, so a match arg with `fv ⊆ {0}` is OK —
/// the single pattern lam binds the `$0`. Further expansion to `(lam $0)` is still sound.
#[test]
fn lambda_wrapped_pattern_accepts_bound_var() {
    let (eg, root) = build(&["(lam $0)", "(lam $0)", "(+ 1 2)"]);
    let shared = shared_from(eg, root);
    let mut s = SearchState::new(&shared);
    assert_eq!(s.pattern.var_depth, vec![0]);
    let lam = StitchLang { op: Op::Lam, children: vec![Id::from(0)] };
    s.expand(0, &lam, &shared);
    assert_eq!(s.pattern.to_string(), "(lam ?#0)");
    assert_eq!(s.pattern.var_depth, vec![1]);
    assert!(!s.matches.is_empty(), "should still have matches after expanding to `lam`");
    let v0 = StitchLang { op: Op::Var(0), children: vec![] };
    s.expand(0, &v0, &shared);
    assert_eq!(s.pattern.to_string(), "(lam $0)");
    assert!(!s.matches.is_empty(), "(lam $0) should match the identity lambdas in the corpus");
}
