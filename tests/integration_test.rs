use clap::Parser;
use egg_stitch::{
    Args, io,
    lang::{LambdaCalc, LanguageFamily, Op, OpChildren, OpDB, Weights},
    pattern::PatternRecExpr,
    smc,
};
use rand::SeedableRng;
use rand::rngs::StdRng;

const INPUT: &str = "data/domains/cogsci/dials.json";
const RULES: &str = "../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites";

fn fixtures_present() -> bool {
    std::path::Path::new(INPUT).exists() && std::path::Path::new(RULES).exists()
}

fn run(args: &Args) -> smc::SmcResult<OpChildren, Op> {
    let (data, _, _) = io::load_egraph::<OpChildren, Op>(&args.input, args.rules.as_deref(), Weights::default());
    let mut rng = StdRng::seed_from_u64(args.seed.unwrap_or(0));
    smc::smc(data, args, &mut rng)
}

fn run_lambda_calc(args: &Args) -> smc::SmcResult<LambdaCalc, OpDB<Op>> {
    let (data, _, _) = io::load_egraph::<LambdaCalc, OpDB<Op>>(&args.input, args.rules.as_deref(), Weights::default());
    let mut rng = StdRng::seed_from_u64(args.seed.unwrap_or(0));
    smc::smc(data, args, &mut rng)
}

fn assert_best_matches_follow(result: &smc::SmcResult<OpChildren, Op>, follow_str: &str) {
    let follow: PatternRecExpr<OpChildren, Op> = follow_str.parse().expect("parse follow");
    let (cost, best) = result.best.as_ref().expect("smc should produce a best pattern");
    assert!(best.matches_follow(&follow), "best pattern (cost={}, pattern={}) should match follow {}", cost, best.pattern, follow_str,);
}

/// Lambda-calc variant: parses through `LambdaCalc::parse_follow_pattern` so
/// var-headed apps (`(?#0 $0)`) and list-headed currying are handled the same
/// way `setup_search` does it.
fn assert_best_matches_follow_lambda(result: &smc::SmcResult<LambdaCalc, OpDB<Op>>, follow_str: &str) {
    let follow = LambdaCalc::parse_follow_pattern::<OpDB<Op>>(follow_str).expect("parse follow");
    let (cost, best) = result.best.as_ref().expect("smc should produce a best pattern");
    assert!(best.matches_follow(&follow), "best pattern (cost={}, pattern={}) should match follow {}", cost, best.pattern, follow_str,);
}

const DIALS_FULL_FOLLOW: &str = "(T (T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0)) (M 1 0 (* ?#0 (* 0.5 (cos (/ pi 4)))) (* ?#0 (* 0.5 (sin (/ pi 4))))))";

/// Full follow baseline — needs high temperature.
#[test]
fn follow_dials_full_baseline() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "1000", "--num-particles", "1000", "--temperature", "1000", "--follow", DIALS_FULL_FOLLOW, "--max-arity", "2"]);
    let result = run(&args);
    assert_best_matches_follow(&result, DIALS_FULL_FOLLOW);
}

/// Shallow follow with no variables — fast.
#[test]
fn follow_shallow_no_placeholders() {
    if !fixtures_present() {
        return;
    }
    let follow = "(T l (M 1 0 -0.5 0))";
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "30", "--num-particles", "200", "--follow", follow, "--max-arity", "2"]);
    let result = run(&args);
    assert_best_matches_follow(&result, follow);
}

/// Follow with a `?#0` variable — verifies the search doesn't crash.
#[test]
fn follow_single_placeholder() {
    if !fixtures_present() {
        return;
    }
    let follow = "(T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0))";
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "1000", "--num-particles", "1000", "--temperature", "1000", "--follow", follow, "--max-arity", "2"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// No follow — sanity check.
#[test]
fn no_follow_still_produces_best() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// Verify fast rewrite cost matches slow (egraph-based) cost.
#[test]
fn check_slow_matches_fast() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// Verify fast == slow on dials without rewrite rules (no high-id children).
#[test]
fn check_slow_no_rules() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

const REWRITES_DIR: &str = "../babble/harness/data/benchmark-dsrs";

/// Verify fast == slow across multiple domains with their rewrite rules.
#[test]
fn check_slow_furniture() {
    let input = "data/domains/cogsci/furniture.json";
    let rules = &format!("{}/drawings.furniture.rewrites", REWRITES_DIR);
    if !std::path::Path::new(input).exists() || !std::path::Path::new(rules).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", input, "--rules", rules, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

#[test]
fn check_slow_nuts_bolts() {
    let input = "data/domains/cogsci/nuts-bolts.json";
    let rules = &format!("{}/drawings.nuts-bolts.rewrites", REWRITES_DIR);
    if !std::path::Path::new(input).exists() || !std::path::Path::new(rules).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", input, "--rules", rules, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

#[test]
fn check_slow_wheels() {
    let input = "data/domains/cogsci/wheels.json";
    let rules = &format!("{}/drawings.wheels.rewrites", REWRITES_DIR);
    if !std::path::Path::new(input).exists() || !std::path::Path::new(rules).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", input, "--rules", rules, "--num-steps", "20", "--num-particles", "100", "--max-arity", "2", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// Verify fast == slow with higher arity.
#[test]
fn check_slow_high_arity() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "20", "--num-particles", "100", "--max-arity", "4", "--check-slow"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// Verify fast == slow with higher arity.
#[test]
fn check_slow_high_arity_multi_abstr() {
    if !fixtures_present() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", INPUT, "--rules", RULES, "--num-steps", "20", "--num-particles", "100", "--max-arity", "4", "--check-slow", "--num-abstractions", "2"]);
    let result = run(&args);
    assert!(result.best.is_some());
}

/// Regression: fast path under-counted when a match's wrapped operand
/// resolved to another match-root eclass that itself had a cheaper rewrite.
#[test]
fn check_slow_lambda_calc_fast_slow_mismatch() {
    let input = "data/domains/stitch/lambda-calc-fast-slow-mismatch.json";
    if !std::path::Path::new(input).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", input, "--num-steps", "50", "--num-particles", "20", "--temperature", "1000", "--check-slow", "--language", "lambda-calc", "--seed", "145514431571737541"]);
    let _ = run_lambda_calc(&args);
}

/// Regression: fast path needs to re-sum non-match parent eclasses (the
/// Programs root above a match-root) after a match's rewrite shrinks the
/// child's size.
#[test]
fn check_slow_intermediate_propagation() {
    let input = "data/domains/stitch/intermediate-propagation.json";
    if !std::path::Path::new(input).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", input, "--num-steps", "50", "--num-particles", "20", "--temperature", "1000", "--check-slow", "--language", "lambda-calc", "--seed", "888315200261588942"]);
    let _ = run_lambda_calc(&args);
}

/// Exercises the lambda-calc fast/slow check against real physics corpora.
/// One test per file so they parallelize and failures point at a specific input.
fn check_slow_physics(name: &str) {
    let input = format!("data/domains/physics/{}", name);
    if !std::path::Path::new(&input).exists() {
        return;
    }
    let args = Args::parse_from(["egg-stitch", "--input", &input, "--num-steps", "100", "--num-particles", "10000", "--temperature", "1000", "--language", "lambda-calc", "--check-slow"]);
    let _ = run_lambda_calc(&args);
}

#[test]
fn check_slow_physics_bench000_it0() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.05.46__bench000_it0.json");
}
#[test]
fn check_slow_physics_bench001_it1() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.05.46__bench001_it1.json");
}
#[test]
fn check_slow_physics_bench002_it2() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.05.46__bench002_it2.json");
}
#[test]
fn check_slow_physics_bench003_it3() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.05.46__bench003_it3.json");
}
#[test]
fn check_slow_physics_bench004_it4() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.05.46__bench004_it4.json");
}
#[test]
fn check_slow_physics_18_09_34_bench000() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.09.34__bench000_it0.json");
}
#[test]
fn check_slow_physics_18_09_34_bench001() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.09.34__bench001_it1.json");
}
#[test]
fn check_slow_physics_18_09_34_bench002() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.09.34__bench002_it2.json");
}
#[test]
fn check_slow_physics_18_09_34_bench003() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.09.34__bench003_it3.json");
}
#[test]
fn check_slow_physics_18_09_34_bench004() {
    check_slow_physics("scientific_unsolved_4h_ellisk_2019-07-20T18.09.34__bench004_it4.json");
}

// --- End-to-end --follow tests in the lambda-calc domain ---
//
// These exercise the LambdaCalc `parse_follow_pattern` override end-to-end:
// the follow strings use shapes (flat n-ary apps, var-headed apps) that egg's
// stock `RecExpr` parser rejects. parse_follow_pattern routes them through
// `parse_program` at `OpWithVar<O>`.

/// Follow string `(lam (app ?#0 (app ?#0 empty)))` uses flat n-ary form for
/// `app` — egg's stock parser rejects `app` as an unknown op, so this only
/// parses via the lambda-calc override. The follow is the abstraction SMC
/// deterministically finds on `hof.json` at the default seed, so the prefix
/// check passes.
#[test]
fn follow_lambda_calc_flat_nary_app() {
    let input = "data/domains/stitch/hof.json";
    if !std::path::Path::new(input).exists() {
        return;
    }
    let follow = "(lam (app ?#0 (app ?#0 empty)))";
    let args = Args::parse_from(["egg-stitch", "--input", input, "--num-steps", "200", "--num-particles", "500", "--temperature", "1000", "--follow", follow, "--max-arity", "2", "--language", "lambda-calc"]);
    let result = run_lambda_calc(&args);
    assert_best_matches_follow_lambda(&result, follow);
}

/// Smoke test for a var-headed follow string `(?#0 (lam (?#0 ?#0)))` — `?#0`
/// in head position is the shape only the lambda-calc override accepts.
/// SMC may or may not reach the target on this tiny corpus; the test just
/// guarantees parse + smc loop integration doesn't crash and any best found
/// is a valid prefix.
#[test]
fn follow_lambda_calc_var_headed_smoke() {
    let input = "data/domains/stitch/simple2.json";
    if !std::path::Path::new(input).exists() {
        return;
    }
    let follow = "(?#0 (lam (?#0 ?#0)))";
    let args = Args::parse_from(["egg-stitch", "--input", input, "--num-steps", "100", "--num-particles", "200", "--temperature", "1000", "--follow", follow, "--max-arity", "2", "--language", "lambda-calc"]);
    let result = run_lambda_calc(&args);
    if result.best.is_some() {
        assert_best_matches_follow_lambda(&result, follow);
    }
}
