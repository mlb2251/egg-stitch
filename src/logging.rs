use std::cmp::min;

use colored::Colorize;

use crate::cost::compute_pattern_size;
use crate::lang::{LanguageFamily, OpWithVar, StitchOp};
use crate::matching::MatchAtEClass;
use crate::search::{SearchState, SharedSearchData};

/// Prints a compact view of a match structure under `--verbose`: the number of
/// match roots, then the first `max_roots` roots (sorted by their largest
/// factor's row count, descending), each shown as its factors' row counts keyed
/// by slot set (e.g. `{0,1,2}=8 {3,4}=3`).
pub fn print_match_structure(matches: &[MatchAtEClass], max_roots: usize) {
    println!("{} {} roots", "match structure:".dimmed(), matches.len());
    let largest = |m: &MatchAtEClass| m.factors.iter().map(|f| f.rows.len()).max().unwrap_or(0);
    let mut order: Vec<&MatchAtEClass> = matches.iter().collect();
    order.sort_by_key(|m| std::cmp::Reverse(largest(m)));
    for m in order.iter().take(max_roots) {
        let factors: Vec<String> = m.factors.iter().map(|f| format!("{{{}}}={}", f.slots.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(","), f.rows.len())).collect();
        println!("  root {} {}", m.root_eclass, factors.join(" "));
    }
    if matches.len() > max_roots {
        println!("  {}", "...".dimmed());
    }
}

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
pub fn print_top_particles<F: LanguageFamily, O: StitchOp>(states: &[SearchState<F, O>], weights: &[f64], shared: &SharedSearchData<F, O>, original_size: usize, match_structure: bool, get_cost: impl Fn(usize) -> usize) {
    let mut sorted_idx: Vec<usize> = (0..states.len()).collect();
    sorted_idx.sort_by(|&a, &b| weights[b].partial_cmp(&weights[a]).unwrap_or(std::cmp::Ordering::Equal));
    for &i in sorted_idx.iter().take(min(5, states.len())) {
        let pat_size = compute_pattern_size(&states[i].pattern, &shared.egraph.analysis.weights);
        let cost_i = get_cost(i);
        let ratio = original_size as f64 / cost_i as f64;
        println!("  {} {}", format!("p{}:", i).dimmed(), states[i].pattern.to_string().cyan());
        println!("      cost={} ratio={:.2}x weight={:.4} matches={} pat_size={}", cost_i, ratio, weights[i], states[i].matches.len(), pat_size,);
        if match_structure {
            print_match_structure(&states[i].matches, 10);
        }
    }
}
