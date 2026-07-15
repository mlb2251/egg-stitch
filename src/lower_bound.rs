use crate::cost::{CostCache, CostScratch, compute_budget_prunable, compute_lower_bound};
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

    /// Computes the `compute_lower_bound` for `state` and compares it against
    /// `cost_to_beat`. Returns whether to prune, keep, or skip the check
    /// entirely (when disabled).
    pub fn try_prune<F: LanguageFamily, O: StitchOp>(&mut self, egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, state: &SearchState<F, O>, cost_to_beat: usize) -> PruneResult {
        if !self.enabled {
            return PruneResult::Disabled;
        }
        let t = Instant::now();
        let lb = compute_lower_bound(egraph, root, cache, scratch, state);
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

/// Verdict from [`BudgetPruner::prune`].
pub enum BudgetVerdict {
    /// Pruner disabled; caller proceeds with no bound and an unmodified state.
    Disabled,
    /// The whole-node bound `lb_root >= cost_to_beat`; drop this node.
    PrunedNode,
    /// Every substitution was budget-dead; the abstraction is dead everywhere —
    /// drop this node.
    Emptied,
    /// Kept: `state.matches` was filtered in place to the live substs. Carries
    /// the node bound for the heap's cached-lb recheck.
    Keep(usize),
}

/// Version-B budget pruning, applied for real. At each child it runs the
/// factoring-preserving per-subst prune ([`compute_budget_prunable`]) and
/// **mutates the state in place**, dropping every substitution that can never be
/// on a plausible min term. The same single optimistic fixpoint also yields the
/// whole-node lower bound, so this subsumes [`LowerBoundPruner`] when enabled —
/// the node-level prune costs nothing extra. Sound because pruned substs are
/// provably dead (monotone: their stub only grows, their budget only shrinks, so
/// they stay dead in every descendant — which is why the filtered set is safe to
/// store and inherit).
pub struct BudgetPruner {
    enabled: bool,
    /// Nodes dropped by the whole-node bound.
    node_hits: usize,
    /// Nodes dropped because every substitution was pruned.
    emptied: usize,
    /// Total substitutions seen across surviving nodes.
    substs_total: usize,
    /// Substitutions removed in surviving nodes.
    substs_pruned: usize,
    time: Duration,
}

impl BudgetPruner {
    /// Builds a pruner; when `enabled` is false every `prune` call returns
    /// [`BudgetVerdict::Disabled`] and leaves the state untouched.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            node_hits: 0,
            emptied: 0,
            substs_total: 0,
            substs_pruned: 0,
            time: Duration::ZERO,
        }
    }

    /// Prunes `state` in place. On [`BudgetVerdict::Keep`] the caller proceeds
    /// with the (now smaller) match set and caches the returned bound; on
    /// `PrunedNode`/`Emptied` it drops the node.
    pub fn prune<F: LanguageFamily, O: StitchOp>(&mut self, egraph: &StitchEgraph<F::Apply<O>>, root: Id, cache: &CostCache, scratch: &mut CostScratch, state: &mut SearchState<F, O>, cost_to_beat: usize) -> BudgetVerdict {
        if !self.enabled {
            return BudgetVerdict::Disabled;
        }
        let t = Instant::now();
        let bp = compute_budget_prunable(egraph, root, cache, scratch, state);
        let verdict = if bp.lb_root >= cost_to_beat {
            self.node_hits += 1;
            BudgetVerdict::PrunedNode
        } else if bp.matches.is_empty() {
            self.emptied += 1;
            BudgetVerdict::Emptied
        } else {
            self.substs_total += bp.substs_before;
            self.substs_pruned += bp.substs_before - bp.substs_after;
            state.num_substs = bp.matches.iter().map(|m| m.num_substs()).sum();
            state.matches = bp.matches;
            BudgetVerdict::Keep(bp.lb_root)
        };
        self.time += t.elapsed();
        verdict
    }

    /// Prints the stats line shown in the `STATS` block.
    pub fn print_stats(&self) {
        if !self.enabled {
            return;
        }
        println!(
            "{} {} {}",
            "budget-prune (B):".dimmed(),
            format!("{}/{} substs pruned", self.substs_pruned, self.substs_total).bold(),
            format!("(nodes pruned: {}, nodes emptied: {}, time: {:.3}s)", self.node_hits, self.emptied, self.time.as_secs_f64()).dimmed(),
        );
    }
}
