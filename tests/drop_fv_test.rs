//! Test for the cost optimizer's "drop a free variable from a metavariable"
//! path — see issue #116. When most matches' captured arg is closed but a
//! minority brings a pattern-internal free variable along, the optimizer
//! should compare both candidates (`S_0 = {}` vs `S_0 = {0}`) and pick
//! whichever total cost is lower. The classic case below has 3 closed
//! matches and 1 fv-bearing match; dropping the fv keeps the 3 closed
//! matches and leaves the 4th uncompressed.

use clap::Parser;
use egg::FromOp;
use egg_stitch::{
    Args,
    lang::{LambdaCalc, LambdaCalcLanguage, Op, OpDB, StitchEgraph, StitchLanguage},
    multiple_step_search,
    shared::SharedData,
};

const PROGRAMS: &[&str] = &["(lam (lam (+ A)))", "(lam (lam (+ B)))", "(lam (lam (+ C)))", "(lam (lam (+ $0)))"];

type Lang = LambdaCalcLanguage<OpDB<Op>>;

fn load() -> (StitchEgraph<Lang>, egg::Id) {
    let mut egraph: StitchEgraph<Lang> = egg::EGraph::default();
    let ids: Vec<egg::Id> = PROGRAMS
        .iter()
        .map(|s| {
            let expr: egg::RecExpr<Lang> = <Lang as StitchLanguage>::parse_program(s).unwrap();
            egraph.add_expr(&expr)
        })
        .collect();
    let root = egraph.add(<Lang as FromOp>::from_op("programs", ids).unwrap());
    egraph.rebuild();
    (egraph, root)
}

/// End-to-end: best-first search over the issue's corpus picks the
/// abstraction that drops `$0` from `?#0`, giving empty-fv ho-arity and
/// rewriting the three constant programs while leaving the `$0` program
/// uncompressed.
#[test]
fn end_to_end_drops_fv_when_beneficial() {
    let (egraph, root) = load();
    let args = Args::parse_from(["egg-stitch", "--language", "lambda-calc", "--search", "best-first", "--num-steps", "500", "--max-arity", "1", "--num-abstractions", "1"]);
    let (library, _orig, _final_cost, final_rewritten) = multiple_step_search::<LambdaCalc, OpDB<Op>>(SharedData::new(egraph, root), &args);
    assert_eq!(library.len(), 1, "expected one abstraction; got {} entries", library.len());
    let rewritten = final_rewritten.expect("rewritten corpus");
    // The fourth program — the only one whose body references `$0` — should
    // stay in its original form, since it can't fit the dropped-fv
    // abstraction.
    assert_eq!(rewritten[3], "(lam (lam (+ $0)))", "$0-bearing program should stay un-rewritten; got rewritten = {:?}", rewritten);
    // The other three must be rewritten via fn_0.
    for (i, r) in rewritten.iter().take(3).enumerate() {
        assert!(r.contains("fn_0"), "program {} should be rewritten via fn_0; got {:?}", i, r);
    }
}
