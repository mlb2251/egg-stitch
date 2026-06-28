use colored::Colorize;

use crate::cost::{CostScratch, SearchStateWithCostSelection, compute_cost_and_select};
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::{LanguageFamily, StitchOp};
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::lower_bound::{LowerBoundPruner, PruneResult};
use crate::math::logaddexp;
use crate::pattern::Pattern;
use crate::search::{Action, SearchState, SharedSearchData, SuccessorEnum, setup_search};
use rand::Rng;
use rand::rngs::StdRng;
use rustc_hash::FxHashMap;

/// Inserts a freshly-expanded state into the parallel (states, mults) deduped-by-pattern
/// buffer, either bumping the multiplicity of an existing group by `count` or pushing a new one.
fn dedup_insert<F: LanguageFamily, O: StitchOp>(s: SearchState<F, O>, count: usize, states: &mut Vec<SearchState<F, O>>, mults: &mut Vec<usize>, dedup: &mut FxHashMap<Pattern<F, O>, usize>) {
    match dedup.get(&s.pattern) {
        Some(&idx) => mults[idx] += count,
        None => {
            let idx = states.len();
            dedup.insert(s.pattern.clone(), idx);
            states.push(s);
            mults.push(count);
        }
    }
}

/// `(cost, winning state + the cost selection the optimiser picked for it)` —
/// the best particle an SMC run has found, or `None` before any is recorded.
type BestSoFar<F, O> = Option<(usize, SearchStateWithCostSelection<F, O>)>;

/// Output of a completed SMC run.
pub struct SmcResult<F: LanguageFamily, O: StitchOp> {
    /// `(cost, winning state + the cost selection the optimiser picked for it)`.
    /// Threading the selection out saves `multiple_step_search` from re-running
    /// `compute_cost_and_select` just to recover it.
    pub best: BestSoFar<F, O>,
    pub original_size: usize,
    pub best_found_at: Option<usize>,
    pub num_steps_run: usize,
    pub data: crate::shared::SharedData<F, O>,
    pub debug_log: Option<DebugLog>,
}

/// Result of reweighting one expanded generation into a resampling distribution.
enum ReweightOutcome {
    /// Normalized resample probabilities (all-zero if every particle died).
    Weights(Vec<f64>),
    /// `--follow` mode: a surviving particle exactly matches the follow target,
    /// so the search is done. Carries the winner's index, its cost, and the
    /// post-constraint weights for the final debug-log entry.
    FollowExactMatch { index: usize, cost: usize, probs: Vec<f64> },
}

/// Mutable driver state for one SMC run: the read-only [`SharedSearchData`]
/// context, the borrowed CLI `args` and `rng`, and the running best-so-far
/// bookkeeping. The best-so-far fields are private so all reads and writes go
/// through the methods below — the search loop never pokes them directly.
pub struct SmcSearchData<'a, F: LanguageFamily, O: StitchOp> {
    /// Shared read-only search context (e-graph, root, follow target, …).
    pub shared: SharedSearchData<F, O>,
    /// CLI arguments for this run.
    pub args: &'a crate::Args,
    /// RNG threaded through every sampling step (action choice, resampling).
    pub rng: &'a mut StdRng,
    /// Size of the un-abstracted corpus; the cost a new best must beat while
    /// none has been recorded yet.
    pub original_size: usize,
    /// `(cost, winning state + the cost selection the optimiser picked)` of the
    /// best particle found so far.
    best_so_far: BestSoFar<F, O>,
    /// Step at which the best (or, in `--follow` mode, prefix progress) last
    /// improved; the dead-runs stopping rule measures staleness from here.
    best_found_at: Option<usize>,
    /// `--follow` mode only: deepest prefix-passing RecExpr node count seen so
    /// far. Growth counts as improvement so the dead-runs check doesn't abort a
    /// search still climbing toward an exact match. See [`smc`].
    best_prefix_progress: usize,
}

impl<'a, F: LanguageFamily, O: StitchOp> SmcSearchData<'a, F, O> {
    /// Builds the driver from `setup_search`'s output and the run's borrows,
    /// with an empty best-so-far.
    fn new(shared: SharedSearchData<F, O>, args: &'a crate::Args, rng: &'a mut StdRng, original_size: usize) -> Self {
        Self {
            shared,
            args,
            rng,
            original_size,
            best_so_far: None,
            best_found_at: None,
            best_prefix_progress: 0,
        }
    }

    /// Expands one generation of particles into the next. For each `(state, mult)`
    /// group, enumerates successor *actions* (no child states built up front),
    /// then resamples `mult` of them. Each action's sampling weight is its
    /// `(match, subst)` support count; reuse-action weights are additionally
    /// multiplied by `--boost-reuse-weight`. Child states are materialised only
    /// for sampled actions via `apply_action`, avoiding the per-shape
    /// `clone + expand` work for successors that win zero samples. Children over
    /// the match-set cap are dropped. Resulting patterns are deduped globally
    /// across groups, so the returned vec holds one entry per unique pattern.
    /// `dominance_hits` / `useless_inline_hits` accumulate the run-wide pruning
    /// stats reported at the end.
    fn expand_particles(&mut self, particles: Vec<(SearchState<F, O>, usize)>, dominance_hits: &mut usize, useless_inline_hits: &mut usize) -> Vec<SearchState<F, O>> {
        let max_match_set = self.args.max_match_set;
        let mut expanded: Vec<SearchState<F, O>> = Vec::new();
        // Per-pattern multiplicities, needed only to merge duplicate patterns
        // during dedup; the merged counts aren't read after expansion (resample
        // weights are per unique pattern, by cost).
        let mut mults: Vec<usize> = Vec::new();
        let mut dedup: FxHashMap<Pattern<F, O>, usize> = FxHashMap::default();
        for (state, mult) in particles {
            let actions = match state.enumerate_successor_actions(&self.shared, self.args.opt_dominance_reuse, self.args.opt_useless_inline, usize::MAX, dominance_hits, useless_inline_hits) {
                SuccessorEnum::Dominant { child, .. } => {
                    if child.within_match_set_cap(max_match_set) {
                        dedup_insert(child, mult, &mut expanded, &mut mults, &mut dedup);
                    }
                    continue;
                }
                // SMC keeps creation order (`freeze_rule = false`), so the freeze
                // rule is inert — `apply_action` ignores `rank` below.
                SuccessorEnum::All { actions, .. } => actions,
            };
            if actions.is_empty() {
                dedup_insert(state, mult, &mut expanded, &mut mults, &mut dedup);
                continue;
            }
            let mut weights = action_weights_with_reuse_boost(&actions, self.args.boost_reuse_weight);
            let acc = normalize_and_accumulate(&mut weights);
            let mut counts: Vec<usize> = vec![0; actions.len()];
            for _ in 0..mult {
                counts[weighted_choice(&acc, self.rng)] += 1;
            }
            for ((action, _), count) in actions.into_iter().zip(counts) {
                if count > 0 {
                    let child = state.apply_action(&action, &self.shared, true, None);
                    if child.within_match_set_cap(max_match_set) {
                        dedup_insert(child, count, &mut expanded, &mut mults, &mut dedup);
                    }
                }
            }
        }
        expanded
    }

    /// Reweights the expanded generation into a resampling distribution.
    ///
    /// Steps: cost → log-weight `-cost/temperature` (lower-bound-pruned
    /// particles are dead at `-inf`, since no descendant can beat the current
    /// best); in `--follow` mode, zero out particles whose pattern isn't a valid
    /// prefix of the target and register prefix progress; kill zero-arity
    /// patterns (they can't abstract anything); then log-normalize to
    /// probabilities. If a surviving particle exactly matches the follow target,
    /// returns [`ReweightOutcome::FollowExactMatch`] instead so the caller can
    /// record it and stop.
    fn reweight(&mut self, expanded: &[SearchState<F, O>], costs: &[usize], pruned: &[bool], step: usize) -> ReweightOutcome {
        let temperature = self.args.temperature;
        let mut log_weights: Vec<f64> = costs.iter().enumerate().map(|(i, c)| if pruned[i] { f64::NEG_INFINITY } else { -(*c as f64) / temperature }).collect();

        if let Some(ref follow) = self.shared.follow {
            apply_follow_constraint(expanded, &mut log_weights, follow, &self.shared, self.original_size, costs, self.args.verbose);
            // Read everything that needs `follow` while its borrow is live, then
            // update best-so-far afterwards (the bookkeeping methods borrow all
            // of `self`, so they can't run while `follow` borrows `self.shared`).
            //
            // Prefix progress: a deeper RecExpr means the particle has expanded
            // further into the follow target's shape (see `note_prefix_progress`).
            let step_progress: usize = (0..expanded.len()).filter(|&i| log_weights[i] > f64::NEG_INFINITY).map(|i| expanded[i].pattern.pattern.nodes.len()).max().unwrap_or(0);
            // If any surviving particle is alpha-equivalent to the follow target,
            // the search has reached the goal — pick the cheapest such particle.
            // Prefix-survival is noisy; an exact hit is unambiguous.
            let exact = (0..expanded.len())
                .filter(|&i| log_weights[i] > f64::NEG_INFINITY && crate::follow::matches_follow_serialized(&expanded[i], follow, &self.shared.egraph))
                .map(|i| (i, costs[i]))
                .min_by_key(|&(_, c)| c);
            self.note_prefix_progress(step, step_progress);
            if let Some((index, cost)) = exact {
                let probs = log_weights.iter().map(|&lw| if lw.is_finite() { lw.exp() } else { 0.0 }).collect();
                return ReweightOutcome::FollowExactMatch { index, cost, probs };
            }
        }

        for (i, s) in expanded.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                log_weights[i] = f64::NEG_INFINITY;
            }
        }

        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let weights = if total_weight.is_finite() { log_weights.iter().map(|lw| (lw - total_weight).exp()).collect() } else { vec![0.0; log_weights.len()] };
        ReweightOutcome::Weights(weights)
    }

    /// The cost a candidate must come in strictly under to become the new best:
    /// the current best's cost, or `original_size` while no best exists.
    fn cost_to_beat(&self) -> usize {
        self.best_so_far.as_ref().map_or(self.original_size, |(cost, _)| *cost)
    }

    /// Records `pair` (cost `cost`) as the new best, found at `step`.
    fn record_best(&mut self, step: usize, cost: usize, pair: SearchStateWithCostSelection<F, O>) {
        self.best_so_far = Some((cost, pair));
        self.best_found_at = Some(step);
    }

    /// `--follow` mode: registers `progress` prefix nodes seen at `step`. If it
    /// beats the deepest prefix seen so far, treats it as improvement (bumps
    /// `best_found_at`) so the dead-runs check keeps the search alive.
    fn note_prefix_progress(&mut self, step: usize, progress: usize) {
        if progress > self.best_prefix_progress {
            self.best_prefix_progress = progress;
            self.best_found_at = Some(step);
        }
    }

    /// The step the last improvement happened, or 0 if none yet — the baseline
    /// the dead-runs stopping rule measures staleness from.
    fn last_improvement_step(&self) -> usize {
        self.best_found_at.unwrap_or(0)
    }

    /// The step at which the best result was found, if any (for reporting).
    fn best_found_at(&self) -> Option<usize> {
        self.best_found_at
    }

    /// Peeks at the current best `(cost, state + selection)`, if any.
    fn best(&self) -> &BestSoFar<F, O> {
        &self.best_so_far
    }

    /// Canonicalizes the winner's variable numbering in place (no-op if no
    /// best has been recorded). Run before output.
    fn canonicalize_best(&mut self) {
        if let Some((_, pair)) = self.best_so_far.as_mut() {
            pair.canonicalize();
        }
    }

    /// Consumes the driver, returning the best particle and the underlying
    /// shared data (the e-graph handed back to the abstraction loop).
    fn into_parts(self) -> (BestSoFar<F, O>, crate::shared::SharedData<F, O>) {
        (self.best_so_far, self.shared.into_data())
    }
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
    // A non-positive temperature makes every weight `-cost/T` non-finite
    // (`-inf`, or `NaN` for a zero-cost particle), collapsing the whole run to
    // "all particles died"; a negative one would silently invert the objective.
    assert!(temperature > 0.0 && temperature.is_finite(), "--temperature must be a positive finite number, got {temperature}");
    // A negative boost yields negative resampling weights, which make the
    // cumulative array non-monotonic and break `partition_point` in
    // `weighted_choice`. Zero is allowed.
    assert!(args.boost_reuse_weight >= 0.0 && args.boost_reuse_weight.is_finite(), "--boost-reuse-weight must be a non-negative finite number, got {}", args.boost_reuse_weight);
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;
    let no_zero_arity = args.no_zero_arity;
    let verbose = args.verbose;

    let mut steps_run = 0;
    let debug = args.debug_log;
    let mut debug_steps: Vec<StepLog> = Vec::new();

    // Bundle the shared context, args, rng, and best-so-far bookkeeping into one
    // driver. All best-so-far access goes through its methods.
    let mut search = SmcSearchData::new(shared, args, rng, original_size);

    let mut particles: Vec<(SearchState<F, O>, usize)> = vec![(SearchState::new(&search.shared, false), num_particles)];
    let mut scratch = CostScratch::new(&search.shared.egraph);
    let mut dominance_hits: usize = 0;
    let mut useless_inline_hits: usize = 0;
    let mut lower_bound_pruner = LowerBoundPruner::new(args.opt_lower_bound);

    for step in 0..num_steps {
        let expanded = search.expand_particles(std::mem::take(&mut particles), &mut dominance_hits, &mut useless_inline_hits);

        // Per-particle: optional lower-bound prune, else full cost; running
        // `best_so_far` update inline so later particles in the same step can
        // benefit from a tighter `cost_to_beat`.
        let mut costs: Vec<usize> = Vec::with_capacity(expanded.len());
        let mut pruned: Vec<bool> = Vec::with_capacity(expanded.len());
        for s in expanded.iter() {
            let cost_to_beat: usize = search.cost_to_beat();
            if let PruneResult::Pruned = lower_bound_pruner.try_prune(&search.shared.egraph, search.shared.root, &cost_cache, &mut scratch, s, cost_to_beat) {
                costs.push(cost_to_beat);
                pruned.push(true);
                continue;
            }
            let selection = compute_cost_and_select(&search.shared.egraph, search.shared.root, &cost_cache, &mut scratch, s, search.shared.check_slow);
            let cost = selection.cost;
            let arity = s.pattern.vars.len();
            // In `--follow` mode the prefix filter lets cheaper non-matching
            // particles through, so skip the prefix-best update — only the
            // exact-match exit below promotes a particle to `best`.
            if search.shared.follow.is_none() && arity <= max_arity && !(no_zero_arity && arity == 0) && cost < cost_to_beat && !s.has_useless_var(&search.shared) {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), s.pattern.to_string().cyan());
                search.record_best(step, cost, SearchStateWithCostSelection { state: s.clone(), selection });
            }
            costs.push(cost);
            pruned.push(false);
        }

        let mut weights = match search.reweight(&expanded, &costs, &pruned, step) {
            ReweightOutcome::FollowExactMatch { index, cost, probs } => {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("follow exact match: {}", cost).green().bold(), expanded[index].pattern.to_string().cyan());
                // Re-derive the selection for this particle: we didn't keep
                // selections in `costs`, and exact-match fires at most once.
                let selection = compute_cost_and_select(&search.shared.egraph, search.shared.root, &cost_cache, &mut scratch, &expanded[index], search.shared.check_slow);
                search.record_best(step, cost, SearchStateWithCostSelection { state: expanded[index].clone(), selection });
                steps_run = step + 1;
                log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &probs, search.best(), &[]);
                break;
            }
            ReweightOutcome::Weights(weights) => weights,
        };

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, search.best(), &[]);
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        // Steps since last improvement; if no improvement yet, measure from step 0
        // so a fully stuck search still aborts after `dead_runs`. Use `>=` so we
        // stop after exactly `dead_runs` no-improvement steps, per the CLI help.
        if (step as i64) - (search.last_improvement_step() as i64) >= dead_runs as i64 {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, search.best(), &[]);
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        if verbose {
            println!("{}", format!("Step {}: expanded all particles", step).dimmed());
            print_top_particles(&expanded, &weights, &search.shared, original_size, |i| costs[i]);
        }

        let weights_acc = normalize_and_accumulate(&mut weights);
        let mut counts: Vec<usize> = vec![0; expanded.len()];
        let resample_indices: Vec<usize> = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights_acc, search.rng);
                counts[idx] += 1;
                idx
            })
            .collect();

        if debug {
            debug_steps.push(StepLog {
                step,
                particles: build_particle_logs(&expanded, &costs, &weights),
                resample_indices,
                best_cost: search.best().as_ref().map(|(c, _)| *c),
                best_pattern: search.best().as_ref().map(|(_, b)| b.state.pattern.to_string()),
            });
        }

        if verbose {
            println!("{}", format!("Step {}: resampled all particles", step).dimmed());
            let resample_weights: Vec<f64> = counts.iter().map(|&c| c as f64 / num_particles as f64).collect();
            print_top_particles(&expanded, &resample_weights, &search.shared, original_size, |i| costs[i]);
        }

        particles = expanded.into_iter().zip(counts).filter(|(_, c)| *c > 0).collect();
        steps_run = step + 1;
    }

    println!("\n{}", "═══ STATS ═══".blue().bold());
    println!("{} {}", "dominance hits:".dimmed(), dominance_hits.to_string().bold());
    println!("{} {}", "useless-inline hits:".dimmed(), useless_inline_hits.to_string().bold());
    lower_bound_pruner.print_stats();

    // Canonicalize the winner's var numbering (DFS first-appearance) before
    // output.
    search.canonicalize_best();

    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, best))) = (search.best_found_at(), search.best().as_ref()) {
        let state = &best.state;
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
    let best_found_at = search.best_found_at();
    let (best, data) = search.into_parts();
    SmcResult {
        best,
        original_size,
        best_found_at,
        num_steps_run: steps_run,
        data,
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
/// `[acc_weights[i-1], acc_weights[i])` (treating the implicit prefix as 0):
/// the smallest `i` with `acc_weights[i] > r`. Closed on the left, open on the
/// right — `partition_point(|w| w <= r)` returns exactly this.
///
/// Uses `partition_point` so that zero-weight prefixes (entries whose
/// cumulative value equals a previous one) are skipped, and clamps to the
/// last index to defend against float round-off leaving the final
/// accumulator slightly below 1.0.
pub fn index_from_cumulative(acc_weights: &[f64], r: f64) -> usize {
    assert!(!acc_weights.is_empty(), "index_from_cumulative requires a non-empty slice");
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
    assert!(!weights.is_empty(), "normalize_and_accumulate requires a non-empty slice");
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
