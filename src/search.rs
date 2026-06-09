use crate::egraph_util::{build_size_minimal_extraction, compute_usage_counts};
use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchOp};
use crate::factor::{Factor, rebuild_factor};
use crate::matching::{MatchAtEClass, identity_matches};
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
fn invalid_literal_expansion<L: Language>(target: &L, depth: u32) -> bool
where
    L::Discriminant: StitchDisc,
{
    let Some(dbidx) = target.discriminant().de_bruijn_index() else { return false };
    target_is_free_db_var(dbidx, depth)
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
    /// Follow target: particles whose pattern isn't a valid prefix get zero
    /// weight at the resample step. Stored as the parsed surface form; the
    /// exact-match check in SMC re-serializes candidate states with HO arity
    /// applied before structurally comparing, since stitch's display adds
    /// `(?#k $i …)` HO-args that the bare pattern doesn't carry.
    pub follow: Option<crate::pattern::PatternRecExpr<F, O>>,
    /// Enable slow rewrite check (assert fast == slow computation).
    pub check_slow: bool,
    /// How many times each e-class is used in the fully-expanded corpus tree.
    pub usage_counts: FxHashMap<Id, usize>,
    /// Precomputed De Bruijn clamp for [`shift_equal`] (see
    /// [`crate::shift_equal::shift_clamp`]). Computed once here because the
    /// e-graph isn't unioned during search, and `shift_equal` is on the hot
    /// path — recomputing it per call would be O(enodes) each time.
    pub shift_clamp: u32,
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
    Dominant { child: SearchState<F, O>, support: usize },
    All(Vec<(Action<F::Discriminant<O>>, usize)>),
}

#[derive(Debug, Clone)]
pub struct SearchState<F: LanguageFamily, O: StitchOp> {
    pub pattern: Pattern<F, O>,
    // each match represents a different eclass at which `pattern` can be rooted
    pub matches: Vec<MatchAtEClass>,
    /// Cached `sum(m.num_substs() for m in matches)` — the total over match
    /// locations of each location's factored product size. Used by the
    /// dominance check in `enumerate_successors` to detect reuses that preserve
    /// the match set's size (and are therefore strictly dominant successors).
    pub num_substs: usize,
    /// Best-first canonical-ordering device: `Some(k)` means `?#0..?#(k-1)`
    /// are committed to never being expanded again. Expanding `?#k` raises this
    /// to `Some(k)`. Restricts only `Expand`, not `Reuse` (whose ordering uses
    /// `Pattern::var_reusable`). `None` disables the rule — SMC uses this to
    /// dedupe purely on the pattern's `RecExpr`.
    pub frozen_count: Option<usize>,
}

/// Computes the total number of (distinct) substitutions across all matches —
/// the sum over match locations of each location's factored product size.
fn total_substs(matches: &[MatchAtEClass]) -> usize {
    matches.iter().map(|m| m.num_substs()).sum()
}

/// Returns a copy of `f` with every slot strictly above `threshold` shifted by
/// `delta` (rows untouched). The shift is monotone over a suffix of an ascending
/// list, so the slot ordering is preserved.
fn renumber_factor(f: &Factor, threshold: usize, delta: isize) -> Factor {
    let slots = f.slots.iter().map(|&s| if s > threshold { (s as isize + delta) as usize } else { s }).collect();
    Factor { slots, rows: f.rows.clone() }
}

/// Applies the reuse collapse to a joint factor's filtered `rows` (over
/// `slots`): sets the kept slot's value to the shallow slot's value, drops the
/// `drop_idx` column, shifts higher slots down by 1, then re-decomposes.
/// Returns `None` when no rows survive.
fn collapse_reuse(slots: &[usize], mut rows: Vec<Vec<Id>>, shallow_idx: usize, keep_idx: usize, drop_idx: usize) -> Option<Vec<Factor>> {
    let pos = |s: usize| slots.iter().position(|&x| x == s).expect("reuse slot present in joint factor");
    let (pk, ps, pd) = (pos(keep_idx), pos(shallow_idx), pos(drop_idx));
    for r in &mut rows {
        r[pk] = r[ps];
        r.remove(pd);
    }
    let new_slots: Vec<usize> = slots.iter().filter(|&&s| s != drop_idx).map(|&s| if s > drop_idx { s - 1 } else { s }).collect();
    Factor::new(new_slots, rows).map(Factor::decompose)
}

/// Shared scaffold for the per-action match builders. For each parent match,
/// `transform` returns the indices of the factor(s) the action rewrites plus
/// their replacement factor(s), or `None` to drop the match (no surviving
/// subst). Every untouched factor is passed through `renumber`. The replacement
/// is appended after the renumbered factors — factor order is unobservable (the
/// product, `num_substs`, and `locate_slot` are all order-independent). Returns
/// the new matches and their total subst count.
fn rebuild_matches(parent_matches: &[MatchAtEClass], mut transform: impl FnMut(&MatchAtEClass) -> Option<(Vec<usize>, Vec<Factor>)>, renumber: impl Fn(&Factor) -> Factor) -> (Vec<MatchAtEClass>, usize) {
    let mut out: Vec<MatchAtEClass> = Vec::with_capacity(parent_matches.len());
    for m in parent_matches {
        let Some((touched, replacement)) = transform(m) else {
            continue;
        };
        let mut new_factors: Vec<Factor> = Vec::with_capacity(m.factors.len());
        for (fi, f) in m.factors.iter().enumerate() {
            if !touched.contains(&fi) {
                new_factors.push(renumber(f));
            }
        }
        new_factors.extend(replacement);
        out.push(MatchAtEClass { root_eclass: m.root_eclass, factors: new_factors });
    }
    let num = total_substs(&out);
    (out, num)
}

impl<F: LanguageFamily, O: StitchOp> SearchState<F, O> {
    /// True iff this pattern is a valid prefix of the follow target.
    pub fn matches_follow(&self, follow: &RevExpr<F::Apply<OpWithVar<O>>>) -> bool {
        crate::follow::follow_unify::<F, O>(&self.pattern.pattern, follow).is_some()
    }

    /// Builds child matches for an `expand(var_idx, target)` action. Mirrors
    /// `Pattern::expand`: slot `var_idx` is replaced by the target node's `k`
    /// child slots at positions `var_idx..var_idx+k`, and every slot above
    /// `var_idx` shifts up by `k-1`.
    ///
    /// Expansion touches a single slot, so it operates entirely within the one
    /// factor that owns `var_idx`: each row whose `var_idx` value has a node
    /// matching `target` spawns a row per such node (old slot dropped, children
    /// spliced in). Every other factor is independent of `var_idx`, so it only
    /// renumbers its slots. The rebuilt factor is re-decomposed in case the
    /// freshly spliced children are independent of the rest of the factor (this
    /// is where the cartesian structure is *discovered*). Matches whose owning
    /// factor produces no rows are dropped.
    ///
    /// We don't fv-prune captures here: captures whose fv reaches into
    /// pattern-internal binders are handled at apply/cost time by η-wrapping
    /// (see `enumerate_candidates` and `shift_free_egraph`), so the match set
    /// stays permissive and search keeps exploring those branches.
    fn build_subset_matches(parent_matches: &[MatchAtEClass], var_idx: usize, target: &F::Apply<O>, shared: &SharedSearchData<F, O>) -> (Vec<MatchAtEClass>, usize) {
        // var_idx → k contiguous slots; higher slots bump by k-1 (k may be 0 for
        // a leaf, shifting them down — hence isize).
        let arity_e = target.children().len();
        let delta = arity_e as isize - 1;
        rebuild_matches(
            parent_matches,
            |m| {
                let (owner, pos) = m.locate_slot(var_idx);
                let f = &m.factors[owner];
                let mut new_slots: Vec<usize> = Vec::with_capacity(f.slots.len() + arity_e.saturating_sub(1));
                for &s in &f.slots {
                    if s == var_idx {
                        new_slots.extend(var_idx..var_idx + arity_e);
                    } else if s > var_idx {
                        new_slots.push((s as isize + delta) as usize);
                    } else {
                        new_slots.push(s);
                    }
                }
                let built = rebuild_factor(new_slots, &f.rows, |row, rows| {
                    for node in &shared.egraph[row[pos]].nodes {
                        if !node.matches(target) {
                            continue;
                        }
                        let mut nr = row.to_vec();
                        nr.remove(pos);
                        for (j, child_id) in node.children().iter().enumerate() {
                            nr.insert(pos + j, *child_id);
                        }
                        rows.push(nr);
                    }
                })?;
                Some((vec![owner], built))
            },
            |f| renumber_factor(f, var_idx, delta),
        )
    }

    /// Builds child matches for a `reuse(var_idx, second_var_idx)` action.
    /// Mirrors `Pattern::reuse`: keeps the lower-indexed slot, removes the
    /// higher one, and shifts every slot above the dropped one down by 1.
    ///
    /// Reuse's `shift_equal` predicate couples two slots. When both live in the
    /// same factor it's a within-factor row filter; when they live in different
    /// factors those two factors are *merged* (their cartesian product, kept
    /// only where the predicate holds) — entangling what used to be
    /// independent. The merged/filtered factor is re-decomposed in case it
    /// happens to split again.
    ///
    /// Cross-depth soundness: the merged metavar appears at *both* original
    /// depths in the body. Its η-applied form `(?#k $0 … $(h-1))` requires
    /// `h` local pattern-internal binders at every site, so `h ≤ min_depth`.
    /// HO arity is `max{i + 1 : i ∈ kept_fv, i < merged_depth}`, so substs
    /// whose kept-eclass fv lands in `[min_depth, merged_depth)` are
    /// representable at the deep site but unbound at the shallow one — those
    /// are dropped. Same-depth reuse has an empty gap.
    fn build_subset_matches_reuse(parent_matches: &[MatchAtEClass], var_idx: usize, second_var_idx: usize, shallow_idx: usize, min_depth: u32, merged_depth: u32, shared: &SharedSearchData<F, O>) -> (Vec<MatchAtEClass>, usize) {
        let keep_idx = var_idx.min(second_var_idx);
        let drop_idx = var_idx.max(second_var_idx);
        let deep_idx = if shallow_idx == var_idx { second_var_idx } else { var_idx };
        rebuild_matches(
            parent_matches,
            |m| {
                let (sf, sp) = m.locate_slot(shallow_idx);
                let (df, dp) = m.locate_slot(deep_idx);
                // Build the joint (slots, filtered rows) over the factor(s)
                // carrying the two coupled slots, then collapse keep/drop and
                // re-decompose.
                let pred = |a: egg::Id, b: egg::Id| shift_equal(a, b, min_depth, merged_depth, &shared.egraph, shared.shift_clamp);
                let (joint_slots, joint_rows): (Vec<usize>, Vec<Vec<Id>>) = if sf == df {
                    let f = &m.factors[sf];
                    let rows = f.rows.iter().filter(|r| pred(r[sp], r[dp])).cloned().collect();
                    (f.slots.clone(), rows)
                } else {
                    let (fa, fb) = (&m.factors[sf], &m.factors[df]);
                    let mut slots: Vec<usize> = fa.slots.iter().chain(&fb.slots).copied().collect();
                    slots.sort_unstable();
                    let mut rows: Vec<Vec<Id>> = Vec::new();
                    for ra in &fa.rows {
                        for rb in &fb.rows {
                            if pred(ra[sp], rb[dp]) {
                                // Reassemble the joint row in ascending slot order.
                                let mut joint = vec![Id::from(0); slots.len()];
                                for (p, &s) in fa.slots.iter().enumerate() {
                                    joint[slots.binary_search(&s).unwrap()] = ra[p];
                                }
                                for (p, &s) in fb.slots.iter().enumerate() {
                                    joint[slots.binary_search(&s).unwrap()] = rb[p];
                                }
                                rows.push(joint);
                            }
                        }
                    }
                    (slots, rows)
                };
                let merged_factors = collapse_reuse(&joint_slots, joint_rows, shallow_idx, keep_idx, drop_idx)?;
                // sf and df are the touched factors (deduped when equal).
                let touched = if sf == df { vec![sf] } else { vec![sf, df] };
                Some((touched, merged_factors))
            },
            // Untouched factors just renumber their above-drop slots down by 1.
            |f| renumber_factor(f, drop_idx, -1),
        )
    }

    /// If `?#k` is useless, returns the (canonical) e-class id it's bound to in
    /// every match; otherwise `None`. "Useless" = every match maps `?#k` to the
    /// same e-class with no above-pattern free DB indices (all `fv < d_k`),
    /// matching stitch's `is_useless_abstract` / argument-capture check.
    fn useless_var_eclass(&self, k: usize, shared: &SharedSearchData<F, O>) -> Option<Id> {
        let d_k = self.pattern.var_depth[k];
        let mut first: Option<Id> = None;
        // Slot `k` lives in exactly one factor; its value across the whole
        // product is just that factor's column, so checking the owning factor's
        // rows (not the materialised product) suffices.
        for m in &self.matches {
            let (fi, pos) = m.locate_slot(k);
            for row in &m.factors[fi].rows {
                let id = shared.egraph.find(row[pos]);
                match first {
                    None => first = Some(id),
                    Some(f) if f == id => {}
                    Some(_) => return None,
                }
            }
        }
        first.filter(|&id| shared.egraph[id].data.fv.iter().all(|&i| (i as u32) < d_k))
    }

    /// True iff metavar `?#k` is "useless" (see [`useless_var_eclass`]).
    fn is_useless_var(&self, k: usize, shared: &SharedSearchData<F, O>) -> bool {
        self.useless_var_eclass(k, shared).is_some()
    }

    /// True iff some frozen variable (k < frozen_count) is useless. Used as a
    /// search-time pruning rule during enumeration; returns `false` when
    /// `frozen_count` is `None` (rule disabled) or when the match set is empty.
    pub fn is_useless_frozen(&self, shared: &SharedSearchData<F, O>) -> bool {
        let Some(fc) = self.frozen_count else { return false };
        (0..fc).any(|k| self.is_useless_var(k, shared))
    }

    /// True iff any metavar in the pattern is useless. Unlike
    /// `is_useless_frozen`, this checks *all* vars and ignores `frozen_count`
    /// — used as a hard rejection gate on candidate result patterns so we
    /// never return an abstraction whose body could be specialised by
    /// inlining a constant arg.
    pub fn has_useless_var(&self, shared: &SharedSearchData<F, O>) -> bool {
        (0..self.pattern.vars.len()).any(|k| self.is_useless_var(k, shared))
    }

    /// Builds a child state by fully concretizing every useless *non-frozen*
    /// metavar (`k >= frozen_count.unwrap_or(0)`) to the size-minimal
    /// extraction of the e-class it's bound to. Returns `None` when no such
    /// var exists. The child preserves the parent's `frozen_count` — this is a
    /// dominating short-circuit that runs "before" any normal expand in the
    /// canonical order, so it shouldn't bump the freeze cursor.
    ///
    /// Concretizations are applied in descending `var_idx` order so earlier
    /// indices don't shift mid-loop. Cross-depth vars inline too: `concretize`
    /// shifts the extraction to each occurrence's depth (sound — every surviving
    /// cross-depth reuse is a genuine shift-variant).
    pub fn inline_useless_nonfrozen(&self, shared: &SharedSearchData<F, O>) -> Option<SearchState<F, O>> {
        let start = self.frozen_count.unwrap_or(0);
        let mut targets: Vec<(usize, Id)> = (start..self.pattern.vars.len()).filter_map(|k| self.useless_var_eclass(k, shared).map(|id| (k, id))).collect();
        if targets.is_empty() {
            return None;
        }
        targets.sort_by_key(|t| std::cmp::Reverse(t.0));
        let mut child = self.clone();
        for (k, eclass) in &targets {
            child.concretize(*k, *eclass, shared);
        }
        Some(child)
    }

    /// Concretizes `?#var_idx` by splicing in the size-minimal extraction of
    /// `eclass`: pattern slot and subst slot both go away, no new metavars
    /// introduced. Caller must guarantee the var is useless — every subst
    /// already maps `vars[var_idx]` to `eclass`, and the eclass's fv is bound
    /// under the enclosing pattern binders (`fv < var_depth[var_idx]`).
    /// `useless_var_eclass` returns the eclass id iff these hold.
    ///
    /// `frozen_count` is left untouched: callers concretize only non-frozen
    /// vars (`var_idx >= frozen_count`), so removing that slot doesn't shift
    /// any frozen-position index.
    pub fn concretize(&mut self, var_idx: usize, eclass: Id, shared: &SharedSearchData<F, O>) {
        let mut extraction: Vec<F::Apply<OpWithVar<O>>> = Vec::new();
        let mut memo: FxHashMap<Id, Id> = FxHashMap::default();
        let root = build_size_minimal_extraction::<F, O>(&shared.egraph, eclass, &mut extraction, &mut memo);
        self.pattern.concretize(var_idx, &extraction, root);
        // Every surviving subst already maps vars[var_idx] to `eclass` by the
        // useless precondition, so dropping that slot can't merge distinct
        // substs — the row count (hence num_substs) is unchanged. Remove the
        // column from its owning factor, re-decompose (the dropped slot may have
        // been the only coupling), then renumber every slot above it down by 1.
        for m in &mut self.matches {
            let (fi, pos) = m.locate_slot(var_idx);
            let old = m.factors.remove(fi);
            let new_slots: Vec<usize> = old.slots.iter().copied().filter(|&s| s != var_idx).collect();
            if !new_slots.is_empty() {
                let new_rows: Vec<Vec<Id>> = old.rows.iter().map(|r| r.iter().enumerate().filter(|&(i, _)| i != pos).map(|(_, &v)| v).collect()).collect();
                if let Some(f) = Factor::new(new_slots, new_rows) {
                    m.factors.extend(f.decompose());
                }
            }
            for f in &mut m.factors {
                for s in &mut f.slots {
                    if *s > var_idx {
                        *s -= 1;
                    }
                }
            }
        }
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

    /// Applies an action to `self` and returns the resulting child without
    /// cloning the parent's matches/substs (only the surviving filtered
    /// substs get allocated in the child — the parent's `Vec<Subst>` data is
    /// not cloned-then-discarded). The pattern is cloned and mutated in
    /// place; `frozen_count` is recomputed inline.
    /// Used by best-first and by SMC after sampling so we don't materialise
    /// child states for successors that don't get picked.
    pub fn apply_action(&self, action: &Action<F::Discriminant<O>>, shared: &SharedSearchData<F, O>) -> SearchState<F, O> {
        let mut new_pattern = self.pattern.clone();
        let mut new_frozen_count = self.frozen_count;
        let (new_matches, new_num_substs) = match action {
            Action::Expand { var_idx, op, arity } => {
                let target = F::make(op.clone(), vec![Id::from(0); *arity]);
                // Commit to freezing every earlier var. `max` (rather than
                // `=`) keeps the count monotone even though best-first's
                // filter already enforces non-decreasing expansion order.
                if let Some(fc) = new_frozen_count.as_mut() {
                    *fc = (*fc).max(*var_idx);
                }
                new_pattern.expand(*var_idx, &target);
                Self::build_subset_matches(&self.matches, *var_idx, &target, shared)
            }
            Action::Reuse { keep, drop } => {
                let var_idx = *keep;
                let second_var_idx = *drop;
                // Snapshot pre-merge depths: `build_subset_matches_reuse`
                // needs both to bound the cross-depth gap, but
                // `pattern.reuse` collapses them.
                let d_a = self.pattern.var_depth[var_idx];
                let d_b = self.pattern.var_depth[second_var_idx];
                let shallow_idx = if d_a <= d_b { var_idx } else { second_var_idx };
                // Reuse is unconstrained by `frozen_count` (the freeze rule
                // only restricts syntactic expansions). If reuse removes a
                // var at index below `fc`, shift `fc` down so it still refers
                // to the same expand-threshold position after the index shift
                // in `pattern.reuse`.
                if let Some(fc) = new_frozen_count.as_mut() {
                    let drop_idx = var_idx.max(second_var_idx);
                    if drop_idx < *fc {
                        *fc -= 1;
                    }
                }
                new_pattern.reuse(var_idx, second_var_idx);
                Self::build_subset_matches_reuse(&self.matches, var_idx, second_var_idx, shallow_idx, d_a.min(d_b), d_a.max(d_b), shared)
            }
        };
        SearchState {
            pattern: new_pattern,
            matches: new_matches,
            num_substs: new_num_substs,
            frozen_count: new_frozen_count,
        }
    }

    /// Returns the enumerable successors of `self`. When dominance pruning
    /// fires, the single dominant child is built and returned via
    /// `SuccessorEnum::Dominant`; otherwise `SuccessorEnum::All` lists every
    /// `(action, support)` pair without building children, so samplers (e.g.
    /// SMC) skip work for unpicked actions. The caller materialises children
    /// for `All` via `apply_action`.
    ///
    /// Reuse candidates are emitted first so the dominance short-circuit can fire:
    /// when a reuse(i, j) preserves `num_substs` (every subst already had the two
    /// vars equal), the resulting child match set is identical to the parent's
    /// modulo the var-merge, so any successor of the parent is reachable via this
    /// reuse — we can return it as the *only* successor and skip enumerating the
    /// rest. Disabled by `--no-opt-dominance-reuse`.
    ///
    /// Expand actions are filtered against the best-first canonical-ordering
    /// rule: any `var_idx < self.frozen_count` or `var_idx > max_arity` is
    /// skipped before the action is even constructed. SMC passes
    /// `max_arity = usize::MAX` and starts with `frozen_count = None`, so the
    /// filter is a no-op for it.
    ///
    /// `support` is the (m,s)-pair count feeding the SMC weighting; it equals
    /// the surviving subst count, so `support > 0` ⇒ non-empty child.
    /// `support == self.num_substs` ⇒ dominant reuse (every subst already has
    /// the two vars unified); short-circuited unless disabled by
    /// `--no-opt-dominance-reuse`. Expand actions are emitted whenever
    /// `support > 0`; `subset_matches` then guarantees the child's match set is
    /// non-empty.
    #[allow(clippy::type_complexity)]
    pub fn enumerate_successor_actions(&self, shared: &SharedSearchData<F, O>, opt_dominance_reuse: bool, opt_useless_inline: bool, max_arity: usize, dominance_hits: &mut usize, useless_inline_hits: &mut usize) -> SuccessorEnum<F, O> {
        // Useless-non-frozen inlining is a strictly dominating short-circuit:
        // a constant arg adds no compression, so specialising the body by
        // inlining its size-minimal extraction can only improve cost. Runs
        // before reuse/expand enumeration in the canonical order.
        if opt_useless_inline && let Some(child) = self.inline_useless_nonfrozen(shared) {
            *useless_inline_hits += 1;
            let support = child.num_substs;
            return SuccessorEnum::Dominant { child, support };
        }
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
        // `var_reusable` is a best-first canonical-ordering device, mirroring
        // `frozen_count`. SMC (frozen_count = None) ignores it so its reuse
        // exploration stays unrestricted. Reusing two stale (non-reusable)
        // vars always re-reaches a pattern already obtainable by reusing them
        // earlier — when the later-created of the two was still in the fresh
        // cohort — so we skip it as a duplicate. This holds regardless of the
        // two vars' depths: a cross-depth reuse commutes past every expansion
        // after the one that created the deeper var, so it too has an
        // earlier-reuse canonical form.
        let enforce_reusable = self.frozen_count.is_some();
        for i in 0..n {
            for j in (i + 1)..n {
                let di = self.pattern.var_depth[i];
                let dj = self.pattern.var_depth[j];
                if enforce_reusable && !self.pattern.var_reusable[i] && !self.pattern.var_reusable[j] {
                    continue;
                }
                // Count full substs with `shift_equal(vars[i], vars[j])` per
                // match, factored: when both slots share a factor it's a row
                // filter scaled by the other factors' product; when they're in
                // different factors it's a filtered cross-product of the two,
                // scaled by the rest. Avoids materialising the product.
                let pred = |a: Id, b: Id| shift_equal(a, b, di, dj, &shared.egraph, shared.shift_clamp);
                let (support, raw_count): (usize, usize) = self.matches.iter().fold((0, 0), |(s, r), m| {
                    let (fi, pi) = m.locate_slot(i);
                    let (fj, pj) = m.locate_slot(j);
                    let total = m.num_substs();
                    let c = if fi == fj {
                        let f = &m.factors[fi];
                        let hits = f.rows.iter().filter(|row| pred(row[pi], row[pj])).count();
                        hits * (total / f.rows.len())
                    } else {
                        let (fa, fb) = (&m.factors[fi], &m.factors[fj]);
                        let mut pairs = 0usize;
                        for ra in &fa.rows {
                            for rb in &fb.rows {
                                if pred(ra[pi], rb[pj]) {
                                    pairs += 1;
                                }
                            }
                        }
                        pairs * (total / (fa.rows.len() * fb.rows.len()))
                    };
                    (s + usage(m.root_eclass) * c, r + c)
                });
                if support == 0 {
                    continue;
                }
                if opt_dominance_reuse && raw_count == self.num_substs {
                    *dominance_hits += 1;
                    let child = self.apply_action(&Action::Reuse { keep: i, drop: j }, shared);
                    return SuccessorEnum::Dominant { child, support };
                }
                out.push((Action::Reuse { keep: i, drop: j }, support));
            }
        }
        for var_idx in 0..n {
            // Freezing rule: expanding `?#k` commits to never expanding any
            // `?#j` with j < k; `max_arity` caps the eventual frozen_count
            // (since a successful expand at var_idx raises fc to >= var_idx).
            // Both checks are no-ops for SMC (frozen_count = None,
            // max_arity = usize::MAX).
            if var_idx > max_arity {
                continue;
            }
            if let Some(fc) = self.frozen_count
                && var_idx < fc
            {
                continue;
            }
            let d_k = self.pattern.var_depth[var_idx];
            let mut shape_idx: FxHashMap<(F::Discriminant<O>, usize), usize> = FxHashMap::default();
            let mut shapes: Vec<((F::Discriminant<O>, usize), usize)> = Vec::new();
            for m in &self.matches {
                let (fi, pos) = m.locate_slot(var_idx);
                let f = &m.factors[fi];
                // Each row of the owning factor stands in for `total/|f.rows|`
                // full substs (the product of the other factors), so weight the
                // per-row node contributions by that multiplier.
                let w = usage(m.root_eclass) * (m.num_substs() / f.rows.len());
                for row in &f.rows {
                    for node in &shared.egraph[row[pos]].nodes {
                        if invalid_literal_expansion(node, d_k) {
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
    let follow_expr: Option<crate::pattern::PatternRecExpr<F, O>> = args.follow.as_deref().map(|s| F::parse_follow_pattern::<O>(s).unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e)));
    let usage_counts = compute_usage_counts(&data.egraph, data.root);
    let crate::shared::SharedData { egraph, root } = data;
    let shift_clamp = crate::shift_equal::shift_clamp(&egraph);
    let shared = SharedSearchData {
        egraph,
        root,
        follow: follow_expr,
        usage_counts,
        check_slow: args.check_slow,
        shift_clamp,
    };
    let cache = crate::cost::CostCache::new(&shared.egraph, root);
    let initial = SearchState::new(&shared, None);
    let mut scratch = crate::cost::CostScratch::new(&shared.egraph);
    let initial_candidate = crate::cost::CostCandidate {
        variable_indices: vec![Vec::new(); initial.pattern.var_depth.len()],
    };
    let original_size = crate::cost::compute_size_for_candidate(&shared.egraph, root, &cache, &mut scratch, &initial, shared.check_slow, &initial_candidate);
    (shared, cache, original_size)
}

impl<F: LanguageFamily, O: StitchOp> std::fmt::Display for SearchState<F, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState {{ pattern: {}, matches: {} }}", self.pattern, self.matches.len())
    }
}
