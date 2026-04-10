use std::cmp::min;

use colored::Colorize;

use crate::cost::compute_pattern_size;
use crate::math::logaddexp;
use crate::search::{SearchState, SharedSearchData};

/// Sets the log weight of particles that don't match the follow pattern to -inf, and prints status.
pub fn apply_follow_constraint(states: &[SearchState], log_weights: &mut [f64], follow: &crate::revexpr::RevExpr<egg::ENodeOrVar<crate::lang::StitchLang>>) {
    let log_total = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
    let mut found = false;
    for (i, state) in states.iter().enumerate() {
        if !state.matches_follow(follow) {
            log_weights[i] = f64::NEG_INFINITY;
        } else {
            found = true;
        }
    }
    if found {
        let log_matching = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let frac = if log_total > f64::NEG_INFINITY { (log_matching - log_total).exp() } else { 0.0 };
        let num_matching = log_weights.iter().filter(|&&lw| lw > f64::NEG_INFINITY).count();
        println!("{} {}", "follow:".dimmed(), format!("{} / {} particles match ({:.1}% of weight)", num_matching, log_weights.len(), frac * 100.0).blue());
    } else {
        println!("{}", "No particles match the follow pattern".red().bold());
    }
}

/// Prints summary info for the top particles (up to 5).
pub fn print_top_particles(states: &[SearchState], weights: &[f64], shared: &SharedSearchData, original_size: usize, get_cost: impl Fn(usize) -> usize) {
    for i in 0..min(5, states.len()) {
        let usage_matches: usize = states[i].matches.iter().map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
        let pat_size = compute_pattern_size(&states[i].pattern);
        let appx_cost = original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
        let cost_i = get_cost(i);
        let ratio = original_size as f64 / cost_i as f64;
        println!("  {} {}", format!("p{}:", i).dimmed(), states[i].pattern.to_string().cyan());
        println!(
            "      cost={} ratio={:.2}x weight={:.4} matches={} usage_matches={} pat_size={} appx_cost={}",
            cost_i,
            ratio,
            weights[i],
            states[i].matches.len(),
            usage_matches,
            pat_size,
            appx_cost
        );
    }
}
