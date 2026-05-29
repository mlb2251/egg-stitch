use crate::cost::{CostCache, CostScratch, compute_lower_bound, compute_pattern_size};
use crate::lang::{LanguageFamily, StitchEgraph, StitchOp};
use crate::search::SearchState;
use colored::Colorize;
use egg::Id;
use std::time::{Duration, Instant};

/// Outcome of a lower-bound check on a candidate state.
pub enum PruneResult {
    /// Pruner is disabled; caller should proceed without a bound.
    Disabled,
    /// `lb >= cost_to_beat`; caller should drop this state.
    Pruned,
    /// Bound computed but does not prune; caller proceeds and may cache `lb`.
    Keep(usize),
}

/// Encapsulates the optional lower-bound pruning shared by best-first and SMC:
/// each candidate's optimistic descendant cost is compared against the current
/// best, and accumulated stats (hits + wall time) are reported at search end.
pub struct LowerBoundPruner {
    enabled: bool,
    hits: usize,
    time: Duration,
}

impl LowerBoundPruner {
    /// Builds a pruner; when `enabled` is false every `try_prune` call returns
    /// [`PruneResult::Disabled`] without computing a bound.
    pub fn new(enabled: bool) -> Self {
        Self { enabled, hits: 0, time: Duration::ZERO }
    }

    /// Computes `compute_lower_bound + pattern_size` for `state` and compares
    /// it against `cost_to_beat`. Returns whether to prune, keep, or skip the
    /// check entirely (when disabled).
    pub fn try_prune<F: LanguageFamily, O: StitchOp>(&mut self, egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, state: &SearchState<F, O>, cost_to_beat: usize) -> PruneResult {
        if !self.enabled {
            return PruneResult::Disabled;
        }
        let t = Instant::now();
        let lb = compute_lower_bound(egraph, root, cache, scratch, state) + compute_pattern_size(&state.pattern, &egraph.analysis.weights);
        self.time += t.elapsed();
        if lb >= cost_to_beat {
            self.hits += 1;
            PruneResult::Pruned
        } else {
            PruneResult::Keep(lb)
        }
    }

    /// Re-checks an already-cached `lb` against an updated `cost_to_beat`
    /// (best may have improved since the node was first inserted). Bumps the
    /// hit counter when it prunes.
    pub fn recheck_cached(&mut self, lb: usize, cost_to_beat: usize) -> bool {
        if lb >= cost_to_beat {
            self.hits += 1;
            true
        } else {
            false
        }
    }

    /// Prints the stats line shown in the `STATS` block of each search driver.
    pub fn print_stats(&self) {
        println!("{} {} {}", "lower-bound hits:".dimmed(), self.hits.to_string().bold(), format!("(time: {:.3}s)", self.time.as_secs_f64()).dimmed());
    }
}
