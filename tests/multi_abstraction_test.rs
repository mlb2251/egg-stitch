/// Tests for the multi-abstraction loop in `run_abstractions`.
///
/// Uses best-first search (deterministic) with a fixed inline dataset so that
/// expected values can be hardcoded exactly.
///
/// Dataset: 4 programs of the form (+ (f (g (h x)) (g (h y))) (f (g (h z)) (g (h w)))).
/// With max-arity=2, best-first finds (f (g (h ?#0)) (g (h ?#1))) as the first
/// abstraction (8 matches across 4 programs), leaving (+ (fn_0 ..) (fn_0 ..)) programs
/// for the second round.
use clap::Parser;
use egg_stitch::{
    Args,
    lang::{OpChildrenLanguage, StitchEgraph, StitchLanguage},
    multiple_step_search,
};

const PROGRAMS: &[&str] = &["(+ (f (g (h a)) (g (h b))) 2 2 2)", "(+ (f (g (a e)) (g (b f))) 3 3 3)", "(+ (f (g (e i)) (g (f j))) 4 4 4)", "(* (f (g (k m)) (g (l n))) 5)"];

fn load<L: StitchLanguage>() -> (StitchEgraph<L>, egg::Id) {
    let mut egraph: StitchEgraph<L> = egg::EGraph::default();
    let ids: Vec<egg::Id> = PROGRAMS
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

fn args(num_steps: usize, num_abstractions: usize) -> Args {
    Args::parse_from(["egg-stitch", "--search", "best-first", "--num-steps", &num_steps.to_string(), "--max-arity", "2", "--num-abstractions", &num_abstractions.to_string()])
}

const FIRST_REWRITTEN: &[&str] = &["(+ (fn_0 (h a) (h b)) 2 2 2)", "(+ (fn_0 (a e) (b f)) 3 3 3)", "(+ (fn_0 (e i) (f j)) 4 4 4)", "(* (fn_0 (k m) (l n)) 5)"];

/// Baseline: num_abstractions=0 gives an empty library.
#[test]
fn zero_abstractions() {
    let (eg, root) = load::<OpChildrenLanguage>();
    let (library, _original_size, final_cost) = multiple_step_search(eg, root, &args(100, 0));
    assert!(library.is_empty());
    assert!(final_cost.is_none());
}

/// Two-abstraction run: second search runs on the rewritten corpus (+ (fn_0 ..) (fn_0 ..)).
/// With max-arity=2 the arity-4 outer pattern is out of reach, so the best second
/// abstraction is the leaf `a` (appears once in the first program).
#[test]
fn two_abstractions() {
    let (eg, root) = load::<OpChildrenLanguage>();
    let (library, original_size, final_cost) = multiple_step_search(eg, root, &args(500, 2));

    println!("Abstractions found:");
    for abs in &library {
        println!("Pattern: {}, Arity: {}, Matches: {}", abs.pattern, abs.arity, abs.num_matches);
    }

    assert_eq!(original_size, 43);
    assert_eq!(final_cost, Some(34));
    assert_eq!(library.len(), 2);

    let first = &library[0];
    assert_eq!(first.pattern, "fn_0: (f (g ?#0) (g ?#1))");
    assert_eq!(first.rewritten_programs, FIRST_REWRITTEN);

    // Second search ran on the rewritten corpus; with max-arity=2 and flat (fn_0 x y)
    // programs the best it can find is the leaf `a`.
    let second = &library[1];
    assert_eq!(second.pattern, "fn_1: (+ ?#0 ?#1 ?#1 ?#1)");
    assert_eq!(second.arity, 2);
    assert_eq!(second.pattern_size, 5);
    assert_eq!(second.num_matches, 3);
}
