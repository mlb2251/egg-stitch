use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchOp};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use rand::Rng;
use rand::rngs::StdRng;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Tracks already-explored canonical patterns to dedupe successors during
/// search. Accumulates hit count and time spent so the host loop can report
/// stats. Wrap in `Option<…>` at the call site — `None` disables the check
/// entirely (useful for measuring how much pruning the seen-set buys).
pub struct SeenTracker<F: LanguageFamily, O: StitchOp> {
    set: FxHashSet<Pattern<F, O>>,
    pub hits: usize,
    pub time: Duration,
}

impl<F: LanguageFamily, O: StitchOp> Default for SeenTracker<F, O> {
    fn default() -> Self {
        Self {
            set: FxHashSet::default(),
            hits: 0,
            time: Duration::ZERO,
        }
    }
}

impl<F: LanguageFamily, O: StitchOp> SeenTracker<F, O> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Number of distinct patterns recorded.
    pub fn len(&self) -> usize {
        self.set.len()
    }
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
    /// Records `pattern` if new; returns `true` if it was already present
    /// (caller should skip this successor).
    pub fn check_and_insert(&mut self, pattern: Pattern<F, O>) -> bool {
        let t = Instant::now();
        let already_present = !self.set.insert(pattern);
        self.time += t.elapsed();
        if already_present {
            self.hits += 1;
        }
        already_present
    }
}

/// True iff `target` is a free De Bruijn variable leaf with index `i ≥ d_k`.
fn target_is_free_db_var<L: Language>(target: &L, d_k: u32) -> bool
where
    L::Discriminant: StitchDisc,
{
    target.children().is_empty() && target.discriminant().de_bruijn_index().is_some_and(|i| i >= 0 && (i as u32) >= d_k)
}

/// True iff `target` cannot be expanded to in a literal expansion.
/// Currently the only such case is a free DB-var leaf.
fn invalid_literal_expansion<L: Language>(target: &L, depth: u32) -> bool
where
    L::Discriminant: StitchDisc,
{
    target_is_free_db_var(target, depth)
}

/// A deterministic move taken at a search node: either expanding a pattern variable
/// with a specific enode shape, or unifying two existing variables. Doubles as
/// the canonical dedup key for sampled expansions: two samples that yield the
/// same `Action` produce identical resulting states.
///
/// Parameterized on the discriminant type `D` (rather than `(F, O)`) so the
/// derived `Hash`/`Eq` bounds land on `D: StitchDisc` and don't leak onto `F`.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Action<D> {
    Expand { var_idx: usize, op: D, arity: usize },
    Reuse { keep: usize, drop: usize },
}

impl<D: std::fmt::Display> std::fmt::Display for Action<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Expand { var_idx, op, arity } => write!(f, "expand #{} := {}/{}", var_idx, op, arity),
            Action::Reuse { keep, drop } => write!(f, "reuse #{} = #{}", keep, drop),
        }
    }
}

/// Shared read-only context passed to all search operations.
#[derive(Debug)]
pub struct SharedSearchData<F: LanguageFamily, O: StitchOp> {
    pub egraph: StitchEgraph<F::Apply<O>>,
    /// Root e-class of the corpus (the `(programs ...)` wrapper). Excluded
    /// from the initial match set so patterns can't be rooted there.
    pub root: Id,
    /// Follow pattern: particles whose pattern isn't a valid prefix get zero
    /// weight at the resample step.
    pub follow: Option<RevExpr<F::Apply<OpWithVar<O>>>>,
    /// Probability of attempting variable reuse during expansion.
    pub p_reuse: f64,
    /// Enable slow rewrite check (assert fast == slow computation).
    pub check_slow: bool,
    /// Whether to weight match selection by usage count during expansion.
    pub weight_by_usage: bool,
    /// How many times each e-class is used in the fully-expanded corpus tree.
    pub usage_counts: FxHashMap<Id, usize>,
}

#[derive(Debug, Clone)]
pub struct SearchState<F: LanguageFamily, O: StitchOp> {
    pub pattern: Pattern<F, O>,
    // each match represents a different eclass at which `pattern` can be rooted
    pub matches: Vec<MatchAtEClass>,
    /// Cached `sum(m.substs.len() for m in matches)`. Used by the dominance
    /// check in `enumerate_successors` to detect reuses that preserve the
    /// match set's size (and are therefore strictly dominant successors).
    pub num_substs: usize,
}

/// Computes the total number of substitutions across all matches.
fn total_substs(matches: &[MatchAtEClass]) -> usize {
    matches.iter().map(|m| m.substs.len()).sum()
}

impl<F: LanguageFamily, O: StitchOp> SearchState<F, O> {
    /// Randomly samples a single expansion or reuse step without mutating self.
    /// Returns `None` when the chosen variable's e-class has no valid literal
    /// expansion candidates (the caller treats this as a no-op leaving the
    /// state unchanged). The returned `Action` is also a hashable dedup key,
    /// so callers can collapse identical samples and apply each unique one once.
    pub fn sample_random_expansion(&self, shared: &SharedSearchData<F, O>, verbose: bool, rng: &mut StdRng) -> Option<Action<F::Discriminant<O>>> {
        let match_idx = if shared.weight_by_usage {
            let mut weights: Vec<f64> = self.matches.iter().map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1) as f64).collect();
            let weights_acc = crate::smc::normalize_and_accumulate(&mut weights);
            crate::smc::weighted_choice(&weights_acc, rng)
        } else {
            rng.random_range(0..self.matches.len())
        };
        let m = &self.matches[match_idx];
        let extractor = if verbose { Some(egg::Extractor::new(&shared.egraph, egg::AstSize)) } else { None };
        if let Some(ref ext) = extractor {
            let (_cost, minimal_term) = ext.find_best(m.root_eclass);
            println!("Expanding on match at eclass {} with pattern {}", minimal_term, self.pattern);
        }
        let subst_idx = rng.random_range(0..m.substs.len());
        let subst = &m.substs[subst_idx];

        let var_idx = rng.random_range(0..self.pattern.vars.len());
        if verbose {
            println!("Expanding variable {:?} in pattern {}", self.pattern.vars[var_idx], self.pattern);
        }
        let target_id = subst.vars[var_idx];

        if let Some(ref ext) = extractor {
            println!("Target eclass is represented by minimal term {}", ext.find_best(target_id).1);
        }

        if rng.random_bool(shared.p_reuse) {
            // Pre-filter reuse candidates so we only invoke `reuse` when at
            // least one subst will survive `subset_matches_reuse`; otherwise
            // we'd empty the match set and panic on the next call.
            //
            // Cross-depth reuse soundness: the merged metavar's HO arity
            // must fit at *every* occurrence site, so its kept-eclass fv
            // must avoid the gap `[min(d_a, d_b), max(d_a, d_b))` — fv in
            // that range is η-wrappable at the deep site but unbound at
            // the shallow one. Same-depth reuse has an empty gap, so the
            // check is a no-op.
            let reuse_candidates: Vec<usize> = subst
                .vars
                .iter()
                .enumerate()
                .filter_map(|(idx, id)| {
                    if idx == var_idx || *id != target_id {
                        return None;
                    }
                    let keep_idx = idx.min(var_idx);
                    let d_a = self.pattern.var_depth[var_idx];
                    let d_b = self.pattern.var_depth[idx];
                    let min_depth = d_a.min(d_b);
                    let merged_depth = d_a.max(d_b);
                    let kept_fv = &shared.egraph[subst.vars[keep_idx]].data.fv;
                    if kept_fv.iter().all(|&i| i < min_depth as i32 || i >= merged_depth as i32) { Some(idx) } else { None }
                })
                .collect();
            if !reuse_candidates.is_empty() {
                let second_var_idx = reuse_candidates[rng.random_range(0..reuse_candidates.len())];
                return Some(Action::Reuse {
                    keep: var_idx.min(second_var_idx),
                    drop: var_idx.max(second_var_idx),
                });
            }
        }

        let target_eclass = &shared.egraph[target_id];
        let d_k = self.pattern.var_depth[var_idx];
        let candidates: Vec<&F::Apply<O>> = target_eclass.nodes.iter().filter(|n| !invalid_literal_expansion(*n, d_k)).collect();
        if candidates.is_empty() {
            return None;
        }
        let target = candidates[rng.random_range(0..candidates.len())];
        Some(Action::Expand {
            var_idx,
            op: target.discriminant(),
            arity: target.children().len(),
        })
    }

    /// Applies a previously-sampled `Action` to this state. `expand` and
    /// `subset_matches` only consult the target's discriminant and arity
    /// (egg's `Language::matches` ignores child ids), so we synthesize a
    /// placeholder node from `(op, arity)` via `F::make` instead of stashing
    /// the original enode.
    pub fn apply_action(&mut self, action: &Action<F::Discriminant<O>>, shared: &SharedSearchData<F, O>) {
        match action {
            Action::Expand { var_idx, op, arity } => {
                let target = F::make(op.clone(), vec![Id::from(0); *arity]);
                self.expand(*var_idx, &target, shared);
            }
            Action::Reuse { keep, drop } => self.reuse(*keep, *drop, shared),
        }
    }

    /// Check if this particle's pattern is a valid prefix of the follow target.
    pub fn matches_follow(&self, follow: &RevExpr<F::Apply<OpWithVar<O>>>) -> bool {
        let mut var_bindings = HashMap::new();
        crate::follow::check_follow::<F, O>(&self.pattern.pattern, Id::from(0), follow, Id::from(0), &mut var_bindings)
    }

    /// Expands the pattern at `var_idx` with `target` and filters matches accordingly.
    pub fn expand(&mut self, var_idx: usize, target: &F::Apply<O>, shared: &SharedSearchData<F, O>) {
        self.pattern.expand(var_idx, target);
        self.subset_matches(var_idx, target, shared);
    }

    /// Merges two pattern variables and filters matches to those where both point to the same e-class.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize, shared: &SharedSearchData<F, O>) {
        // Snapshot pre-merge depths: `subset_matches_reuse` needs both to
        // bound the cross-depth gap, but `pattern.reuse` collapses them.
        let d_a = self.pattern.var_depth[var_idx];
        let d_b = self.pattern.var_depth[second_var_idx];
        self.pattern.reuse(var_idx, second_var_idx);
        self.subset_matches_reuse(var_idx, second_var_idx, d_a.min(d_b), d_a.max(d_b), shared);
    }

    /// Updates all matches by transforming each substitution via the given closure,
    /// which may produce zero or more new substitutions per input. Removes matches
    /// with no remaining substitutions.
    fn update_matches(&mut self, mut f: impl FnMut(&Subst, &mut Vec<Subst>)) {
        for m in &mut self.matches {
            let mut new_substs: Vec<Subst> = vec![];
            for subst in &m.substs {
                f(subst, &mut new_substs);
            }
            m.substs = new_substs;
        }
        self.matches.retain(|m| !m.substs.is_empty());
        self.num_substs = total_substs(&self.matches);
    }

    /// Filters matches to those where `var_idx` can be expanded with `target`, updating substitutions.
    /// Mirrors `Pattern::expand`: drops the old var from `subst.vars` and inserts the new
    /// child eclass ids at positions `var_idx..var_idx+k`, keeping substs aligned with
    /// the pattern's DFS-ordered vars list.
    ///
    /// We don't fv-prune captures here: captures whose fv reaches into
    /// pattern-internal binders are handled at apply/cost time by η-wrapping
    /// (see `compute_ho_arity` and `shift_free_egraph`), so the match set
    /// stays permissive and search keeps exploring those branches.
    pub fn subset_matches(&mut self, var_idx: usize, target: &F::Apply<O>, shared: &SharedSearchData<F, O>) {
        self.update_matches(|subst, out| {
            let var_id = subst.vars[var_idx];
            let var_eclass = &shared.egraph[var_id];
            for node in &var_eclass.nodes {
                if !node.matches(target) {
                    continue;
                }
                let mut new_subst = subst.clone();
                new_subst.vars.remove(var_idx);
                for (j, child_id) in node.children().iter().enumerate() {
                    new_subst.vars.insert(var_idx + j, *child_id);
                }
                out.push(new_subst);
            }
        });
    }

    /// Filters matches to those where `var_idx` and `second_var_idx` point to the same e-class.
    /// Mirrors `Pattern::reuse`: keeps the lower-indexed var and removes the higher one,
    /// so substs stay aligned with the pattern regardless of caller argument order.
    ///
    /// Cross-depth soundness: the merged metavar appears at *both* original
    /// depths in the body. Its η-applied form `(?#k $0 … $(h-1))` requires
    /// `h` local pattern-internal binders at every site, so `h ≤ min_depth`.
    /// HO arity is `max{i + 1 : i ∈ kept_fv, i < merged_depth}`, so substs
    /// whose kept-eclass fv lands in `[min_depth, merged_depth)` are
    /// representable at the deep site but unbound at the shallow one — those
    /// are dropped. Same-depth reuse has an empty gap.
    pub fn subset_matches_reuse(&mut self, var_idx: usize, second_var_idx: usize, min_depth: u32, merged_depth: u32, shared: &SharedSearchData<F, O>) {
        let keep_idx = var_idx.min(second_var_idx);
        let drop_idx = var_idx.max(second_var_idx);
        self.update_matches(|subst, out| {
            if subst.vars[var_idx] != subst.vars[second_var_idx] {
                return;
            }
            let kept_fv = &shared.egraph[subst.vars[keep_idx]].data.fv;
            if !kept_fv.iter().all(|&i| i < min_depth as i32 || i >= merged_depth as i32) {
                return;
            }
            let mut new_subst = subst.clone();
            new_subst.vars.remove(drop_idx);
            out.push(new_subst);
        });
    }

    /// Creates the initial search state: a single-variable pattern matching every e-class.
    pub fn new(shared: &SharedSearchData<F, O>) -> Self {
        let matches = identity_matches(&shared.egraph, shared.root);
        let num_substs = total_substs(&matches);
        Self { pattern: Pattern::single_var(), matches, num_substs }
    }

    /// Enumerates every successor state reachable in one `expand` or `reuse` step.
    ///
    /// Reuse candidates are emitted first so the dominance short-circuit can fire:
    /// when a reuse(i, j) preserves `num_substs` (every subst already had the two
    /// vars equal), the resulting child match set is identical to the parent's
    /// modulo the var-merge, so any successor of the parent is reachable via this
    /// reuse — we can return it as the *only* successor and skip enumerating the
    /// rest. Disabled by `--no-opt-dominance-reuse`.
    ///
    /// Expansion candidates: for each variable, collect every distinct `(op, arity)`
    /// pair appearing as an enode in any bound e-class across all matches, then produce
    /// one child per shape. Children whose match set becomes empty after filtering
    /// are dropped.
    #[allow(clippy::type_complexity)]
    pub fn enumerate_successors(&self, shared: &SharedSearchData<F, O>, opt_dominance_reuse: bool, dominance_hits: &mut usize) -> Vec<(Action<F::Discriminant<O>>, SearchState<F, O>)> {
        let mut out = Vec::new();

        // Reuse pairs first — enables dominance short-circuit.
        let n = self.pattern.vars.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let unifiable = self.matches.iter().any(|m| m.substs.iter().any(|s| s.vars[i] == s.vars[j]));
                if !unifiable {
                    continue;
                }
                let mut child = self.clone();
                child.reuse(i, j, shared);
                if child.matches.is_empty() {
                    continue;
                }
                let action = Action::Reuse { keep: i, drop: j };
                let is_dominant = child.num_substs == self.num_substs;
                if opt_dominance_reuse && is_dominant {
                    *dominance_hits += 1;
                    return vec![(action, child)];
                }
                out.push((action, child));
            }
        }

        // Literal expansions.
        for var_idx in 0..self.pattern.vars.len() {
            let d_k = self.pattern.var_depth[var_idx];
            let mut seen: FxHashSet<(F::Discriminant<O>, usize)> = FxHashSet::default();
            let mut shapes: Vec<F::Apply<O>> = Vec::new();
            for m in &self.matches {
                for subst in &m.substs {
                    let eclass = &shared.egraph[subst.vars[var_idx]];
                    for node in &eclass.nodes {
                        if invalid_literal_expansion(node, d_k) {
                            continue;
                        }
                        let key = (node.discriminant(), node.children().len());
                        if seen.insert(key) {
                            shapes.push(node.clone());
                        }
                    }
                }
            }
            for shape in shapes {
                let mut child = self.clone();
                child.expand(var_idx, &shape, shared);
                if !child.matches.is_empty() {
                    out.push((
                        Action::Expand {
                            var_idx,
                            op: shape.discriminant(),
                            arity: shape.children().len(),
                        },
                        child,
                    ));
                }
            }
        }

        out
    }
}

/// Parses the shared-context fields out of CLI args, computes usage counts, and
/// returns the initial corpus size alongside the populated `SharedSearchData`.
pub fn setup_search<F: LanguageFamily, O: StitchOp>(egraph: StitchEgraph<F::Apply<O>>, root: Id, args: &crate::Args) -> (SharedSearchData<F, O>, crate::cost::CostCache, usize) {
    let follow_expr: Option<RevExpr<F::Apply<OpWithVar<O>>>> = args.follow.as_deref().map(|s| s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e)));
    let usage_counts = compute_usage_counts(&egraph, root);
    let shared = SharedSearchData {
        egraph,
        root,
        follow: follow_expr,
        weight_by_usage: args.weight_by_usage,
        usage_counts,
        p_reuse: args.p_reuse,
        check_slow: args.check_slow,
    };
    let cache = crate::cost::CostCache::new(&shared.egraph, root);
    let initial = SearchState::new(&shared);
    let initial_ho_arity = crate::cost::compute_ho_arity::<F, O>(&shared.egraph, &initial);
    let mut scratch = crate::cost::CostScratch::new(&shared.egraph);
    let original_size = crate::cost::compute_size(&shared.egraph, root, &cache, &mut scratch, &initial, shared.check_slow, &initial_ho_arity);
    (shared, cache, original_size)
}

impl<F: LanguageFamily, O: StitchOp> std::fmt::Display for SearchState<F, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState {{ pattern: {}, matches: {} }}", self.pattern, self.matches.len())
    }
}

/// Computes how many times each e-class appears in the fully-expanded corpus tree.
/// Top-down pass: root gets count 1, then propagate to children of the best (first) enode.
pub fn compute_usage_counts<L: crate::lang::StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> FxHashMap<Id, usize> {
    let mut counts = FxHashMap::<Id, usize>::default();
    counts.insert(root, 1);
    let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
    for i in (0..=max_id).rev() {
        let id = Id::from(i);
        let count = match counts.get(&id) {
            Some(&c) => c,
            None => continue,
        };
        if let Some(enode) = egraph[id].nodes.first() {
            for &child in enode.children() {
                *counts.entry(child).or_insert(0) += count;
            }
        }
    }
    counts
}
