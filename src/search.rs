use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchOp};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use crate::shift_equal::shift_equal;
use egg::{Id, Language};
use rustc_hash::FxHashMap;
use std::time::{Duration, Instant};

/// Tracks already-explored canonical patterns to dedupe successors during
/// search. Accumulates hit count and time spent so the host loop can report
/// stats. Wrap in `Option<…>` at the call site — `None` disables the check
/// entirely (useful for measuring how much pruning the seen-set buys).
/// Stores the *minimum* frozen_count ever seen per pattern. A repeat insertion
/// at an equal-or-higher frozen_count is a hit (the prior visit was at least
/// as flexible). A repeat at a strictly lower frozen_count overwrites and
/// passes through, because the new visit unlocks expand actions the prior one
/// had forbidden.
pub struct SeenTracker<F: LanguageFamily, O: StitchOp> {
    map: FxHashMap<Pattern<F, O>, usize>,
    pub hits: usize,
    pub time: Duration,
}

impl<F: LanguageFamily, O: StitchOp> Default for SeenTracker<F, O> {
    fn default() -> Self {
        Self {
            map: FxHashMap::default(),
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
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// Records `pattern` at `frozen_count` if this is the first visit or a
    /// strictly-lower-frozen one; returns `true` (skip) if an equal-or-lower
    /// frozen visit was already recorded — the prior visit was at least as
    /// flexible, so all of this visit's reachable successors are already
    /// reachable from it.
    pub fn check_and_insert(&mut self, pattern: Pattern<F, O>, frozen_count: usize) -> bool {
        let t = Instant::now();
        let skip = match self.map.get(&pattern) {
            Some(&existing) if existing <= frozen_count => true,
            _ => {
                self.map.insert(pattern, frozen_count);
                false
            }
        };
        self.time += t.elapsed();
        if skip {
            self.hits += 1;
        }
        skip
    }
}

/// True iff `target` is a free De Bruijn variable leaf with index `i ≥ d_k`.
fn target_is_free_db_var(dbidx: i32, d_k: u32) -> bool {
    (dbidx as u32) >= d_k
}

/// True iff `target` cannot be expanded to in a literal expansion.
fn invalid_literal_expansion<L: Language>(target: &L, depth: u32, cross_depth: bool) -> bool
where
    L::Discriminant: StitchDisc,
{
    let Some(dbidx) = target.discriminant().de_bruijn_index() else { return false };
    cross_depth || target_is_free_db_var(dbidx, depth)
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
    /// Enable slow rewrite check (assert fast == slow computation).
    pub check_slow: bool,
    /// How many times each e-class is used in the fully-expanded corpus tree.
    pub usage_counts: FxHashMap<Id, usize>,
}

impl<F: LanguageFamily, O: StitchOp> SharedSearchData<F, O> {
    /// Unwraps the search-specific fields and returns the underlying
    /// e-graph + root pair. Used by search drivers to hand the e-graph back
    /// to the outer abstraction loop.
    pub fn into_data(self) -> crate::shared::SharedData<F, O> {
        crate::shared::SharedData::new(self.egraph, self.root)
    }
}

/// Result of `enumerate_successor_actions`: either a single pre-built dominant
/// child (dominance pruning fired) or a list of `(action, support)` pairs the
/// caller can sample from. SMC builds children lazily only for sampled actions.
pub enum SuccessorEnum<F: LanguageFamily, O: StitchOp> {
    Dominant { action: Action<F::Discriminant<O>>, child: SearchState<F, O>, support: usize },
    All(Vec<(Action<F::Discriminant<O>>, usize)>),
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
    /// Best-first canonical-ordering device: `Some(k)` means `?#0..?#(k-1)`
    /// are committed to never being expanded *and* are forbidden from
    /// participating in `Reuse`. Expanding `?#k` raises this to `Some(k)`.
    /// `None` disables the rule entirely — SMC uses this so it can dedupe
    /// purely on the pattern's `RecExpr`.
    pub frozen_count: Option<usize>,
}

/// Computes the total number of substitutions across all matches.
fn total_substs(matches: &[MatchAtEClass]) -> usize {
    matches.iter().map(|m| m.substs.len()).sum()
}

impl<F: LanguageFamily, O: StitchOp> SearchState<F, O> {
    /// True iff this pattern is a valid prefix of the follow target.
    pub fn matches_follow(&self, follow: &RevExpr<F::Apply<OpWithVar<O>>>) -> bool {
        crate::follow::follow_unify::<F, O>(&self.pattern.pattern, follow).is_some()
    }

    /// Expands the pattern at `var_idx` with `target` and filters matches accordingly.
    pub fn expand(&mut self, var_idx: usize, target: &F::Apply<O>, shared: &SharedSearchData<F, O>) {
        // Commit to freezing every earlier var. `max` (rather than `=`) keeps
        // the count monotone even though best-first's filter already enforces
        // non-decreasing expansion order.
        if let Some(fc) = self.frozen_count.as_mut() {
            *fc = (*fc).max(var_idx);
        }
        self.pattern.expand(var_idx, target);
        self.subset_matches(var_idx, target, shared);
    }

    /// Merges two pattern variables and filters matches to those where both point to the same e-class.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize, shared: &SharedSearchData<F, O>) {
        // Snapshot pre-merge depths: `subset_matches_reuse` needs both to
        // bound the cross-depth gap, but `pattern.reuse` collapses them.
        let d_a = self.pattern.var_depth[var_idx];
        let d_b = self.pattern.var_depth[second_var_idx];
        let shallow_idx = if d_a <= d_b { var_idx } else { second_var_idx };
        // Reuse is unconstrained by `frozen_count` (the freeze rule only
        // restricts syntactic expansions). If reuse removes a var at index
        // below `fc`, shift `fc` down so it still refers to the same
        // expand-threshold position after the index shift in `pattern.reuse`.
        if let Some(fc) = self.frozen_count.as_mut() {
            let drop_idx = var_idx.max(second_var_idx);
            if drop_idx < *fc {
                *fc -= 1;
            }
        }
        self.pattern.reuse(var_idx, second_var_idx);
        self.subset_matches_reuse(var_idx, second_var_idx, shallow_idx, d_a.min(d_b), d_a.max(d_b), shared);
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
    pub fn subset_matches_reuse(&mut self, var_idx: usize, second_var_idx: usize, shallow_idx: usize, min_depth: u32, merged_depth: u32, shared: &SharedSearchData<F, O>) {
        let keep_idx = var_idx.min(second_var_idx);
        let drop_idx = var_idx.max(second_var_idx);
        let deep_idx = if shallow_idx == var_idx { second_var_idx } else { var_idx };
        self.update_matches(|subst, out| {
            let shallow_id = subst.vars[shallow_idx];
            let deep_id = subst.vars[deep_idx];
            if !shift_equal(shallow_id, deep_id, min_depth, merged_depth, &shared.egraph) {
                return;
            }
            let mut new_subst = subst.clone();
            new_subst.vars[keep_idx] = shallow_id;
            new_subst.vars.remove(drop_idx);
            out.push(new_subst);
        });
    }

    /// True iff some frozen variable `?#k` (k < frozen_count) is "useless":
    /// every match's substitution maps `?#k` to the same e-class, and that
    /// e-class has no above-pattern free DB indices (all `fv < d_k`). Such a
    /// metavar adds no compression — its arg is invariant across sites, so the
    /// abstraction could be specialised by inlining the arg.
    ///
    /// Stitch analog: `is_useless_abstract` (argument capture). We require fv
    /// to lie strictly below `d_k`, which matches stitch's "shifted_id fv is
    /// empty" check (the shift drops indices `< d_k`).
    ///
    /// Returns `false` when `frozen_count` is `None` (rule disabled) or when
    /// the match set is empty.
    pub fn is_useless_frozen(&self, shared: &SharedSearchData<F, O>) -> bool {
        let Some(fc) = self.frozen_count else { return false };
        for k in 0..fc {
            let d_k = self.pattern.var_depth[k];
            let mut first: Option<Id> = None;
            let mut same = true;
            'outer: for m in &self.matches {
                for s in &m.substs {
                    match first {
                        None => first = Some(s.vars[k]),
                        Some(f) if f == s.vars[k] => {}
                        Some(_) => {
                            same = false;
                            break 'outer;
                        }
                    }
                }
            }
            if !same {
                continue;
            }
            if let Some(id) = first
                && shared.egraph[id].data.fv.iter().all(|&i| (i as u32) < d_k)
            {
                return true;
            }
        }
        false
    }

    /// Creates the initial search state: a single-variable pattern matching every e-class.
    /// `frozen_count` enables the freeze-based canonical-ordering rule when `Some(0)`;
    /// pass `None` to disable the check (e.g. for SMC).
    pub fn new(shared: &SharedSearchData<F, O>, frozen_count: Option<usize>) -> Self {
        let matches = identity_matches(&shared.egraph, shared.root);
        let num_substs = total_substs(&matches);
        Self {
            pattern: Pattern::single_var(),
            matches,
            num_substs,
            frozen_count,
        }
    }

    /// Applies an action to a clone of `self` and returns the resulting child.
    /// Used by SMC after sampling so we don't materialise child states for
    /// successors that don't get picked.
    pub fn apply_action(&self, action: &Action<F::Discriminant<O>>, shared: &SharedSearchData<F, O>) -> SearchState<F, O> {
        let mut child = self.clone();
        match action {
            Action::Expand { var_idx, op, arity } => {
                let target = F::make(op.clone(), vec![Id::from(0); *arity]);
                child.expand(*var_idx, &target, shared);
            }
            Action::Reuse { keep, drop } => child.reuse(*keep, *drop, shared),
        }
        child
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
    /// Thin wrapper over `enumerate_successor_actions` that materialises every
    /// successor's child state up front. Best-first needs all children to push
    /// into the search frontier; SMC uses the lazy variant directly so it only
    /// builds children for the actions it actually samples.
    #[allow(clippy::type_complexity)]
    pub fn enumerate_successors(&self, shared: &SharedSearchData<F, O>, opt_dominance_reuse: bool, dominance_hits: &mut usize) -> Vec<(Action<F::Discriminant<O>>, SearchState<F, O>, usize)> {
        match self.enumerate_successor_actions(shared, opt_dominance_reuse, dominance_hits) {
            SuccessorEnum::Dominant { action, child, support } => vec![(action, child, support)],
            SuccessorEnum::All(actions) => actions
                .into_iter()
                .map(|(a, support)| {
                    let child = self.apply_action(&a, shared);
                    (a, child, support)
                })
                .collect(),
        }
    }

    /// Lazy variant of `enumerate_successors`: returns `(action, support)` pairs
    /// without building children, so samplers (e.g. SMC) skip work for unpicked
    /// actions. The caller materialises children via `apply_action`. When
    /// dominance pruning fires, the single dominant child is built and returned
    /// via `SuccessorEnum::Dominant`, matching the eager method's short-circuit.
    ///
    /// `support` is the (m,s)-pair count feeding the SMC weighting; it equals
    /// the surviving subst count, so `support > 0` ⇒ non-empty child.
    /// `support == self.num_substs` ⇒ dominant reuse (every subst already has
    /// the two vars unified); short-circuited unless disabled by
    /// `--no-opt-dominance-reuse`. Expand actions are emitted whenever
    /// `support > 0`; `subset_matches` then guarantees the child's match set is
    /// non-empty.
    #[allow(clippy::type_complexity)]
    pub fn enumerate_successor_actions(&self, shared: &SharedSearchData<F, O>, opt_dominance_reuse: bool, dominance_hits: &mut usize) -> SuccessorEnum<F, O> {
        let mut out: Vec<(Action<F::Discriminant<O>>, usize)> = Vec::new();
        let n = self.pattern.vars.len();
        // Weight each (match, subst) contribution by how often that match's
        // root e-class appears in the fully-expanded corpus, so popular
        // root-positions sway the action distribution proportionally to the
        // compression value they represent — not just their hash-consed
        // distinctness. Without this, an abstraction that fires on a single
        // eclass used thousands of times looks like the same support as one
        // that fires on thousands of distinct one-off eclasses.
        let usage = |root: Id| shared.usage_counts.get(&root).copied().unwrap_or(1);
        for i in 0..n {
            for j in (i + 1)..n {
                let di = self.pattern.var_depth[i];
                let dj = self.pattern.var_depth[j];
                let (support, raw_count): (usize, usize) = self.matches.iter().fold((0, 0), |(s, r), m| {
                    let c = m.substs.iter().filter(|s| shift_equal(s.vars[i], s.vars[j], di, dj, &shared.egraph)).count();
                    (s + usage(m.root_eclass) * c, r + c)
                });
                if support == 0 {
                    continue;
                }
                let action = Action::Reuse { keep: i, drop: j };
                if opt_dominance_reuse && raw_count == self.num_substs {
                    *dominance_hits += 1;
                    let mut child = self.clone();
                    child.reuse(i, j, shared);
                    return SuccessorEnum::Dominant { action, child, support };
                }
                out.push((action, support));
            }
        }
        for var_idx in 0..n {
            let d_k = self.pattern.var_depth[var_idx];
            let cross_depth = self.pattern.var_cross_depth[var_idx];
            let mut shape_idx: FxHashMap<(F::Discriminant<O>, usize), usize> = FxHashMap::default();
            let mut shapes: Vec<((F::Discriminant<O>, usize), usize)> = Vec::new();
            for m in &self.matches {
                let w = usage(m.root_eclass);
                for subst in &m.substs {
                    let eclass = &shared.egraph[subst.vars[var_idx]];
                    for node in &eclass.nodes {
                        if invalid_literal_expansion(node, d_k, cross_depth) {
                            continue;
                        }
                        let key = (node.discriminant(), node.children().len());
                        match shape_idx.get(&key) {
                            Some(&idx) => shapes[idx].1 += w,
                            None => {
                                shape_idx.insert(key.clone(), shapes.len());
                                shapes.push((key, w));
                            }
                        }
                    }
                }
            }
            for ((op, arity), support) in shapes {
                out.push((Action::Expand { var_idx, op, arity }, support));
            }
        }
        SuccessorEnum::All(out)
    }
}

/// Parses the shared-context fields out of CLI args, computes usage counts, and
/// returns the initial corpus size alongside the populated `SharedSearchData`.
pub fn setup_search<F: LanguageFamily, O: StitchOp>(data: crate::shared::SharedData<F, O>, args: &crate::Args) -> (SharedSearchData<F, O>, crate::cost::CostCache, usize) {
    // The follow pattern is whatever `display_recexpr` would emit for a
    // pattern: flat-form sexps that may have a `?#k` variable head (e.g.
    // `(?#0 a b c)`). egg's stock pattern parser rejects both shapes, so
    // each family ships its own walker.
    let follow_expr: Option<RevExpr<F::Apply<OpWithVar<O>>>> = args.follow.as_deref().map(|s| F::parse_follow_pattern::<O>(s).unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e)));
    let usage_counts = compute_usage_counts(&data.egraph, data.root);
    let crate::shared::SharedData { egraph, root } = data;
    let shared = SharedSearchData {
        egraph,
        root,
        follow: follow_expr,
        usage_counts,
        check_slow: args.check_slow,
    };
    let cache = crate::cost::CostCache::new(&shared.egraph, root);
    let initial = SearchState::new(&shared, None);
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
