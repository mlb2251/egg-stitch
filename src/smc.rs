use colored::Colorize;

use crate::cost::{CostCache, CostScratch, SearchStateWithCostSelection, compute_cost_and_select};
use crate::lang::{LanguageFamily, StitchOp};
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::lower_bound::{LowerBoundPruner, PruneResult};
use crate::math::logaddexp;
use crate::pattern::Pattern;
use crate::search::{Action, EnumStats, SearchState, SharedSearchData, SuccessorEnum, setup_search};
use rand::Rng;
use rand::rngs::StdRng;
use rustc_hash::FxHashMap;

/// Inserts a freshly-expanded state into the parallel (states, mults, props)
/// deduped-by-pattern buffer, accumulating into an existing group or pushing a new
/// one.
///
/// Two quantities are accumulated per unique child pattern `t_j'`:
/// * `mults[j] += count` — the realized child multiplicity `C_j'` (the number of
///   particles that landed on `t_j'`, a multinomial outcome of the proposal
///   sampling).
/// * `props[j] += prop` — the proposal mass routed to `t_j'`,
///   `Σ_l C_l · K_n(t_l, t_j')`, summed over every parent `t_l` (multiplicity `C_l`)
///   that reaches it. This is the empirical proposal marginal (Del Moral, Doucet &
///   Jasra 2006, §2.4, eq. 9) the optimal-backward-kernel weight divides by — see
///   [`smc`] and [`SmcSearchData::reweight`].
fn dedup_insert<F: LanguageFamily, O: StitchOp>(s: SearchState<F, O>, count: usize, prop: f64, states: &mut Vec<SearchState<F, O>>, mults: &mut Vec<usize>, props: &mut Vec<f64>, dedup: &mut FxHashMap<Pattern<F, O>, usize>) {
    match dedup.get(&s.pattern) {
        Some(&idx) => {
            mults[idx] += count;
            props[idx] += prop;
        }
        None => {
            let idx = states.len();
            dedup.insert(s.pattern.clone(), idx);
            states.push(s);
            mults.push(count);
            props.push(prop);
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
    /// Step at which the best particle was last recorded; the dead-runs stopping
    /// rule (non-`--follow` mode) measures staleness from here.
    best_found_at: Option<usize>,
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
    /// across groups, so the returned vecs hold one entry per unique pattern.
    /// `enum_stats` accumulates the run-wide enumeration-stage pruning tallies
    /// reported at the end (see [`EnumStats`]).
    ///
    /// Returns `(states, mults, props)`: alongside each unique child `t_j'` it
    /// reports the realized child multiplicity `C_j'` (`mults`) and the proposal
    /// mass `Σ_l C_l · K_n(t_l, t_j')` (`props`) — the denominator the optimal-
    /// backward-kernel weight in [`Self::reweight`] divides by.
    fn expand_particles(&mut self, particles: Vec<(SearchState<F, O>, usize)>, enum_stats: &mut EnumStats) -> (Vec<SearchState<F, O>>, Vec<usize>, Vec<f64>) {
        let max_match_set = self.args.max_match_set;
        let mut expanded: Vec<SearchState<F, O>> = Vec::new();
        // Per-unique-pattern child multiplicities `C_j'` (the multinomial outcome
        // of the proposal sampling) and proposal masses `Σ_l C_l · K_n(t_l, t_j')`
        // (the denominator the reweight divides by). Both accumulate across parents
        // during dedup.
        let mut mults: Vec<usize> = Vec::new();
        let mut props: Vec<f64> = Vec::new();
        let mut dedup: FxHashMap<Pattern<F, O>, usize> = FxHashMap::default();
        for (state, mult) in particles {
            let actions = match state.enumerate_successor_actions(&self.shared, self.args.opt_dominance_reuse, self.args.opt_useless_inline, usize::MAX, enum_stats) {
                SuccessorEnum::Dominant { child, .. } => {
                    // Deterministic successor: K_n(t_l, t_j') = 1, so the proposal
                    // mass routed here is C_l · 1 = mult.
                    if child.within_match_set_cap(max_match_set) {
                        dedup_insert(child, mult, mult as f64, &mut expanded, &mut mults, &mut props, &mut dedup);
                    }
                    continue;
                }
                // SMC keeps creation order (`freeze_rule = false`), so the freeze
                // rule is inert — `apply_action` ignores `rank` below.
                SuccessorEnum::All { actions, .. } => actions,
            };
            if actions.is_empty() {
                // Terminal particle carried forward unchanged: K_n = 1.
                dedup_insert(state, mult, mult as f64, &mut expanded, &mut mults, &mut props, &mut dedup);
                continue;
            }
            let mut weights = action_weights_with_reuse_boost(&actions, self.args.boost_reuse_weight);
            // `sample_counts` normalizes `weights` in place, so afterwards
            // `weights[k]` holds the proposal kernel value K_n(t_l, t_j') for the
            // action a_k leading to child `t_j'`.
            let counts = sample_counts(&mut weights, mult, self.rng);
            for (((action, _), count), q) in actions.into_iter().zip(counts).zip(weights) {
                if count > 0 {
                    let child = state.apply_action(&action, &self.shared, true, None);
                    if child.within_match_set_cap(max_match_set) {
                        // Proposal mass from this parent: C_l · K_n(t_l, t_j') =
                        // mult · q. Accumulated independent of the realized `count`
                        // so the reweight divides by the true proposal mass
                        // `Σ_l C_l · K_n(t_l, t_j')`, not the realized-count
                        // self-estimate (which would cancel the multiplicity).
                        dedup_insert(child, count, mult as f64 * q, &mut expanded, &mut mults, &mut props, &mut dedup);
                    }
                }
            }
        }
        (expanded, mults, props)
    }

    /// Computes each expanded particle's cost, lower-bound-pruning where it can,
    /// and updates `best_so_far` inline so later particles in the same step see a
    /// tighter `cost_to_beat`. Returns parallel `(costs, pruned)` vecs: `pruned[i]`
    /// marks particles killed by the lower bound (no descendant can beat the
    /// current best), whose `costs[i]` is the bound they failed rather than a
    /// real cost. In `--follow` mode the best update is skipped — only the
    /// exact-match exit promotes a particle.
    fn compute_costs(&mut self, expanded: &[SearchState<F, O>], step: usize, cost_cache: &CostCache, scratch: &mut CostScratch, lower_bound_pruner: &mut LowerBoundPruner) -> (Vec<usize>, Vec<bool>) {
        let max_arity = self.args.max_arity;
        let no_zero_arity = self.args.no_zero_arity;
        let mut costs: Vec<usize> = Vec::with_capacity(expanded.len());
        let mut pruned: Vec<bool> = Vec::with_capacity(expanded.len());
        for s in expanded.iter() {
            let cost_to_beat: usize = self.cost_to_beat();
            if let PruneResult::Pruned = lower_bound_pruner.try_prune(&self.shared.egraph, self.shared.root, cost_cache, scratch, s, cost_to_beat) {
                costs.push(cost_to_beat);
                pruned.push(true);
                continue;
            }
            let selection = compute_cost_and_select(&self.shared.egraph, self.shared.root, cost_cache, scratch, s, self.shared.check_slow);
            let cost = selection.cost;
            let arity = s.pattern.vars.len();
            // In `--follow` mode the prefix filter lets cheaper non-matching
            // particles through, so skip the prefix-best update — only the
            // exact-match exit promotes a particle to `best`.
            if self.shared.follow.is_none() && arity <= max_arity && !(no_zero_arity && arity == 0) && cost < cost_to_beat && !s.has_useless_var(&self.shared) {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), s.pattern.to_string().cyan());
                self.record_best(step, cost, SearchStateWithCostSelection { state: s.clone(), selection });
            }
            costs.push(cost);
            pruned.push(false);
        }
        (costs, pruned)
    }

    /// Normalizes the per-pattern weights `v(t_j')` into the resampling
    /// distribution via log-sum-exp. `v(t_j')` is the deduplicated, optimal-
    /// backward-kernel importance weight derived in [`smc`] (Del Moral, Doucet &
    /// Jasra 2006, §3.3.1, Proposition 1, eq. 21):
    /// `v(t_j') = C_j' · exp(-cost_j / T) / [Σ_l C_l K_n(t_l, t_j')]`, i.e. the
    /// log-weight `ln(mults[j]) - cost_j/T - ln(props[j])`, where
    /// `mults[j] = C_j'` is the realized child multiplicity and
    /// `props[j] = Σ_l C_l K_n(t_l, t_j')` is the proposal mass routed to `t_j'`.
    /// Dividing by that proposal mass is what keeps the cloud on the fixed target
    /// `exp(-cost(x)/T)` rather than drifting with the proposal. Pruned particles
    /// get `-inf`. Returns probabilities summing to 1, or an all-zero vector if
    /// every particle is dead (the caller treats that as "all particles died").
    fn reweight(&self, mults: &[usize], props: &[f64], costs: &[usize], pruned: &[bool]) -> Vec<f64> {
        let temperature = self.args.temperature;
        let log_weights: Vec<f64> = costs.iter().enumerate().map(|(i, c)| if pruned[i] { f64::NEG_INFINITY } else { (mults[i] as f64).ln() - (*c as f64) / temperature - props[i].ln() }).collect();
        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        if total_weight.is_finite() { log_weights.iter().map(|lw| (lw - total_weight).exp()).collect() } else { vec![0.0; log_weights.len()] }
    }

    /// Resamples the next generation from `weights`: draws `num_particles`
    /// indices proportional to the normalized weights, then returns each
    /// `expanded` particle paired with how many draws it won (its multiplicity),
    /// dropping particles that won zero. `costs`/`step` feed the verbose dump.
    fn resample(&mut self, expanded: Vec<SearchState<F, O>>, weights: Vec<f64>, costs: &[usize], step: usize) -> Vec<(SearchState<F, O>, usize)> {
        let num_particles = self.args.num_particles;
        let counts = sample_counts_normalized(&weights, num_particles, self.rng);

        if self.args.verbose {
            println!("{}", format!("Step {}: resampled all particles", step).dimmed());
            let resample_weights: Vec<f64> = counts.iter().map(|&c| c as f64 / num_particles as f64).collect();
            print_top_particles(&expanded, &resample_weights, &self.shared, self.original_size, |i| costs[i]);
        }

        expanded.into_iter().zip(counts).filter(|(_, c)| *c > 0).collect()
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

/// Runs an SMC sampler to find a pattern that minimizes compressed corpus size.
///
/// Follows the construction given  in https://www.stats.ox.ac.uk/~doucet/delmoral_doucet_jasra_sequentialmontecarlosamplersJRSSB.pdf
///
/// Specifically, we use 2.3.1(c) Equation 8; we wish to have π(x) ∝ exp(-cost(x)/T)
///
/// First, we provide an exposition without multiplicity:
///
/// At step n we hold N particles x_1, …, x_N. One step is expand → weight →
/// resample.
///
/// Since we resample at every step, we can assume γ_n(x_i) ∝ η_n(x_i) for all n and i.
///
/// 1. Expand: Sample x_i' ∼ K_n(x_i, x_i') via the kernel
/// 2. **Weight.** The unnormalized incremental importance weight (eq. 12) is
///    ~w_n(x_i, x_i') = [γ_n(x_i') · L_{n-1}(x_i', x_i)] / [γ_{n-1}(x_i) · K_n(x_i, x_i')]
///    where γ is the target distribution
///    γ_n(x) = exp(-cost(x)/T)
///    Taking the *optimal* backward kernel L_{n-1}^opt as defined by (eq. 21) as
///    L_{n-1}(x_i', x_i) = [η_{n-1}(x_i) · K_n(x_i, x_i')] / η_n(x_i')
///    ~w_n(x_i, x_i') = [γ_n(x_i') · η_{n-1}(x_i)] / [γ_{n-1}(x_i) · η_n(x_i')]
///    = [γ_n(x_i') / η_n(x_i')] · [η_{n-1}(x_i) / γ_{n-1}(x_i)]
///    the paper says that this gives us w_n(x_i') ∝ γ_n(x_i') / η_n(x_i')
///
///    Now, η_n(x') = η_{n-1} K_n (x') is the proposal marginal (eq. 9).
///    η_n has no closed form, so we estimate it empirically as
///    η_{n-1}^N K_n(x') = (1/N) Σ_k K_n(x_k, x') (the equation after eq. 9)
///    This leads us to the final per-particle weight
///    ~w_n(x_i') ∝ exp(-cost(x_i')/T) / [Σ_k K_n(x_k, x_i')]
/// 3. **Resample.** Normalize to `α_i = ~w_n(x_i') / Σ_k ~w_n(x_k')` and draw N new
///    particles ∝ α.
///
/// Now, accounting for multiplicity via deduplication:
/// We want to make sure that w_n(x_i') ∝ exp(-cost(x_i')/T) / [Σ_k K_n(x_k, x_i')]
/// is preserved. Let t_1...t_k be the deduplicated patterns, let S(j) = {i | x_i = t_j}
/// be the set of particles that collapsed to t_j, and let C_j = |S(j)| be the realized multiplicity of t_j.
/// Define the same for t_j'.
///
/// Then let v(t_j')
///     = sum_{i in S'(j)} w_n(x_i')
///     = sum_{i in S'(j)} exp(-cost(x_i')/T) / [Σ_k K_n(x_k, x_i')]
///     = C_j' · exp(-cost(t_j')/T) / [Σ_l C_l K_n(t_l, t_j')]
/// where C_j' = |S'(j)| is the child multiplicity (the Σ over S'(j) collapses to it
/// since every term is equal) and C_l = |S(l)| is the parent multiplicity.
///
/// So the denominator term here can be computed by summing the proposal mass routed to t_j' from each parent t_l, weighted by the realized multiplicity C_l.
///
/// We further assume that K_n(t_l, t_j') = 0 for all transitions that are not realized in the sample, so we
/// only accumulate over the parents that actually reached t_j'.
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
    let verbose = args.verbose;

    let mut steps_run = 0;

    // Bundle the shared context, args, rng, and best-so-far bookkeeping into one
    // driver. All best-so-far access goes through its methods.
    let mut search = SmcSearchData::new(shared, args, rng, original_size);

    let mut particles: Vec<(SearchState<F, O>, usize)> = vec![(SearchState::new(&search.shared, false), num_particles)];
    let mut scratch = CostScratch::new(&search.shared.egraph);
    let mut enum_stats = EnumStats::default();
    let mut lower_bound_pruner = LowerBoundPruner::new(args.opt_lower_bound);

    for step in 0..num_steps {
        let (expanded, mults, props) = search.expand_particles(std::mem::take(&mut particles), &mut enum_stats);

        let (costs, mut pruned) = search.compute_costs(&expanded, step, &cost_cache, &mut scratch, &mut lower_bound_pruner);

        // Follow stage: kill non-matching particles directly in `pruned` so the
        // `reweight` below drops them, and stop on an exact match. Kept inline
        // (not in `reweight`) because the exact-match exit owns the loop's
        // `break` / `steps_run` / logging.
        if let Some(ref follow) = search.shared.follow {
            if !apply_follow_constraint(&expanded, &mut pruned, follow) {
                println!("{}", "No particles match the follow pattern".red().bold());
            }
            // If any surviving particle is alpha-equivalent to the follow target,
            // the search has reached the goal — pick the cheapest such particle
            // and stop. Compute the match before `record_best` so `follow`'s
            // borrow of `search.shared` is released before the bookkeeping method
            // takes `&mut search`.
            let exact = (0..expanded.len()).filter(|&i| !pruned[i] && crate::follow::matches_follow_serialized(&expanded[i], follow, &search.shared.egraph)).map(|i| (i, costs[i])).min_by_key(|&(_, c)| c);
            if let Some((index, cost)) = exact {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("follow exact match: {}", cost).green().bold(), expanded[index].pattern.to_string().cyan());
                // Re-derive the selection for this particle: we didn't keep
                // selections in `costs`, and exact-match fires at most once.
                let selection = compute_cost_and_select(&search.shared.egraph, search.shared.root, &cost_cache, &mut scratch, &expanded[index], search.shared.check_slow);
                search.record_best(step, cost, SearchStateWithCostSelection { state: expanded[index].clone(), selection });
                steps_run = step + 1;
                break;
            }
        }

        // Zero-arity patterns can't be further expanded.
        for (i, s) in expanded.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                pruned[i] = true;
            }
        }

        let weights = search.reweight(&mults, &props, &costs, &pruned);

        if weights.iter().sum::<f64>() == 0.0 {
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        // `--follow` mode has no cost objective (the prefix filter lets cheaper
        // non-matching particles through), so the no-improvement heuristic
        // doesn't apply — a follow run stops only on exact match, all particles
        // dying, or the `num_steps` cap. Otherwise: steps since the last
        // improvement; if none yet, measure from step 0 so a fully stuck search
        // still aborts after `dead_runs`. `>=` stops after exactly `dead_runs`
        // no-improvement steps, per the CLI help.
        if search.shared.follow.is_none() && (step as i64) - (search.last_improvement_step() as i64) >= dead_runs as i64 {
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        if verbose {
            println!("{}", format!("Step {}: expanded all particles", step).dimmed());
            print_top_particles(&expanded, &weights, &search.shared, original_size, |i| costs[i]);
        }

        particles = search.resample(expanded, weights, &costs, step);
        steps_run = step + 1;
    }

    println!("\n{}", "═══ STATS ═══".blue().bold());
    println!("{} {}", "dominance hits:".dimmed(), enum_stats.dominance_hits.to_string().bold());
    println!("{} {}", "useless-inline hits:".dimmed(), enum_stats.useless_inline_hits.to_string().bold());
    // max-arity / frozen-var / reuse-order are inert under SMC (max_arity = MAX,
    // freeze_rule = false); zero-support can still fire.
    println!("{} {}", "zero-support hits:".dimmed(), enum_stats.zero_support_hits.to_string().bold());
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

    let best_found_at = search.best_found_at();
    let (best, data) = search.into_parts();
    SmcResult {
        best,
        original_size,
        best_found_at,
        num_steps_run: steps_run,
        data,
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

/// Draws `draws` independent samples from the categorical distribution given by
/// the un-normalized `weights`, returning how many times each index was chosen.
/// `weights` is normalized in place in the process.
pub fn sample_counts(weights: &mut [f64], draws: usize, rng: &mut StdRng) -> Vec<usize> {
    let acc = normalize_and_accumulate(weights);
    counts_from_cumulative(&acc, draws, rng)
}

/// Draws `draws` independent samples from an already-normalized probability
/// slice `probs` (must sum to 1), returning how many times each index was
/// chosen. Skips the normalization pass that `sample_counts` does.
pub fn sample_counts_normalized(probs: &[f64], draws: usize, rng: &mut StdRng) -> Vec<usize> {
    counts_from_cumulative(&accumulate(probs), draws, rng)
}

/// Draws `draws` samples from the cumulative distribution `acc`, returning
/// per-index counts.
fn counts_from_cumulative(acc: &[f64], draws: usize, rng: &mut StdRng) -> Vec<usize> {
    let mut counts: Vec<usize> = vec![0; acc.len()];
    for _ in 0..draws {
        counts[weighted_choice(acc, rng)] += 1;
    }
    counts
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
    accumulate(weights)
}

/// Builds a cumulative distribution from `weights` (assumed to sum to 1).
fn accumulate(weights: &[f64]) -> Vec<f64> {
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
