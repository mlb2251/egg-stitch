use colored::Colorize;

use crate::best_first::InteractiveSearch;
use crate::math::logaddexp;
use rand::Rng;

/// Configuration for an SMC search run.
pub struct SmcConfig {
    pub num_particles: usize,
    pub num_steps: usize,
    pub temperature: f64,
    pub dead_runs: usize,
    pub verbose: bool,
}

/// Runs SMC over the interactive search tree. Particles are node IDs;
/// expanding a particle enumerates all successors in the shared tree,
/// then one child is chosen uniformly at random. Best tracking and
/// tree snapshots are all maintained by `InteractiveSearch`.
pub fn smc(search: &mut InteractiveSearch, config: &SmcConfig) {
    let mut particles: Vec<usize> = vec![0; config.num_particles];
    let mut best_found_at_step: Option<usize> = None;

    for step in 0..config.num_steps {
        let old_best = search.best_cost();

        // === PROPOSE: expand each particle's node, pick a random child ===
        for p in particles.iter_mut() {
            let children = search.ensure_expanded(*p).to_vec();
            if !children.is_empty() {
                *p = children[rand::rng().random_range(0..children.len())];
            }
        }

        if search.best_cost() != old_best {
            if let Some((cost, state)) = search.best_state() {
                println!("{} {} {}", format!("[step {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), state.pattern.to_string().cyan());
            }
            best_found_at_step = Some(step);
        }

        // === WEIGHT ===
        let costs: Vec<usize> = particles.iter().map(|&p| search.node_cost(p)).collect();
        let mut log_weights: Vec<f64> = costs.iter().map(|c| -(*c as f64) / config.temperature).collect();

        // Terminal particles (no variables) get -inf weight
        for (i, &p) in particles.iter().enumerate() {
            if search.node_num_vars(p) == 0 {
                log_weights[i] = f64::NEG_INFINITY;
            }
        }
        // Follow constraint is enforced during tree expansion (children that
        // don't match are never added), so no separate filtering needed here.

        let total = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let mut weights: Vec<f64> = if total.is_finite() { log_weights.iter().map(|lw| (lw - total).exp()).collect() } else { vec![0.0; config.num_particles] };

        if weights.iter().sum::<f64>() == 0.0 {
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        if best_found_at_step.is_some_and(|bf| (step as i64 - bf as i64) > config.dead_runs as i64) {
            println!("{}", format!("no progress in {} steps, stopping at {}", config.dead_runs, step).yellow());
            break;
        }

        // === RESAMPLE ===
        let weights_acc = normalize_and_accumulate(&mut weights);
        particles = (0..config.num_particles).map(|_| particles[weighted_choice(&weights_acc)]).collect();
    }
}

/// Samples an index from a normalized cumulative weight array.
pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    let r: f64 = rand::rng().random_range(0.0..1.0);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) | Err(idx) => idx,
    }
}

/// Normalizes `weights` to sum to 1 in-place and returns a fresh cumulative distribution.
pub fn normalize_and_accumulate(weights: &mut [f64]) -> Vec<f64> {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum == 0.0 {
        let len = weights.len();
        weights.fill(1.0 / len as f64);
    } else {
        weights.iter_mut().for_each(|w| *w /= weight_sum);
    }
    let mut weights_acc = Vec::with_capacity(weights.len());
    let mut accum = 0.0;
    for w in weights.iter() {
        accum += *w;
        weights_acc.push(accum);
    }
    weights_acc
}
