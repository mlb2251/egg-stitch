use colored::Colorize;

use crate::cost::{CostScratch, compute_cost};
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::{LanguageFamily, OpWithVar, StitchOp};
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::math::logaddexp;
use crate::revexpr::RevExpr;
use crate::search::{Action, SearchState, SuccessorEnum, setup_search};
use rand::Rng;
use rand::rngs::StdRng;
use rustc_hash::FxHashMap;

/// Inserts a freshly-expanded state into the parallel (states, mults) deduped-by-pattern
/// buffer, either bumping the multiplicity of an existing group by `count` or pushing a new one.
fn dedup_insert<F: LanguageFamily, O: StitchOp>(s: SearchState<F, O>, count: usize, states: &mut Vec<SearchState<F, O>>, mults: &mut Vec<usize>, dedup: &mut FxHashMap<RevExpr<F::Apply<OpWithVar<O>>>, usize>) {
    match dedup.get(&s.pattern.pattern) {
        Some(&idx) => mults[idx] += count,
        None => {
            let idx = states.len();
            dedup.insert(s.pattern.pattern.clone(), idx);
            states.push(s);
            mults.push(count);
        }
    }
}

/// Output of a completed SMC run.
pub struct SmcResult<F: LanguageFamily, O: StitchOp> {
    pub best: Option<(usize, SearchState<F, O>)>,
    pub original_size: usize,
    pub best_found_at: Option<usize>,
    pub num_steps_run: usize,
    pub data: crate::shared::SharedData<F, O>,
    pub debug_log: Option<DebugLog>,
}

/// Runs SMC to find a pattern that minimizes compressed corpus size.
///
/// Particles are stored as `(SearchState, multiplicity)` pairs. After each
/// expansion step, identical patterns are deduplicated and their counts merged,
/// so cost computation runs once per unique pattern instead of once per particle.
#[allow(clippy::needless_range_loop)]
pub fn smc<F: LanguageFamily, O: StitchOp>(data: crate::shared::SharedData<F, O>, args: &crate::Args, rng: &mut StdRng) -> SmcResult<F, O> {
    let (shared, cost_cache, original_size) = setup_search(data, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let num_particles = args.num_particles;
    let num_steps = args.num_steps.expect("--num-steps is required for SMC search");
    let temperature = args.temperature;
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;
    let no_zero_arity = args.no_zero_arity;
    let verbose = args.verbose;

    let mut best_so_far: Option<(usize, SearchState<F, O>)> = None;
    let mut best_found_at = None;
    let mut steps_run = 0;
    let debug = args.debug_log;
    let mut debug_steps: Vec<StepLog> = Vec::new();

    let mut particles: Vec<(SearchState<F, O>, usize)> = vec![(SearchState::new(&shared, None), num_particles)];
    let mut scratch = CostScratch::new(&shared.egraph);
    let mut dominance_hits: usize = 0;

    for step in 0..num_steps {
        // For each (state, mult) group, enumerate successor *actions* (no child
        // states built up front), then resample `mult` of them. Each action's
        // sampling weight is its `(match, subst)` support count; reuse-action
        // weights are additionally multiplied by `boost_reuse_weight`. Child
        // states are materialised only for sampled actions via `apply_action`,
        // avoiding the per-shape `clone + expand` work for successors that
        // win zero samples. Resulting patterns are deduped globally across groups.
        let mut expanded: Vec<SearchState<F, O>> = Vec::new();
        let mut mults: Vec<usize> = Vec::new();
        let mut dedup: FxHashMap<RevExpr<F::Apply<OpWithVar<O>>>, usize> = FxHashMap::default();
        for (state, mult) in particles.drain(..) {
            let actions = match state.enumerate_successor_actions(&shared, args.opt_dominance_reuse, &mut dominance_hits) {
                SuccessorEnum::Dominant { child, .. } => {
                    dedup_insert(child, mult, &mut expanded, &mut mults, &mut dedup);
                    continue;
                }
                SuccessorEnum::All(actions) => actions,
            };
            if actions.is_empty() {
                dedup_insert(state, mult, &mut expanded, &mut mults, &mut dedup);
                continue;
            }
            let mut weights = action_weights_with_reuse_boost(&actions, args.boost_reuse_weight);
            let acc = normalize_and_accumulate(&mut weights);
            let mut counts: Vec<usize> = vec![0; actions.len()];
            for _ in 0..mult {
                counts[weighted_choice(&acc, rng)] += 1;
            }
            for ((action, _), count) in actions.into_iter().zip(counts) {
                if count > 0 {
                    let child = state.apply_action(&action, &shared);
                    dedup_insert(child, count, &mut expanded, &mut mults, &mut dedup);
                }
            }
        }
        drop(dedup);

        let costs: Vec<usize> = expanded.iter().map(|s| compute_cost(&shared.egraph, shared.root, &cost_cache, &mut scratch, s, shared.check_slow)).collect();

        for (i, cost) in costs.iter().enumerate() {
            let cost_to_beat: usize = best_so_far.as_ref().map_or(original_size, |best| best.0);
            let arity = expanded[i].pattern.vars.len();
            if arity <= max_arity && !(no_zero_arity && arity == 0) && *cost < cost_to_beat {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), expanded[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, expanded[i].clone()));
                best_found_at = Some(step);
            }
        }

        // log-space weights: logw_i = -cost_i / temperature
        let mut log_weights: Vec<f64> = costs.iter().map(|c| -(*c as f64) / temperature).collect();

        for (i, s) in expanded.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                log_weights[i] = f64::NEG_INFINITY;
            }
        }

        if let Some(ref follow) = shared.follow {
            apply_follow_constraint(&expanded, &mut log_weights, follow, &shared, original_size, &costs, verbose);
        }

        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let mut weights: Vec<f64> = if total_weight.is_finite() { log_weights.iter().map(|lw| (lw - total_weight).exp()).collect() } else { vec![0.0; log_weights.len()] };

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        if best_found_at.is_some_and(|bf| (step as i64) - (bf as i64) > dead_runs as i64) {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        if verbose {
            println!("{}", format!("Step {}: expanded all particles", step).dimmed());
            print_top_particles(&expanded, &weights, &shared, original_size, |i| costs[i]);
        }

        let weights_acc = normalize_and_accumulate(&mut weights);
        let mut counts: Vec<usize> = vec![0; expanded.len()];
        let resample_indices: Vec<usize> = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights_acc, rng);
                counts[idx] += 1;
                idx
            })
            .collect();

        if debug {
            debug_steps.push(StepLog {
                step,
                particles: build_particle_logs(&expanded, &costs, &weights),
                resample_indices,
                best_cost: best_so_far.as_ref().map(|(c, _)| *c),
                best_pattern: best_so_far.as_ref().map(|(_, s)| s.pattern.to_string()),
            });
        }

        if verbose {
            println!("{}", format!("Step {}: resampled all particles", step).dimmed());
            let resample_weights: Vec<f64> = counts.iter().map(|&c| c as f64 / num_particles as f64).collect();
            print_top_particles(&expanded, &resample_weights, &shared, original_size, |i| costs[i]);
        }

        particles = expanded.into_iter().zip(counts).filter(|(_, c)| *c > 0).collect();
        steps_run = step + 1;
    }

    println!("\n{}", "═══ STATS ═══".blue().bold());
    println!("{} {}", "dominance hits:".dimmed(), dominance_hits.to_string().bold());

    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, state))) = (best_found_at, best_so_far.as_ref()) {
        println!("{} {}", "best found at iteration:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / *cost as f64).green().bold());
    }

    let debug_log = if debug {
        Some(DebugLog {
            original_size,
            num_particles,
            temperature,
            steps: debug_steps,
        })
    } else {
        None
    };
    SmcResult {
        best: best_so_far,
        original_size,
        best_found_at,
        num_steps_run: steps_run,
        data: shared.into_data(),
        debug_log,
    }
}

/// Weights each successor by its `(match, subst)` support count, with reuse
/// supports additionally multiplied by `boost_reuse_weight`. With a boost of
/// 1.0 this is the pure support-weighted distribution; larger values bias
/// sampling toward reuse actions, smaller values toward expansion.
fn action_weights_with_reuse_boost<D>(actions: &[(Action<D>, usize)], boost_reuse_weight: f64) -> Vec<f64> {
    actions
        .iter()
        .map(|(a, support)| {
            let kind_scale = if matches!(a, Action::Reuse { .. }) { boost_reuse_weight } else { 1.0 };
            kind_scale * (*support as f64)
        })
        .collect()
}

/// Finds the index `i` such that `r` falls into the half-open interval
/// `(acc_weights[i-1], acc_weights[i]]` (treating the implicit prefix as 0).
///
/// Uses `partition_point` so that zero-weight prefixes (entries whose
/// cumulative value equals a previous one) are skipped, and clamps to the
/// last index to defend against float round-off leaving the final
/// accumulator slightly below 1.0.
pub fn index_from_cumulative(acc_weights: &[f64], r: f64) -> usize {
    let idx = acc_weights.partition_point(|&w| w <= r);
    idx.min(acc_weights.len() - 1)
}

/// Samples an index from a normalized cumulative weight array.
pub fn weighted_choice(acc_weights: &[f64], rng: &mut StdRng) -> usize {
    let r: f64 = rng.random_range(0.0..1.0);
    index_from_cumulative(acc_weights, r)
}

/// Normalizes weights in-place and returns a separate cumulative distribution.
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

#[cfg(test)]
mod tests {
    use super::index_from_cumulative;

    #[test]
    fn picks_interval_containing_r() {
        let acc = vec![0.3, 0.7, 1.0];
        assert_eq!(index_from_cumulative(&acc, 0.0), 0);
        assert_eq!(index_from_cumulative(&acc, 0.2), 0);
        assert_eq!(index_from_cumulative(&acc, 0.5), 1);
        assert_eq!(index_from_cumulative(&acc, 0.9), 2);
    }

    #[test]
    fn skips_zero_weight_prefix() {
        // Two leading zero-weight entries, then one with all the mass.
        let acc = vec![0.0, 0.0, 1.0];
        assert_eq!(index_from_cumulative(&acc, 0.0), 2);
        assert_eq!(index_from_cumulative(&acc, 0.5), 2);
    }

    #[test]
    fn skips_zero_weight_in_middle() {
        // Middle entry has zero weight; should never be picked.
        let acc = vec![0.3, 0.3, 1.0];
        for i in 0..1000 {
            let r = i as f64 / 1000.0;
            assert_ne!(index_from_cumulative(&acc, r), 1);
        }
    }

    #[test]
    fn clamps_when_r_exceeds_last_accumulator() {
        // Simulates float roundoff where the final cumulative drifts below 1.0.
        let acc = vec![0.3, 0.7, 0.9999999];
        assert_eq!(index_from_cumulative(&acc, 0.99999995), 2);
    }
}
