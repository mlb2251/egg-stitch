use std::cmp::min;

use crate::cost::{compute_cost, compute_size};
use crate::follow::validate_follow;
use crate::lang::StitchEgraph;
use crate::logging::apply_follow_constraint;
use crate::search::{SearchState, SharedSearchData};
use rand::Rng;

/// Output of a completed SMC run, surfacing everything the caller needs to
/// build an aggregate `RunResult` for JSON output.
pub struct SmcResult {
    pub best: Option<(usize, SearchState)>,
    pub original_size: usize,
    pub best_found_at: Option<usize>,
    pub num_steps_run: usize,
    pub egraph: StitchEgraph,
}

pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> SmcResult {
    let follow_expr = args.follow.as_deref().map(|s| {
        let parsed = s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e));
        validate_follow(&parsed);
        parsed
    });
    let usage_counts = crate::search::compute_usage_counts(&egraph, root);
    let shared = SharedSearchData {
        egraph,
        p_reuse: args.p_reuse,
        check_slow: args.check_slow,
        weight_by_usage: args.weight_by_usage,
        usage_counts,
        follow: follow_expr,
    };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared), shared.check_slow);
    println!("original size of egraph: {}", original_size);

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs as i64;
    let max_arity = args.max_arity;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;
    let mut steps_run = 0;

    // make a bunch of search states
    let mut search_states: Vec<SearchState> = (0..num_particles).map(|_| SearchState::new(&shared)).collect();

    for step in 0..num_steps {
        for search_state in &mut search_states {
            search_state.expand_random(&shared);
        }

        let costs: Vec<usize> = search_states
            .iter()
            .map(|search_state| compute_cost(&shared.egraph, root, search_state, shared.check_slow))
            .collect();

        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity
                && best_so_far.as_ref().is_none_or(|best| *cost < best.0)
                && shared.follow.as_ref().is_none_or(|f| search_states[i].matches_follow(f))
            {
                println!("[iteration {}] new best: {} {}", step, cost, search_states[i].pattern);
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }


        let mut weights: Vec<f64> = costs.iter().map(|cost| -(*cost as f64) / temperature).collect();
        let max_weight = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        for w in &mut weights {
            *w = (*w - max_weight).exp();
        }

        // force no resampling of completed patterns
        for (i, state) in search_states.iter().enumerate() {
            if state.pattern.vars.is_empty() {
                weights[i] = 0.0;
            }
        }

        // zero out particles whose pattern doesn't match the follow target
        if let Some(ref follow) = shared.follow {
            apply_follow_constraint(&search_states, &mut weights, follow);
        }

        if weights.iter().sum::<f64>() == 0.0 {
            steps_run = step + 1;
            println!("all particles died, stopping");
            break;
        }

        if best_found_at.is_some_and(|best_found_at| (step as i64) - (best_found_at as i64) > dead_runs) {
            steps_run = step + 1;
            println!("no progress in 100 steps, stopping at {}", step);
            break;
        }

        // resample
        normalize_and_accumulate(&mut weights);

        println!("Step {}: expanded all particles", step);
        for i in 0..min(5, search_states.len()) {
            println!("Sample particle {}: {}; cost={} weight={}", i, search_states[i].pattern, costs[i], weights[i]);
        }

        search_states = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights);
                search_states[idx].clone()
            })
            .collect();
        steps_run = step + 1;
    }

    if let (Some(iter), Some((cost, state))) = (best_found_at, best_so_far.as_ref()) {
        println!("best found at iteration {}: {}", iter, cost);
        println!("program: {}", state.pattern);
        println!("best: {}", cost);
        println!("Compression ratio: {}", original_size as f64 / *cost as f64);
    }

    SmcResult {
        best: best_so_far,
        original_size,
        best_found_at,
        num_steps_run: steps_run,
        egraph: shared.egraph,
    }
}

pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    // println!("Choosing from weights: {:?}", cum_weights);
    let r: f64 = rand::rng().random_range(0.0..1.0);
    // println!("r: {:?}", r);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) => idx,
        Err(idx) => idx, // it could be inserted at idx, which means it's <= cum_weights[idx]
    }
}

pub fn normalize_and_accumulate(weights: &mut Vec<f64>) {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum == 0.0 {
        let len = weights.len();
        weights.fill(1.0 / len as f64);
    } else {
        weights.iter_mut().for_each(|w| *w /= weight_sum);
    }
    let mut accum = 0.0;
    for w in weights {
        accum += *w;
        *w = accum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Args;
    use crate::io;
    use clap::Parser;

    const INPUT: &str = "data/domains/cogsci/dials.json";
    const RULES: &str = "../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites";

    /// True if the fixture files required for the follow-constraint integration
    /// tests are present on disk. If not, tests skip with a message instead of
    /// failing, so fresh clones and CI without the babble checkout still pass.
    fn fixtures_present() -> bool {
        std::path::Path::new(INPUT).exists() && std::path::Path::new(RULES).exists()
    }

    fn parse_follow(s: &str) -> crate::revexpr::RevExpr<egg::ENodeOrVar<crate::lang::StitchLang>> {
        let parsed = s.parse().expect("parse follow");
        validate_follow(&parsed);
        parsed
    }

    fn run(args: &Args) -> SmcResult {
        let (egraph, root, _cost_before) = io::load_egraph(&args.input, args.rules.as_deref());
        smc(egraph, root, args)
    }

    fn assert_best_matches_follow(result: &SmcResult, follow_str: &str) {
        let follow = parse_follow(follow_str);
        let (cost, best) = result.best.as_ref().expect("smc should produce a best pattern");
        assert!(
            best.matches_follow(&follow),
            "best pattern (cost={}, pattern={}) should match follow {}",
            cost,
            best.pattern,
            follow_str,
        );
    }

    /// The full complex follow from the baseline invocation we're locking in.
    const DIALS_FULL_FOLLOW: &str = "(T (T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0)) (M 1 0 (* ?#0 (* 0.5 (cos (/ pi 4)))) (* ?#0 (* 0.5 (sin (/ pi 4))))))";

    /// Mirror of the baseline CLI. Needs high temperature (1000) so enough
    /// particles survive random expansion at follow-Var positions.
    #[test]
    #[ignore = "slow: 1000 steps * 1000 particles; run with --release --ignored"]
    fn follow_dials_full_baseline() {
        if !fixtures_present() {
            eprintln!("skipping: fixtures not available");
            return;
        }
        let args = Args::parse_from([
            "egg-stitch",
            "--input", INPUT,
            "--rules", RULES,
            "--num-steps", "1000",
            "--num-particles", "1000",
            "--temperature", "1000",
            "--follow", DIALS_FULL_FOLLOW,
            "--max-arity", "2",
        ]);
        let result = run(&args);
        assert_best_matches_follow(&result, DIALS_FULL_FOLLOW);
    }

    /// Shallow follow with no variables — fast variant that should be
    /// reachable in a small number of SMC steps.
    #[test]
    fn follow_shallow_no_placeholders() {
        if !fixtures_present() {
            eprintln!("skipping: fixtures not available");
            return;
        }
        let follow = "(T l (M 1 0 -0.5 0))";
        let args = Args::parse_from([
            "egg-stitch",
            "--input", INPUT,
            "--rules", RULES,
            "--num-steps", "30",
            "--num-particles", "200",
            "--follow", follow,
            "--max-arity", "2",
        ]);
        let result = run(&args);
        assert_best_matches_follow(&result, follow);
    }

    /// Follow containing a `?#0` variable: exercises the strict check where
    /// pattern ENode at a follow-Var position is rejected.
    #[test]
    #[ignore = "slow: 1000 steps * 1000 particles; run with --release --ignored"]
    fn follow_single_placeholder() {
        if !fixtures_present() {
            eprintln!("skipping: fixtures not available");
            return;
        }
        let follow = "(T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0))";
        let args = Args::parse_from([
            "egg-stitch",
            "--input", INPUT,
            "--rules", RULES,
            "--num-steps", "1000",
            "--num-particles", "1000",
            "--temperature", "1000",
            "--follow", follow,
            "--max-arity", "2",
        ]);
        let result = run(&args);
        assert_best_matches_follow(&result, follow);
    }

    /// SMC with no follow at all — sanity that the loop still produces a best
    /// candidate when the constraint is absent.
    #[test]
    fn no_follow_still_produces_best() {
        if !fixtures_present() {
            eprintln!("skipping: fixtures not available");
            return;
        }
        let args = Args::parse_from([
            "egg-stitch",
            "--input", INPUT,
            "--rules", RULES,
            "--num-steps", "20",
            "--num-particles", "100",
            "--max-arity", "2",
        ]);
        let result = run(&args);
        assert!(result.best.is_some(), "smc should produce a best pattern without a follow");
    }
}
