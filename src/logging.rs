use colored::Colorize;

use crate::search::SearchState;

/// Zeros the weight of particles that don't match the follow pattern. Warns if no
/// particles match. Weights are in linear space here; when particle multiplicity
/// lands this will move to log space.
pub fn apply_follow_constraint(states: &[SearchState], weights: &mut [f64], follow: &crate::revexpr::RevExpr<crate::lang::StitchLang>) {
    let mut found = false;
    for (i, state) in states.iter().enumerate() {
        if state.matches_follow(follow) {
            found = true;
        } else {
            weights[i] = 0.0;
        }
    }
    if !found {
        println!("{}", "No particles match the follow pattern".red().bold());
    }
}
