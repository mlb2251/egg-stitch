//! Regression tests for reusing one metavariable across a *nesting boundary*.
//!
//! Building a k-occurrence metavar takes k-1 `reuse` merges. Best-first's
//! reuse canonical-ordering used to stale the merged slot *and* every slot
//! below it on each reuse, while `expand` had already staled the pre-existing
//! siblings. So once a variable's occurrences were spread across an expansion
//! (one nested inside `(g …)`, one a sibling), the final cross-boundary merge
//! saw two non-reusable slots and was skipped — best-first stalled at a
//! partial reuse (`(f (g ?#0 ?#0) ?#1)`) or, for a body with no concrete head,
//! found nothing, even though SMC found the full abstraction.
//!
//! The fix keeps the merged (kept) slot reusable, so the chain can complete.
//! These pin the now-reachable patterns; each one fails (partial reuse / empty)
//! without the fix. Best-first is deterministic, so the expected pattern is
//! exact.
use clap::Parser;
use egg::FromOp;
use egg_stitch::{
    Args,
    lang::{LambdaCalc, LambdaCalcLanguage, Op, OpChildren, OpChildrenLanguage, OpDB, StitchEgraph, StitchLanguage},
    multiple_step_search,
    shared::SharedData,
};

/// One best-first abstraction over flat op-children programs; returns `library[0].pattern`.
fn op_children_pattern(programs: &[&str]) -> String {
    let mut egraph: StitchEgraph<OpChildrenLanguage<Op>> = egg::EGraph::default();
    let ids: Vec<egg::Id> = programs.iter().map(|s| egraph.add_expr(&s.parse().unwrap())).collect();
    let root = egraph.add(OpChildrenLanguage::<Op>::from_op("programs", ids).unwrap());
    egraph.rebuild();
    let args = Args::parse_from(["egg-stitch", "--search", "best-first", "--num-steps", "20000", "--max-arity", "2", "--num-abstractions", "1"]);
    let (library, ..) = multiple_step_search::<OpChildren, Op>(SharedData::new(egraph, root), &args);
    assert_eq!(library.len(), 1, "expected one abstraction, got {}", library.len());
    library[0].pattern.clone()
}

/// Same, over curried lambda-calc programs (so reuse spans `App`-nesting).
fn lambda_calc_pattern(programs: &[&str]) -> String {
    type Lang = LambdaCalcLanguage<OpDB<Op>>;
    let mut egraph: StitchEgraph<Lang> = egg::EGraph::default();
    let ids: Vec<egg::Id> = programs.iter().map(|s| egraph.add_expr(&<Lang as StitchLanguage>::parse_program(s).unwrap())).collect();
    let root = egraph.add(<Lang as FromOp>::from_op("programs", ids).unwrap());
    egraph.rebuild();
    let args = Args::parse_from(["egg-stitch", "--language", "lambda-calc", "--search", "best-first", "--num-steps", "20000", "--max-arity", "2", "--num-abstractions", "1"]);
    let (library, ..) = multiple_step_search::<LambdaCalc, OpDB<Op>>(SharedData::new(egraph, root), &args);
    assert_eq!(library.len(), 1, "expected one abstraction, got {}", library.len());
    library[0].pattern.clone()
}

/// `?#0` twice inside `(g …)` plus once as a sibling of `g`. Without the fix
/// best-first stalls at `(f (g ?#0 ?#0) ?#1)`.
#[test]
fn reuse_nested_pair_and_sibling() {
    let programs = &["(f (g a a) a)", "(f (g b b) b)", "(f (g c c) c)", "(f (g d d) d)", "(f (g e e) e)", "(f (g h h) h)"];
    assert_eq!(op_children_pattern(programs), "fn_0: (f (g ?#0 ?#0) ?#0)");
}

/// The reused pair is two levels down, inside `(g (h …))`, with a top-level
/// sibling. Without the fix best-first stalls at `(f (g (h ?#0 ?#0)) ?#1)`.
#[test]
fn reuse_deeply_nested_pair_and_sibling() {
    let programs = &["(f (g (h a a)) a)", "(f (g (h b b)) b)", "(f (g (h c c)) c)", "(f (g (h d d)) d)", "(f (g (h e e)) e)", "(f (g (h k k)) k)"];
    assert_eq!(op_children_pattern(programs), "fn_0: (f (g (h ?#0 ?#0)) ?#0)");
}

/// Self-application `(?#0 ?#0 ?#0)` (= `lam x. x x x`): a body with no concrete
/// node, the metavar in both operator and argument positions. Without the fix
/// best-first finds nothing here (empty library); SMC always could.
#[test]
fn reuse_self_application() {
    let programs = &["(a a a)", "(b b b)", "(c c c)", "(d d d)", "(e e e)", "(h h h)"];
    assert_eq!(lambda_calc_pattern(programs), "fn_0: (?#0 ?#0 ?#0)");
}
