use std::cmp::min;

use colored::Colorize;

use crate::cost::compute_pattern_size;
use crate::lang::StitchLanguage;
use crate::math::logaddexp;
use crate::search::{SearchState, SharedSearchData};

/// Sets the log weight of particles that don't match the follow pattern to -inf.
pub fn apply_follow_constraint<L: StitchLanguage>(states: &[SearchState<L>], log_weights: &mut [f64], follow: &crate::revexpr::RevExpr<egg::ENodeOrVar<L>>, shared: &SharedSearchData<L>, original_size: usize, costs: &[usize], verbose: bool) {
    let log_total = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);

    if verbose {
        let weights_before: Vec<f64> = if log_total.is_finite() { log_weights.iter().map(|lw| (lw - log_total).exp()).collect() } else { vec![0.0; log_weights.len()] };
        let mut sorted_idx: Vec<usize> = (0..states.len()).collect();
        sorted_idx.sort_by(|&a, &b| weights_before[b].partial_cmp(&weights_before[a]).unwrap_or(std::cmp::Ordering::Equal));
        println!("{}", "top 5 particles before follow constraint:".dimmed());
        for &i in sorted_idx.iter().take(5) {
            let usage_matches: usize = states[i].matches.iter().map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
            let pat_size = compute_pattern_size(&states[i].pattern);
            let appx_cost = original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
            println!(
                "  {} {} cost={} ratio={:.2}x weight={:.4} matches={} usage_matches={} pat_size={} appx_cost={}",
                format!("p{}:", i).dimmed(),
                states[i].pattern.to_string().cyan(),
                costs[i],
                original_size as f64 / costs[i] as f64,
                weights_before[i],
                states[i].matches.len(),
                usage_matches,
                pat_size,
                appx_cost
            );
        }
    }

    let mut found = false;
    for (i, state) in states.iter().enumerate() {
        if !state.matches_follow(follow) {
            log_weights[i] = f64::NEG_INFINITY;
        } else {
            found = true;
        }
    }
    if found {
        if verbose {
            let log_matching = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
            let frac = if log_total > f64::NEG_INFINITY { (log_matching - log_total).exp() } else { 0.0 };
            let num_matching = log_weights.iter().filter(|&&lw| lw > f64::NEG_INFINITY).count();
            println!("{} {}", "follow:".dimmed(), format!("{} / {} particles match ({:.1}% of weight)", num_matching, log_weights.len(), frac * 100.0).blue());
        }
    } else {
        println!("{}", "No particles match the follow pattern".red().bold());
    }
}

/// Prints summary info for the top particles (up to 5).
pub fn print_top_particles<L: StitchLanguage>(states: &[SearchState<L>], weights: &[f64], shared: &SharedSearchData<L>, original_size: usize, get_cost: impl Fn(usize) -> usize) {
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
