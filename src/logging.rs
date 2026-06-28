use std::cmp::min;

use colored::Colorize;

use crate::cost::compute_pattern_size;
use crate::lang::{LanguageFamily, OpWithVar, StitchOp};
use crate::search::{SearchState, SharedSearchData};

/// Marks particles that don't match the follow pattern as `pruned`, returning
/// whether any particle matched (the caller warns when none do).
pub fn apply_follow_constraint<F: LanguageFamily, O: StitchOp>(states: &[SearchState<F, O>], pruned: &mut [bool], follow: &crate::revexpr::RevExpr<F::Apply<OpWithVar<O>>>) -> bool {
    let mut found = false;
    for (i, state) in states.iter().enumerate() {
        if state.matches_follow(follow) {
            found = true;
        } else {
            pruned[i] = true;
        }
    }
    found
}

/// Prints summary info for the top particles (up to 5), ordered by descending weight.
pub fn print_top_particles<F: LanguageFamily, O: StitchOp>(states: &[SearchState<F, O>], weights: &[f64], shared: &SharedSearchData<F, O>, original_size: usize, get_cost: impl Fn(usize) -> usize) {
    let mut sorted_idx: Vec<usize> = (0..states.len()).collect();
    sorted_idx.sort_by(|&a, &b| weights[b].partial_cmp(&weights[a]).unwrap_or(std::cmp::Ordering::Equal));
    for &i in sorted_idx.iter().take(min(5, states.len())) {
        let pat_size = compute_pattern_size(&states[i].pattern, &shared.egraph.analysis.weights);
        let cost_i = get_cost(i);
        let ratio = original_size as f64 / cost_i as f64;
        println!("  {} {}", format!("p{}:", i).dimmed(), states[i].pattern.to_string().cyan());
        println!("      cost={} ratio={:.2}x weight={:.4} matches={} pat_size={}", cost_i, ratio, weights[i], states[i].matches.len(), pat_size,);
    }
}
