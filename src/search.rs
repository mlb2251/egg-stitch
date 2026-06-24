use crate::egraph_util::{build_size_minimal_extraction, compute_usage_counts};
use crate::factor::{Factor, rebuild_factor};
use crate::lang::{LanguageFamily, OpWithFrozenVar, OpWithVar, StitchDisc, StitchEgraph, StitchOp};
use crate::matching::{MatchAtEClass, identity_matches};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use crate::shift_equal::shift_equal;
use egg::{Id, Language, RecExpr};
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::{Duration, Instant};

/// A candidate expansion shape: an enode's discriminant paired with its arity.
type Shape<F, O> = (<F as LanguageFamily>::Discriminant<O>, usize);
/// Per-eclass histogram: each distinct enode shape `(disc, arity)` with its
/// multiplicity, in enode first-appearance order.
type EclassShapeHist<F, O> = Vec<(<F as LanguageFamily>::Discriminant<O>, usize, usize)>;
/// A candidate expansion shape for a var together with the statistics that drive
/// successor emission and ordering. Built by [`SearchState::expand_shapes`].
struct VarShape<F: LanguageFamily, O: StitchOp> {
    shape: Shape<F, O>,
    /// Usage-weighted support `Σ_(r, σ) usage(r) n(σ)`, where `n(σ)` is the
    /// number of enodes of this shape at the var's binding
    support: usize,
    /// Numerator of the explosion estimate `Σ_(r, σ) n(σ)`
    expand_enodes: usize,
    /// Denominator of the explosion estimate `Σ_(r, σ) 1` (total full substs)
    substs: usize,
}
/// Per var, its candidate expansion shapes in enode first-appearance order.
type VarShapes<F, O> = Vec<VarShape<F, O>>;

/// Tracks already-explored canonical patterns to dedupe successors during
/// search. Accumulates hit count and time spent so the host loop can report
/// stats. Wrap in `Option<…>` at the call site — `None` disables the check
/// entirely (useful for measuring how much pruning the seen-set buys).
/// Stores the most-flexible freeze mask ever seen per pattern. A repeat
/// insertion is a hit when a recorded visit's frozen set is a *subset* of the
/// new one — the prior visit was at least as flexible, so all of this visit's
/// reachable successors are already reachable from it. A repeat that isn't
/// dominated overwrites and passes through, because it unlocks expand actions
/// the prior one had forbidden.
pub struct SeenTracker<F: LanguageFamily, O: StitchOp> {
    map: FxHashMap<Pattern<F, O>, Vec<bool>>,
    /// Shadow domination map we *never* evict from: per pattern it retains every
    /// distinct mask ever seen, where `map` keeps only the latest non-dominated
    /// one. A repeat is a hit when any retained mask is a subset of the new one.
    /// `full_dom_hits - hits` is the extra dedup that never forgetting a mask
    /// would buy over the single-slot `map`.
    dom_map: FxHashMap<Pattern<F, O>, Vec<Vec<bool>>>,
    /// Experimental second seen-set: a pure hash-cons egraph (no analysis, no
    /// unions) over the frozen-var language. Each pattern is converted into a
    /// `RecExpr<F::Apply<OpWithFrozenVar<O>>>` and looked up / inserted here, so
    /// the freeze mask is part of the *term* rather than a side value. Shadow-run
    /// alongside `map` to validate the encoding — `map` still drives the search.
    ///
    /// Each inserted term is wrapped in a `Root` sentinel
    /// ([`pattern_to_frozen_recexpr`]). Without it, `add_expr` interns every
    /// subterm, so `lookup_expr` would hit on a pattern that merely appears
    /// inside a larger inserted one — far more than true top-level repeats. With
    /// the wrapper a hit means the whole `(syntax, frozen)` term was inserted as
    /// a top-level pattern (modulo congruence), matching `map`'s notion of a
    /// visited state.
    seen_egraph: egg::EGraph<F::Apply<OpWithFrozenVar<O>>, ()>,
    pub hits: usize,
    /// Calls that would skip under *exact*-mask matching rather than subset
    /// domination: same `map` lookup, but the stored mask must equal `frozen`
    /// exactly. `hits - exact_hits` is the extra dedup subset-domination buys.
    pub exact_hits: usize,
    pub time: Duration,
    /// Total calls the shadow seen-egraph deduped (exact `(syntax, frozen)`
    /// membership, modulo congruence). The `map`-side total is `hits`.
    pub egraph_hits: usize,
    /// Calls the never-evicted [`Self::dom_map`] would skip: some retained mask
    /// dominates `frozen`. This is what `map`'s subset domination would catch if
    /// it never overwrote, so `full_dom_hits >= hits`.
    pub full_dom_hits: usize,
    /// Total top-level terms inserted into `seen_egraph` (one `add_expr` per
    /// [`Self::check_and_insert`] call). `inserted - egraph_hits` is the number
    /// of *distinct* `(syntax, frozen)` terms, which the end-of-run audit checks
    /// against the count of `Root`-headed terms the egraph actually represents.
    pub inserted: usize,
}

/// True iff every var frozen in `a` is also frozen in `b` (i.e. `a ⊆ b`, so `a`
/// is at least as flexible). Masks for the same pattern share a length.
fn frozen_subset(a: &[bool], b: &[bool]) -> bool {
    a.iter().zip(b).all(|(&fa, &fb)| !fa || fb)
}

/// Convert a pattern plus its freeze mask into a `RecExpr` over the frozen-var
/// language: every `Var(k)` leaf becomes `FrozenVar(k)` iff `frozen[k]`, so the
/// freeze bit is encoded structurally. Reuses the existing `RevExpr -> RecExpr`
/// conversion, then functor-maps each node's leaf-op slot via
/// `LanguageFamily::map_discriminant` (structural variants pass through; only
/// the leaf op is rewritten), preserving children and node order.
fn pattern_to_frozen_recexpr<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, frozen: &[bool]) -> RecExpr<F::Apply<OpWithFrozenVar<O>>> {
    // The canonical invariant numbers `?#k` as `egg::Var::from(k)`, so build the
    // frozen set keyed by that same `Var` rather than recovering `k` per leaf.
    let frozen_vars: FxHashSet<egg::Var> = frozen.iter().enumerate().filter_map(|(k, &f)| f.then(|| egg::Var::from(k as u32))).collect();
    let src: RecExpr<F::Apply<OpWithVar<O>>> = pattern.pattern.clone().into();
    let src_nodes: Vec<F::Apply<OpWithVar<O>>> = src.into();
    let mut nodes: Vec<F::Apply<OpWithFrozenVar<O>>> = src_nodes
        .into_iter()
        .map(|node| {
            let kids = node.children().to_vec();
            let disc = F::map_discriminant(node.discriminant(), |op| match op {
                OpWithVar::Node(o) => OpWithFrozenVar::Node(o),
                OpWithVar::Var(v) if frozen_vars.contains(&v) => OpWithFrozenVar::FrozenVar(v),
                OpWithVar::Var(v) => OpWithFrozenVar::Var(v),
            });
            F::make(disc, kids)
        })
        .collect();
    // Wrap the whole term in a `Root` sentinel so lookups match only against
    // top-level inserts, not subterms `add_expr` interned from larger patterns.
    let root = Id::from(nodes.len() - 1);
    F::wrap_seen_root::<O>(&mut nodes, root);
    RecExpr::from(nodes)
}

impl<F: LanguageFamily, O: StitchOp> Default for SeenTracker<F, O> {
    fn default() -> Self {
        Self {
            map: FxHashMap::default(),
            dom_map: FxHashMap::default(),
            seen_egraph: egg::EGraph::new(()),
            hits: 0,
            exact_hits: 0,
            time: Duration::ZERO,
            egraph_hits: 0,
            full_dom_hits: 0,
            inserted: 0,
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
    /// Number of e-classes in the shadow seen-egraph (one per distinct
    /// `(syntax, frozen)` term inserted, modulo congruence).
    pub fn egraph_len(&self) -> usize {
        self.seen_egraph.number_of_classes()
    }
    /// Records `pattern` at `frozen` if this is the first visit or a
    /// not-dominated one; returns `true` (skip) if a recorded visit's frozen
    /// set is a subset of `frozen` — the prior visit was at least as flexible,
    /// so all of this visit's reachable successors are already reachable from
    /// it.
    ///
    /// The `map` result drives the search and is unchanged. In parallel we run
    /// the egraph-backed and exact-match seen-sets, tallying each side's total
    /// hits so a POC run can compare their dedup power against `map`.
    pub fn check_and_insert(&mut self, pattern: Pattern<F, O>, frozen: Vec<bool>) -> bool {
        let t = Instant::now();

        // Egraph side (shadow): exact `(syntax, frozen)` membership. Look up
        // before inserting so `egraph_skip` reflects prior visits only.
        let recexpr = pattern_to_frozen_recexpr(&pattern, &frozen);
        let egraph_skip = self.seen_egraph.lookup_expr(&recexpr).is_some();
        self.seen_egraph.add_expr(&recexpr);
        self.inserted += 1;
        if egraph_skip {
            self.egraph_hits += 1;
        }

        // Map side (source of truth): subset-domination, overwrite on miss. The
        // exact-match shadow shares this lookup but is checked independently of
        // the subset decision: it counts a hit whenever the stored mask equals
        // `frozen` exactly — a stricter rule than subset domination.
        let map_skip = match self.map.get(&pattern) {
            Some(existing) => {
                if *existing == frozen {
                    self.exact_hits += 1;
                }
                frozen_subset(existing, &frozen)
            }
            None => false,
        };

        // Full-domination shadow: retain every distinct mask per pattern (never
        // evicted) and count a hit whenever any retained mask dominates `frozen`.
        // Skips storing dominated/duplicate masks since they add no domination
        // power the retained ones don't already have.
        let dom_hit = match self.dom_map.get_mut(&pattern) {
            Some(masks) => {
                let hit = masks.iter().any(|m| frozen_subset(m, &frozen));
                if !masks.contains(&frozen) {
                    masks.push(frozen.clone());
                }
                hit
            }
            None => {
                self.dom_map.insert(pattern.clone(), vec![frozen.clone()]);
                false
            }
        };
        if dom_hit {
            self.full_dom_hits += 1;
        }

        if !map_skip {
            self.map.insert(pattern, frozen);
        }

        self.time += t.elapsed();
        if map_skip {
            self.hits += 1;
        }
        map_skip
    }

    /// End-of-run audit of the shadow seen-egraph (pure stats — nothing the
    /// search reads). Records enodes, eclasses, and the number of distinct
    /// e-classes the inserted top-level terms occupy (see
    /// [`Self::distinct_root_classes`]), then runs `rules` to saturation,
    /// rebuilds, and re-records. `iter_limit`/`node_limit` should be high so
    /// saturation (not a limit) is what stops the run; the returned
    /// `stop_reason`/`applications` report whether that happened and how many
    /// firings occurred.
    ///
    /// `rules` are the program-language DSRs lifted to the frozen-var language
    /// (see [`crate::best_first`]); `SimpleScheduler` fires every match each
    /// iteration (true saturation semantics, no backoff throttling).
    ///
    /// Before rewriting, `root_classes_before` equals the number of distinct
    /// `(syntax, frozen)` patterns inserted (`inserted - egraph_hits`): the
    /// egraph is a pure hash-cons, so each distinct insert is its own class. The
    /// rewrites merge classes the DSRs prove equivalent, so `root_classes_after`
    /// drops by exactly the number of inserted terms that collapse together.
    pub fn audit_seen_egraph(&mut self, rules: &[egg::Rewrite<F::Apply<OpWithFrozenVar<O>>, ()>], iter_limit: usize, node_limit: usize) -> SeenEgraphAudit {
        let unique_inserted = self.inserted - self.egraph_hits;
        let nodes_before = self.seen_egraph.total_number_of_nodes();
        let classes_before = self.seen_egraph.number_of_classes();
        let root_classes_before = self.distinct_root_classes();

        let egraph = std::mem::replace(&mut self.seen_egraph, egg::EGraph::new(()));
        let runner = egg::Runner::default()
            .with_egraph(egraph)
            .with_node_limit(node_limit)
            .with_iter_limit(iter_limit)
            .with_time_limit(Duration::from_secs(60 * 60 * 24))
            .with_scheduler(egg::SimpleScheduler)
            .run(rules);
        let iterations = runner.iterations.len();
        // `applied` is keyed per rule per iteration, so sum it for total firings.
        let applications: usize = runner.iterations.iter().map(|it| it.applied.values().sum::<usize>()).sum();
        let stop_reason = format!("{:?}", runner.stop_reason);
        self.seen_egraph = runner.egraph;
        self.seen_egraph.rebuild();

        let nodes_after = self.seen_egraph.total_number_of_nodes();
        let classes_after = self.seen_egraph.number_of_classes();
        let root_classes_after = self.distinct_root_classes();

        SeenEgraphAudit {
            unique_inserted,
            total_inserted: self.inserted,
            num_rules: rules.len(),
            iterations,
            applications,
            stop_reason,
            nodes_before,
            classes_before,
            root_classes_before,
            nodes_after,
            classes_after,
            root_classes_after,
        }
    }

    /// Number of distinct e-classes the inserted top-level terms currently
    /// occupy. `dom_map` retains every distinct `(pattern, frozen)` ever
    /// inserted, so re-derive each one's `Root`-wrapped frozen-var term, look up
    /// its canonical e-class id (asserting it's present — it *was* inserted),
    /// and count the distinct ids. Before rewriting this equals `unique_inserted`
    /// (pure hash-cons); after, it drops by however many inserted terms the
    /// rewrites proved equivalent.
    fn distinct_root_classes(&self) -> usize {
        let mut classes: FxHashSet<Id> = FxHashSet::default();
        for (pattern, masks) in &self.dom_map {
            for mask in masks {
                let recexpr = pattern_to_frozen_recexpr(pattern, mask);
                let id = self.seen_egraph.lookup_expr(&recexpr).expect("every inserted root term must be present in the seen-egraph");
                classes.insert(id);
            }
        }
        classes.len()
    }
}

/// Stats produced by [`SeenTracker::audit_seen_egraph`], split before/after the
/// saturation+rebuild pass. `root_classes_*` is the number of distinct e-classes
/// the inserted top-level terms occupy — finite even when the rewrites make each
/// class represent infinitely many terms.
pub struct SeenEgraphAudit {
    pub unique_inserted: usize,
    pub total_inserted: usize,
    pub num_rules: usize,
    pub iterations: usize,
    pub applications: usize,
    pub stop_reason: String,
    pub nodes_before: usize,
    pub classes_before: usize,
    pub root_classes_before: usize,
    pub nodes_after: usize,
    pub classes_after: usize,
    pub root_classes_after: usize,
}

/// True iff `target` is a free De Bruijn variable leaf with index `i ≥ d_k`.
fn target_is_free_db_var(dbidx: i32, d_k: u32) -> bool {
    (dbidx as u32) >= d_k
}

/// True iff a candidate expansion shape `disc` cannot be a literal expansion at
/// a hole of depth `depth`
fn invalid_literal_expansion<D: StitchDisc>(disc: &D, depth: u32) -> bool {
    let Some(dbidx) = disc.de_bruijn_index() else { return false };
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
    /// Per-eclass `(discriminant, arity, enode-count)` shape histogram,
    /// precomputed once (the e-graph is static during search). Indexed by
    /// canonical eclass id — sound because no unions happen during search.
    pub eclass_shapes: Vec<EclassShapeHist<F, O>>,
    /// `--decompose-min-rows`: min factor rows before [`Factor::decompose`]
    /// splits. Held here so every decompose site reads one runtime value.
    pub decompose_min_rows: usize,
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
    Dominant {
        child: SearchState<F, O>,
        support: usize,
    },
    /// `(action, support)` pairs plus the rank (`var -> rank`) the freeze rule
    /// in `apply_action` needs to mark the right vars frozen when an `Expand`
    /// fires. Identity order for SMC.
    All {
        actions: Vec<(Action<F::Discriminant<O>>, usize)>,
        rank: Vec<usize>,
    },
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
    /// Whether the best-first freeze rule is active. When `true`, expanding
    /// `?#k` commits `?#0..?#(k-1)` to never being expanded again, and a
    /// non-dominant `Reuse` freezes the merged var (the frozen set is tracked
    /// per-var in `Pattern::var_frozen` and is no longer necessarily a prefix);
    /// reuse *ordering* uses the same field's reusable cohort. `false` disables the
    /// rule — SMC uses this to dedupe purely on the pattern's `RecExpr`.
    pub freeze_rule: bool,
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
fn collapse_reuse(slots: &[usize], mut rows: Vec<Vec<Id>>, shallow_idx: usize, keep_idx: usize, drop_idx: usize, min_rows: usize) -> Option<Vec<Factor>> {
    let pos = |s: usize| slots.iter().position(|&x| x == s).expect("reuse slot present in joint factor");
    let (pk, ps, pd) = (pos(keep_idx), pos(shallow_idx), pos(drop_idx));
    for r in &mut rows {
        r[pk] = r[ps];
        r.remove(pd);
    }
    let new_slots: Vec<usize> = slots.iter().filter(|&&s| s != drop_idx).map(|&s| if s > drop_idx { s - 1 } else { s }).collect();
    Factor::new(new_slots, rows).map(|f| f.decompose(min_rows))
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
        new_factors.retain(|f| !f.slots.is_empty());
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
                let built = rebuild_factor(new_slots, &f.rows, shared.decompose_min_rows, |row, rows| {
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
                let merged_factors = collapse_reuse(&joint_slots, joint_rows, shallow_idx, keep_idx, drop_idx, shared.decompose_min_rows)?;
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

    /// True iff some frozen variable is useless. Used as a search-time pruning
    /// rule during enumeration; returns `false` when the freeze rule is
    /// disabled (SMC) or when the match set is empty.
    pub fn is_useless_frozen(&self, shared: &SharedSearchData<F, O>) -> bool {
        if !self.freeze_rule {
            return false;
        }
        (0..self.pattern.vars.len()).filter(|&k| self.pattern.var_frozen[k]).any(|k| self.is_useless_var(k, shared))
    }

    /// True iff any metavar in the pattern is useless. Unlike
    /// `is_useless_frozen`, this checks *all* vars and ignores `frozen_count`
    /// — used as a hard rejection gate on candidate result patterns so we
    /// never return an abstraction whose body could be specialised by
    /// inlining a constant arg.
    pub fn has_useless_var(&self, shared: &SharedSearchData<F, O>) -> bool {
        (0..self.pattern.vars.len()).any(|k| self.is_useless_var(k, shared))
    }

    /// Total number of frozen vars. The frozen set is no longer a prefix (a
    /// non-dominant reuse can freeze a non-leading slot), so test individual
    /// slots via `pattern.var_frozen[k]`; this count is the
    /// number of holes the stub application keeps.
    pub fn frozen_count(&self) -> usize {
        self.pattern.var_frozen.iter().filter(|&&f| f).count()
    }

    /// Builds a child state by fully concretizing every useless *non-frozen*
    /// metavar to the size-minimal extraction of the e-class it's bound to.
    /// Returns `None` when no such var exists. The child preserves the parent's
    /// freeze set — this is a dominating short-circuit that runs "before" any
    /// normal expand in the canonical order, so it shouldn't bump the freeze
    /// cursor.
    ///
    /// Concretizations are applied in descending `var_idx` order so earlier
    /// indices don't shift mid-loop. Cross-depth vars inline too: `concretize`
    /// shifts the extraction to each occurrence's depth (sound — every surviving
    /// cross-depth reuse is a genuine shift-variant).
    pub fn inline_useless_nonfrozen(&self, shared: &SharedSearchData<F, O>) -> Option<SearchState<F, O>> {
        let mut targets: Vec<(usize, Id)> = (0..self.pattern.vars.len()).filter(|&k| !self.pattern.var_frozen[k]).filter_map(|k| self.useless_var_eclass(k, shared).map(|id| (k, id))).collect();
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
    /// Callers concretize only non-frozen vars; `pattern.concretize` removes the
    /// slot's state and shifts the rest down, keeping `var_frozen` aligned.
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
                    m.factors.extend(f.decompose(shared.decompose_min_rows));
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

    /// Canonicalizes the pattern's var numbering to DFS first-appearance order
    /// and remaps the match factor slots to match. Call on the winning state
    /// before output/rewrite so the abstraction's variable order is canonical.
    /// Returns the old->new permutation so callers can remap other index-aligned
    /// data (e.g. a `CostSelection`'s `variable_indices`). Row count is preserved
    /// (a bijection on slots), so `num_substs` stays valid.
    pub fn canonicalize_vars(&mut self) -> Vec<usize> {
        let perm = self.pattern.canonicalize_vars();
        for m in &mut self.matches {
            m.factors = m
                .factors
                .iter()
                .map(|f| {
                    // Remap each slot to its canonical index, then re-sort
                    // ascending (Factor invariant), permuting row columns to
                    // match.
                    let mut idx: Vec<(usize, usize)> = f.slots.iter().enumerate().map(|(pos, &s)| (perm[s], pos)).collect();
                    idx.sort_unstable();
                    let new_slots: Vec<usize> = idx.iter().map(|&(ns, _)| ns).collect();
                    let new_rows: Vec<Vec<Id>> = f.rows.iter().map(|row| idx.iter().map(|&(_, pos)| row[pos]).collect()).collect();
                    Factor::new(new_slots, new_rows).expect("canonicalize is a bijection; non-empty rows are preserved")
                })
                .collect();
        }
        perm
    }

    /// Creates the initial search state: a single-variable pattern matching every e-class.
    /// `freeze_rule = true` enables the freeze-based canonical-ordering rule
    /// (best-first); pass `false` to disable the check (e.g. for SMC).
    pub fn new(shared: &SharedSearchData<F, O>, freeze_rule: bool) -> Self {
        let matches = identity_matches(&shared.egraph, shared.root);
        let num_substs = total_substs(&matches);
        Self {
            pattern: Pattern::single_var(),
            matches,
            num_substs,
            freeze_rule,
        }
    }

    /// Applies an action to `self` and returns the resulting child without
    /// cloning the parent's matches (only the surviving transformed factor
    /// rows get allocated in the child — the parent's `Vec<Factor>` data is
    /// not cloned-then-discarded). The pattern is cloned and mutated in
    /// place; the freeze set (`Pattern::var_frozen`) is index-shifted by
    /// `expand`/`reuse` and the freeze policy is applied inline: an `Expand`
    /// freezes every var of lower rank (per `rank`), and a `Reuse` freezes the
    /// merged var when `freeze_reused` is set (the caller passes `false` for the
    /// dominant-reuse short-circuit). Used by best-first and by SMC after
    /// sampling so we don't materialise child states for successors that don't
    /// get picked.
    pub fn apply_action(&self, action: &Action<F::Discriminant<O>>, shared: &SharedSearchData<F, O>, freeze_reused: bool, rank: Option<&[usize]>) -> SearchState<F, O> {
        let mut new_pattern = self.pattern.clone();
        let (new_matches, new_num_substs) = match action {
            Action::Expand { var_idx, op, arity } => {
                let target = F::make(op.clone(), vec![Id::from(0); *arity]);
                // Commit to freezing every var of *lower rank*: expanding the
                // var at rank `r` forbids ever expanding a lower-rank var. Set
                // the parent's `var_frozen` (by rank) before `expand` shifts it
                // into the child layout. Disabled for SMC (no freeze rule, so
                // `rank` is `None`).
                if self.freeze_rule {
                    let rank = rank.expect("freeze_rule requires a rank");
                    let rv = rank[*var_idx];
                    for (j, s) in new_pattern.var_frozen.iter_mut().enumerate() {
                        if rank[j] < rv {
                            *s = true;
                        }
                    }
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
                new_pattern.reuse(var_idx, second_var_idx);
                // Sequencing rule: a non-dominant reuse freezes the merged var.
                // Re-expanding it would re-derive (via the reuse↔expand diamond)
                // a pattern also reachable by expanding the two factors first and
                // reusing afterwards, so freezing it keeps expand-then-reuse as
                // the single canonical path. Frozen (like the expand-freeze rule)
                // means a later-useless merged var is pruned, not inlined — under
                // the forced-expansion cap that prune shrinks the bounded search.
                // Skipped for the dominant short-circuit (the forced sole
                // successor), where freezing would make `f(shared…)` unreachable.
                // No-op under `freeze_rule = false`.
                if self.freeze_rule && freeze_reused {
                    new_pattern.var_frozen[var_idx.min(second_var_idx)] = true;
                }
                Self::build_subset_matches_reuse(&self.matches, var_idx, second_var_idx, shallow_idx, d_a.min(d_b), d_a.max(d_b), shared)
            }
        };
        SearchState {
            pattern: new_pattern,
            matches: new_matches,
            num_substs: new_num_substs,
            freeze_rule: self.freeze_rule,
        }
    }

    /// `CostWithoutVars(p)`: the tree-expanded pattern cost minus its var leaves.
    /// `compute_pattern_size` already tree-expands (counting each `?#k` as one
    /// `sym_var_cost` leaf per occurrence), and `var_occurrences` counts those
    /// occurrences the same way, so subtracting them leaves exactly the concrete
    /// (non-var) skeleton.
    fn concrete_skeleton_cost(&self, weights: &crate::lang::Weights) -> i64 {
        let vars: i64 = self.pattern.var_occurrences.iter().sum::<usize>() as i64 * weights.sym_var_cost as i64;
        crate::cost::compute_pattern_size::<F, O>(&self.pattern, weights) as i64 - vars
    }

    /// `ForcedExpansion(r, p) = Cost(r | p) - Cost(r)` for a single match `r`, given
    /// the precomputed skeleton cost `skel = CostWithoutVars(p)`. Here `Cost(r | p) =
    /// skel + min_σ Σ_{s∈σ} Cost(s)`, the inner min factored over the independent
    /// substitution factors (per factor, the min over its rows of the
    /// occurrence-weighted arg sizes), and `Cost(r)` is the e-class's minimal size.
    fn forced_expansion_at(&self, shared: &SharedSearchData<F, O>, skel: i64, m: &crate::matching::MatchAtEClass) -> i64 {
        let occ = &self.pattern.var_occurrences;
        let holes_min: i64 = m
            .factors
            .iter()
            .map(|f| f.rows.iter().map(|row| f.slots.iter().zip(row).map(|(&slot, &id)| occ[slot] as i64 * shared.egraph[id].data.size as i64).sum::<i64>()).min().expect("factor rows are non-empty"))
            .sum();
        skel + holes_min - shared.egraph[m.root_eclass].data.size as i64
    }

    /// `ForcedExpansion(p) ≤ cap`. Per match `r` computes `ForcedExpansion(r, p)` (see
    /// [`Self::forced_expansion_at`]) and early-exits at the first `r` with `≤ cap`
    /// (one match's work for a valid `p`).
    ///
    /// Only roots that appear in the corpus's size-minimal extraction
    /// (`usage_counts != 0`) are considered.
    ///
    /// Sound to prune on: `ForcedExpansion(p)` is monotone non-decreasing under
    /// expand/reuse — committing a hole swaps a `Cost(r)`-minimal arg for `≥`-cost
    /// structure (`Cost(r | p)` rises, `Cost(r)` fixed) and substs only shrink — so
    /// `ForcedExpansion(p) > cap` ⇒ every descendant `> cap`.
    pub fn within_forced_expansion_cap(&self, shared: &SharedSearchData<F, O>, cap: i64) -> bool {
        let skel = self.concrete_skeleton_cost(&shared.egraph.analysis.weights);
        self.matches.iter().filter(|m| shared.usage_counts.get(&m.root_eclass).is_some_and(|&u| u != 0)).any(|m| self.forced_expansion_at(shared, skel, m) <= cap)
    }

    /// Whether this state is within the `--max-match-set` row cap (`true` when
    /// unset). The blowup guard both drivers apply when admitting a successor.
    pub fn within_match_set_cap(&self, max_match_set: Option<usize>) -> bool {
        let max_factor_rows = self.matches.iter().flat_map(|m| m.factors.iter()).map(|f| f.rows.len()).max().unwrap_or(0);
        max_match_set.is_none_or(|cap| max_factor_rows <= cap)
    }

    /// Renders the size-minimal extraction ("min-term") of `eclass` as a string.
    /// Verbose-diagnostics only.
    pub fn min_term(&self, shared: &SharedSearchData<F, O>, eclass: Id) -> String {
        let mut out = Vec::new();
        let mut memo = rustc_hash::FxHashMap::default();
        crate::egraph_util::build_size_minimal_extraction::<F, O>(&shared.egraph, eclass, &mut out, &mut memo);
        let rec: egg::RecExpr<F::Apply<OpWithVar<O>>> = out.into();
        rec.to_string()
    }

    /// `(ForcedExpansion(p), argmin_r ForcedExpansion(r, p))` — the value paired
    /// with the match root `r` closest to minimal. `None` when `p` has no matches.
    /// See [`Self::within_forced_expansion_cap`] for the per-`r` cost decomposition.
    ///
    /// `lower_bound` is a known lower bound on `ForcedExpansion(p)` — in best-first
    /// the parent's value, since forced-expansion is monotone non-decreasing under
    /// expand/reuse (the same property `within_forced_expansion_cap` relies on),
    /// so no descendant drops below it. We early-exit the moment a match attains
    /// it: the min can't go lower, so the result is exact. Pass `i64::MIN` to scan
    /// fully (no bound). Only the *value* is exact under the bound; the returned
    /// root is then the first attaining match rather than the global argmin — fine
    /// for the heap key (value only) and verbose passes `i64::MIN`.
    pub fn forced_expansion_argmin(&self, shared: &SharedSearchData<F, O>, lower_bound: i64) -> Option<(i64, Id)> {
        let skel = self.concrete_skeleton_cost(&shared.egraph.analysis.weights);
        let mut best: Option<(i64, Id)> = None;
        for m in &self.matches {
            if shared.usage_counts.get(&m.root_eclass).is_none_or(|&u| u == 0) {
                continue;
            }
            let forced = self.forced_expansion_at(shared, skel, m);
            if best.is_none_or(|(b, _)| forced < b) {
                best = Some((forced, m.root_eclass));
                if forced <= lower_bound {
                    break;
                }
            }
        }
        best
    }

    /// The candidate expansion shapes for `?#var_idx`: every distinct
    /// `(discriminant, arity)` its bound eclass admits (free DB-var leaves above
    /// the hole's depth excluded), each with the [`VarShape`] stats summed over
    /// every (match, factor-row) the var appears in. Reads the precomputed
    /// per-eclass shape histograms ([`SharedSearchData::eclass_shapes`]) and keeps
    /// shapes in enode first-appearance order so the emitted action order is
    /// deterministic.
    fn expand_shapes(&self, var_idx: usize, shared: &SharedSearchData<F, O>) -> VarShapes<F, O> {
        let d_k = self.pattern.var_depth[var_idx];
        let usage = |root: Id| shared.usage_counts.get(&root).copied().unwrap_or(1);
        let mut shape_idx: FxHashMap<Shape<F, O>, usize> = FxHashMap::default();
        let mut shapes: VarShapes<F, O> = Vec::new();
        for m in &self.matches {
            let (fi, pos) = m.locate_slot(var_idx);
            let f = &m.factors[fi];
            // Each row of the owning factor stands in for `total/|f.rows|`
            // full substs (the product of the other factors), so weight the
            // per-row node contributions by that multiplier. `support` adds the
            // usage factor on top; the rank stats deliberately omit it.
            let w = m.num_substs() / f.rows.len();
            let wu = usage(m.root_eclass) * w;
            for row in &f.rows {
                for (disc, arity, count) in &shared.eclass_shapes[usize::from(row[pos])] {
                    if invalid_literal_expansion(disc, d_k) {
                        continue;
                    }
                    let key = (disc.clone(), *arity);
                    match shape_idx.get(&key) {
                        Some(&idx) => {
                            shapes[idx].support += count * wu;
                            shapes[idx].expand_enodes += count * w;
                            shapes[idx].substs += w;
                        }
                        None => {
                            shape_idx.insert(key.clone(), shapes.len());
                            shapes.push(VarShape {
                                shape: key,
                                support: count * wu,
                                expand_enodes: count * w,
                                substs: w,
                            });
                        }
                    }
                }
            }
        }
        shapes
    }

    /// Non-expansive ordering over all vars, built from each var's [`Self::expand_shapes`].
    /// `f(k)` = the mean enodes per *matching* subst of `k`'s most-exploding
    /// (least-clean) expansion.
    ///
    /// Returns `(shapes, rank, order)`. `rank` (var -> rank) / `order` (rank
    /// -> var) are threaded to the freeze rule, the `max_arity` skip, and the
    /// emission order so those act on `f` rather than creation order, *without*
    /// renumbering the pattern. SMC (`freeze_rule = false`) keeps creation order:
    /// `rank`/`order` are the identity.
    fn shape_pass(&self, shared: &SharedSearchData<F, O>) -> (Vec<VarShapes<F, O>>, Vec<usize>, Vec<usize>) {
        let n = self.pattern.vars.len();
        let shapes: Vec<VarShapes<F, O>> = (0..n).map(|k| self.expand_shapes(k, shared)).collect();
        let (rank, order) = if self.freeze_rule && n > 1 {
            let fval: Vec<f64> = (0..n).map(|k| shapes[k].iter().map(|s| s.expand_enodes as f64 / s.substs as f64).fold(-f64::INFINITY, f64::max)).collect();
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|&a, &b| fval[a].partial_cmp(&fval[b]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
            let mut rank = vec![0usize; n];
            for (r, &k) in order.iter().enumerate() {
                rank[k] = r;
            }
            (rank, order)
        } else {
            ((0..n).collect(), (0..n).collect())
        };
        (shapes, rank, order)
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
    /// rule: any frozen `var_idx` (`self.pattern.var_frozen[var_idx]`) or
    /// `var_idx > max_arity` is skipped before the action is even constructed.
    /// SMC passes `max_arity = usize::MAX` and runs with `freeze_rule = false`,
    /// so the filter is a no-op for it.
    ///
    /// `support` is the (m,s)-pair count feeding the SMC weighting; it equals
    /// the surviving subst count, so `support > 0` ⇒ non-empty child.
    /// `support == self.num_substs` ⇒ dominant reuse (every subst already has
    /// the two vars unified); short-circuited unless disabled by
    /// `--no-opt-dominance-reuse`. Expand actions are emitted whenever
    /// `support > 0`; `subset_matches` then guarantees the child's match set is
    /// non-empty.
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
        // Per-var expansion shapes (first-appearance order, with support) and the
        // `rank`/`order`, in one egraph pass. See [`Self::shape_pass`].
        let (shapes, rank, order) = self.shape_pass(shared);

        // `reuse_pairs` is the best-first reuse-order canonicalization (SMC,
        // freeze_rule = false, ignores it). It holds exactly the reuse pairs
        // currently allowed; `Reuse(i, j)` is canonical iff `(i, j)` is in it. It
        // is maintained in `Pattern` (see `reuse`/`expand`): a reuse stales every
        // lexicographically-earlier pair — so within an expand level reuses run
        // in non-decreasing pair order, killing the absorption-order duplicates —
        // while an expand stales every pre-existing pair (both endpoints predate
        // it, so reusing them now is a reuse<->expand diamond reachable by reusing
        // first) and keeps only pairs that involve a new child. Together these
        // make every reachable pattern reachable exactly once.
        let enforce_reusable = self.freeze_rule;
        for i in 0..n {
            for j in (i + 1)..n {
                let di = self.pattern.var_depth[i];
                let dj = self.pattern.var_depth[j];
                // Skip unless this pair is still allowed by the canonical
                // reuse order (see `reuse_pairs` above).
                if enforce_reusable && !self.pattern.reuse_pairs.contains(&(i, j)) {
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
                    let child = self.apply_action(&Action::Reuse { keep: i, drop: j }, shared, false, Some(&rank));
                    return SuccessorEnum::Dominant { child, support };
                }
                out.push((Action::Reuse { keep: i, drop: j }, support));
            }
        }
        // Emit expands in rank order so the `max_arity` cap and heap tie-breaks
        // act on the f-order. The freeze rule (`var_frozen`) and `max_arity`
        // are no-ops for SMC (freeze_rule = false, identity order, max_arity =
        // usize::MAX).
        for &var_idx in &order {
            if rank[var_idx] > max_arity {
                continue;
            }
            if self.freeze_rule && self.pattern.var_frozen[var_idx] {
                continue;
            }
            // Emit from the shapes `shape_pass` already computed (via
            // `expand_shapes`) for every var — no recomputation here.
            for s in &shapes[var_idx] {
                let (op, arity) = &s.shape;
                out.push((Action::Expand { var_idx, op: op.clone(), arity: *arity }, s.support));
            }
        }
        SuccessorEnum::All { actions: out, rank }
    }
}

/// Precomputes each eclass's `(discriminant, arity, count)` shape histogram in
/// enode first-appearance order. Works because e-graph is never mutated during search.
fn compute_eclass_shapes<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>) -> Vec<EclassShapeHist<F, O>> {
    let len = egraph.classes().map(|c| usize::from(c.id)).max().map_or(0, |m| m + 1);
    let mut out: Vec<EclassShapeHist<F, O>> = vec![Vec::new(); len];
    for class in egraph.classes() {
        let mut idx: FxHashMap<Shape<F, O>, usize> = FxHashMap::default();
        let hist = &mut out[usize::from(class.id)];
        for node in &class.nodes {
            let key = (node.discriminant(), node.children().len());
            match idx.get(&key) {
                Some(&p) => hist[p].2 += 1,
                None => {
                    idx.insert(key.clone(), hist.len());
                    hist.push((key.0, key.1, 1));
                }
            }
        }
    }
    out
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
    let eclass_shapes = compute_eclass_shapes::<F, O>(&egraph);
    // Require decompose_min_rows <= cap: the cap prunes factors over `cap` rows,
    // so each must first have had a chance to decompose — else a benign
    // independent product in `(cap, decompose_min_rows)` rows is pruned as a
    // fake blowup.
    if let Some(cap) = args.max_match_set {
        assert!(args.decompose_min_rows <= cap, "--decompose-min-rows ({}) must be <= --max-match-set ({cap}); otherwise the cap prunes un-decomposed factors", args.decompose_min_rows);
    }
    let shared = SharedSearchData {
        egraph,
        root,
        follow: follow_expr,
        usage_counts,
        check_slow: args.check_slow,
        shift_clamp,
        eclass_shapes,
        decompose_min_rows: args.decompose_min_rows,
    };
    let cache = crate::cost::CostCache::new(&shared.egraph, root);
    let initial = SearchState::new(&shared, false);
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
