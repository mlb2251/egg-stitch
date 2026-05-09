use clap::Parser;
use egg_stitch::{
    Args, io,
    lang::{Op, OpChildren, Weights},
    pattern::PatternRecExpr,
    smc,
};

const INPUT: &str = "data/domains/cogsci/dials.json";
const RULES: &str = "../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites";

fn fixtures_present() -> bool {
    std::path::Path::new(INPUT).exists() && std::path::Path::new(RULES).exists()
}

fn run(args: &Args) -> smc::SmcResult<OpChildren, Op> {
    let (egraph, root, _, _) = io::load_egraph(&args.input, args.rules.as_deref(), Weights::default());
    smc::smc(egraph, root, args)
}

fn assert_best_matches_follow(result: &smc::SmcResult<OpChildren, Op>, follow_str: &str) {
    let follow: PatternRecExpr<OpChildren, Op> = follow_str.parse().expect("parse follow");
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
