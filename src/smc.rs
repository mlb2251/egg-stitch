use std::cmp::min;

use colored::Colorize;

use crate::cost::{compute_cost, compute_pattern_size, compute_size};
use crate::lang::StitchEgraph;
use crate::revexpr::RevExpr;
use crate::search::{SearchState, SharedSearchData};
use egg::ENodeOrVar;
use rand::Rng;

/// Runs SMC to find a pattern that minimizes compressed corpus size.
pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> Option<(usize, SearchState)> {
    let follow_expr: Option<RevExpr<ENodeOrVar<crate::lang::StitchLang>>> = args.follow.as_deref().map(|s|
        s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e))
    );
    let usage_counts = crate::search::compute_usage_counts(&egraph, root);
    let shared = SharedSearchData { egraph, follow: follow_expr, weight_by_usage: args.weight_by_usage, usage_counts, p_reuse: args.p_reuse, check_slow: args.check_slow };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared), shared.check_slow);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;

    let mut search_states: Vec<SearchState> = (0..num_particles)
        .map(|_i| SearchState::new(&shared))
        .collect();

    for step in 0..num_steps {
        for (i, search_state) in search_states.iter_mut().enumerate() {
            let verb = false;
            if verb {
                println!("Expanding particle {} with pattern: {}", i, search_state.pattern);
            }
            search_state.expand_random(&shared, verb);
            if verb {
                println!("Expanded particle {} to pattern: {}", i, search_state.pattern);
            }
        }

        let costs: Vec<usize> = search_states
            .iter()
            .map(|search_state| compute_cost(&shared.egraph, root, search_state, shared.check_slow))
            .collect();
        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity
                && best_so_far.as_ref().is_none_or(|best| *cost < best.0)
            {
                println!("{} {} {}",
                    format!("[iteration {}]", step).yellow().bold(),
                    format!("new best: {}", cost).green().bold(),
                    search_states[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }

        let mut weights: Vec<f64> = costs.iter().map(|cost| -(*cost as f64) / temperature).collect();
        let max_weight = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        for w in &mut weights {
            *w = (*w - max_weight).exp();
        }

        for (i, state) in search_states.iter().enumerate() {
            if state.pattern.vars.is_empty() {
                weights[i] = 0.0;
            }
        }

        if let Some(ref follow) = shared.follow {
            let total_weight: f64 = weights.iter().sum();
            let mut found = false;
            for (i, state) in search_states.iter().enumerate() {
                if !state.matches_follow(follow) {
                    weights[i] = 0.0;
                } else {
                    found = true;
                }
            }
            if found {
                let matching_weight: f64 = weights.iter().sum();
                let weight_frac = if total_weight > 0.0 { matching_weight / total_weight } else { 0.0 };
                println!("{} {}", "follow:".dimmed(), format!("{} / {} particles match ({:.1}% of weight)", weights.iter().filter(|&&w| w > 0.0).count(), weights.len(), weight_frac * 100.0).blue());
            } else {
                println!("{}", "No particles match the follow pattern".red().bold());
            }
        }

        if weights.iter().sum::<f64>() == 0.0 {
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }

        if best_found_at.is_some_and(|best_found_at| (step as i64) - (best_found_at as i64) > dead_runs as i64) {
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        normalize_and_accumulate(&mut weights);

        println!("{}", format!("Step {}: expanded all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| costs[i]);

        search_states = (0..num_particles).map(|_| {
            let idx = weighted_choice(&weights);
            search_states[idx].clone()
        }).collect();

        println!("{}", format!("Step {}: resampled all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| {
            compute_cost(&shared.egraph, root, &search_states[i], shared.check_slow)
        });
    }

    let cost = compute_cost(&shared.egraph, root, &best_so_far.as_ref().unwrap().1, shared.check_slow);
    println!("\n{}", "═══ RESULT ═══".green().bold());
    println!("{} {}", "best found at iteration:".dimmed(), best_found_at.unwrap().to_string().yellow());
    println!("{} {}", "pattern:".dimmed(), best_so_far.as_ref().unwrap().1.pattern.to_string().cyan().bold());
    println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
    println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / cost as f64).green().bold());

    best_so_far
}

/// Samples an index from a normalized cumulative weight array.
pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    let r: f64 = rand::rng().random_range(0.0..1.0);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) => idx,
        Err(idx) => idx,
    }
}

/// Normalizes weights to sum to 1 then converts to a cumulative distribution in-place.
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

/// Prints summary info for the top particles (up to 5).
fn print_top_particles(
    states: &[SearchState],
    weights: &[f64],
    shared: &SharedSearchData,
    original_size: usize,
    get_cost: impl Fn(usize) -> usize,
) {
    for i in 0..min(5, states.len()) {
        let usage_matches: usize = states[i].matches.iter()
            .map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1))
            .sum();
        let pat_size = compute_pattern_size(&states[i].pattern);
        let appx_cost = original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
        let cost_i = get_cost(i);
        let ratio = original_size as f64 / cost_i as f64;
        println!("  {} {}", format!("p{}:", i).dimmed(), states[i].pattern.to_string().cyan());
        println!("      cost={} ratio={:.2}x weight={:.4} matches={} usage_matches={} pat_size={} appx_cost={}", cost_i, ratio, weights[i], states[i].matches.len(), usage_matches, pat_size, appx_cost);
    }
}
