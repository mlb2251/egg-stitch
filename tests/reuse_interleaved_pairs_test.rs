//! Regression test for reusing two *interleaved* metavariable pairs.
//!
//! Companion to `reuse_across_nesting_test.rs`. That bug was a single var
//! reused across a nesting boundary; this one is two independent pairs whose
//! DFS indices interleave as 0,1,2,3, where the wanted abstraction merges
//! {0,2} and {1,3}.
//!
//! Best-first's reuse canonical-ordering stales every slot below `drop_idx`
//! on each merge (keeping only the kept slot reusable). Take
//! `(f (g ?#0 ?#1) ?#2 ?#3)`: `expand` left the `g`-children 0,1 reusable and
//! the siblings 2,3 stale. Whichever pair we merge first stales the surviving
//! reusable slot of the *other* pair, so the second merge sees two stale slots
//! and is skipped — best-first stalls before reaching `(f (g ?#0 ?#1) ?#0 ?#1)`.
//! (`reuse(0,2)` kills slot 1; `reuse(1,3)` kills slot 0 — neither order leaves
//! the other pair a chance.)
//!
//! Best-first is deterministic, so the expected pattern is exact. This test
//! fails without a fix to the canonical-ordering staling.
use clap::Parser;
use egg::FromOp;
use egg_stitch::{
    Args,
    lang::{Op, OpChildren, OpChildrenLanguage, StitchEgraph},
    multiple_step_search,
    shared::SharedData,
};

/// One best-first abstraction over flat op-children programs; returns
/// `library[0].pattern`.
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

/// `(f (g X Y) X Y)`: a pair `X,Y` inside `(g …)` reused as the two siblings of
/// `g`. Building it needs the interleaved merges `reuse(0,2)` and `reuse(1,3)`;
/// without the fix best-first stalls one merge short.
#[test]
fn reuse_interleaved_pairs() {
    let programs = &[
        "(f (g a b) a b)",
        "(f (g c d) c d)",
        "(f (g e h) e h)",
        "(f (g i j) i j)",
        "(f (g k l) k l)",
        "(f (g m n) m n)",
    ];
    assert_eq!(op_children_pattern(programs), "fn_0: (f (g ?#0 ?#1) ?#0 ?#1)");
}
