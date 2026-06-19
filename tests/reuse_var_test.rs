//! Tests that best-first search can reuse one metavariable more than once
//! within a single abstraction — both across sibling argument positions and
//! across nested structure.
//!
//! A k-occurrence metavar is built by `k-1` `reuse` actions, each merging two
//! slots. These pins guard the reuse / canonical-ordering machinery: a
//! regression that dropped repeated-var abstractions (or stopped merging after
//! one reuse) would change the found pattern and fail here. Uses best-first
//! (deterministic) with tiny inline corpora so the expected pattern is exact.
use clap::Parser;
use egg_stitch::{
    Args,
    lang::{Op, OpChildren, OpChildrenLanguage, StitchEgraph, StitchLanguage},
    multiple_step_search,
    shared::SharedData,
};

/// Builds a `programs`-rooted egraph from flat op-children s-expressions.
fn load<L: StitchLanguage>(programs: &[&str]) -> (StitchEgraph<L>, egg::Id) {
    let mut egraph: StitchEgraph<L> = egg::EGraph::default();
    let ids: Vec<egg::Id> = programs
        .iter()
        .map(|s| {
            let expr: egg::RecExpr<L> = s.parse().unwrap();
            egraph.add_expr(&expr)
        })
        .collect();
    let root = egraph.add(L::from_op("programs", ids).unwrap());
    egraph.rebuild();
    (egraph, root)
}

/// Runs one best-first abstraction over `programs` and returns `library[0].pattern`.
fn first_pattern(programs: &[&str]) -> String {
    let (eg, root) = load::<OpChildrenLanguage>(programs);
    let args = Args::parse_from(["egg-stitch", "--search", "best-first", "--num-steps", "5000", "--max-arity", "2", "--num-abstractions", "1"]);
    let (library, ..) = multiple_step_search::<OpChildren, Op>(SharedData::new(eg, root), &args);
    assert_eq!(library.len(), 1, "expected exactly one abstraction, got {}", library.len());
    library[0].pattern.clone()
}

/// One metavar reused across three sibling argument positions (`?#0` thrice).
#[test]
fn reuse_var_thrice_siblings() {
    let programs = &["(f a a a)", "(f b b b)", "(f c c c)", "(f d d d)", "(f e e e)", "(f g g g)"];
    assert_eq!(first_pattern(programs), "fn_0: (f ?#0 ?#0 ?#0)");
}

/// One metavar reused three times with one occurrence nested inside `(g ...)`,
/// so the merges span different structural depths — not just flat siblings.
#[test]
fn reuse_var_across_nesting() {
    let programs = &["(f a (g a) a)", "(f b (g b) b)", "(f c (g c) c)", "(f d (g d) d)", "(f e (g e) e)", "(f h (g h) h)"];
    assert_eq!(first_pattern(programs), "fn_0: (f ?#0 (g ?#0) ?#0)");
}
