use std::cmp::min;

use colored::Colorize;

use crate::cost::compute_pattern_size;
use crate::search::{SearchState, SharedSearchData};

/// Zeros the weight of particles that don't match the follow pattern, and prints status.
pub fn apply_follow_constraint(states: &[SearchState], weights: &mut [f64], follow: &crate::revexpr::RevExpr<egg::ENodeOrVar<crate::lang::StitchLang>>) {
    let total_weight: f64 = weights.iter().sum();
    let mut found = false;
    for (i, state) in states.iter().enumerate() {
        if !state.matches_follow(follow) {
            weights[i] = 0.0;
        } else {
            found = true;
        }
    }
    if found {
        let matching_weight: f64 = weights.iter().sum();
        let frac = if total_weight > 0.0 { matching_weight / total_weight } else { 0.0 };
        println!(
            "{} {}",
            "follow:".dimmed(),
            format!("{} / {} particles match ({:.1}% of weight)", weights.iter().filter(|&&w| w > 0.0).count(), weights.len(), frac * 100.0).blue()
        );
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
