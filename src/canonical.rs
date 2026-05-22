//! Canonical-form keying for best-first's seen-set.
//!
//! Given a candidate pattern P (a `RecExpr` in `F::Apply<OpWithVar<O>>`), build a
//! fresh egraph in that language, insert P, saturate with the user's rewrite
//! rules, and compute a deterministic `u64` "canonical key" for the root
//! eclass. Two patterns equivalent under the rules return the same key, so
//! best-first can dedupe semantic duplicates in its seen-set.
//!
//! The key is a recursive hash: for each eclass, among its min-cost enodes,
//! pick the one whose `hash(disc, [child_canonical_keys...])` is smallest;
//! that minimum is the eclass's canonical key. The fixed-point iteration
//! handles cycles introduced by productive rules (e.g. `c => (T c (M …))`)
//! without ever materialising a string.
//!
//! No rule file → no equivalences → canonical key equals a plain structural
//! hash of the pattern and the seen-set adds no extra dedup beyond syntactic.

use crate::lang::{LanguageFamily, OpWithVar, StitchAnalysis, StitchDisc, StitchLanguage, StitchOp, Weights};
use egg::{EGraph, Id, RecExpr, Rewrite, Runner};
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};

/// Pattern-side language: program ops extended with metavariable leaves.
pub type PatternLang<F, O> = <F as LanguageFamily>::Apply<OpWithVar<O>>;
pub type PatternRules<F, O> = Vec<Rewrite<PatternLang<F, O>, StitchAnalysis>>;

/// Holds the rewrite rules (parsed against the pattern language) and a memo
/// from pattern-structure hash → canonical-form hash. Used by best-first to
/// dedupe patterns by *semantic* equivalence under the rules (not just
/// syntactic equality): two patterns that saturate to the same eclass return
/// the same `u64` canonical key, so the seen-set drops the second.
pub struct CanonicalChecker<F: LanguageFamily, O: StitchOp> {
    rules: PatternRules<F, O>,
    weights: Weights,
    memo: FxHashMap<u64, u64>,
    /// Memo hits (saturation work avoided).
    pub memo_hits: usize,
}

impl<F: LanguageFamily, O: StitchOp> CanonicalChecker<F, O> {
    pub fn new(rules: PatternRules<F, O>, weights: Weights) -> Self {
        Self {
            rules,
            weights,
            memo: FxHashMap::default(),
            memo_hits: 0,
        }
    }

    /// True when the user supplied no rule file: canonical key == pattern key,
    /// so the canonical step is a pass-through.
    pub fn trivial(&self) -> bool {
        self.rules.is_empty()
    }

    /// Returns the canonical-form hash of `pattern`'s eclass after rule
    /// saturation. Patterns in the same equivalence class return identical
    /// keys; with no rules, just returns the pattern's own structural hash.
    pub fn canonical_key(&mut self, pattern: &RecExpr<PatternLang<F, O>>) -> u64 {
        let pkey = hash_recexpr::<PatternLang<F, O>>(pattern);
        if self.rules.is_empty() {
            return pkey;
        }
        if let Some(&v) = self.memo.get(&pkey) {
            self.memo_hits += 1;
            return v;
        }
        let canonical = canonical_hash::<F, O>(pattern, &self.rules, self.weights);
        self.memo.insert(pkey, canonical);
        canonical
    }
}

/// Build a fresh egraph, add `pattern`, saturate `rules`, return the canonical
/// hash of the resulting root eclass.
fn canonical_hash<F: LanguageFamily, O: StitchOp>(pattern: &RecExpr<PatternLang<F, O>>, rules: &PatternRules<F, O>, weights: Weights) -> u64 {
    let mut egraph: EGraph<PatternLang<F, O>, StitchAnalysis> = EGraph::new(StitchAnalysis::new(weights));
    let root = egraph.add_expr(pattern);
    egraph.rebuild();
    // Tight saturation budget: per-pattern saturation runs once per distinct
    // pattern explored by best-first (which can be thousands). Productive rules
    // (e.g. nuts-bolts' `c => (T c (M …))`) explode the egraph; the caps below
    // prevent any single check from dominating the search.
    let mut runner: Runner<PatternLang<F, O>, StitchAnalysis> = Runner::new(StitchAnalysis::new(weights)).with_egraph(egraph).with_iter_limit(4).with_node_limit(2_000).with_time_limit(std::time::Duration::from_millis(50));
    runner = runner.run(rules);
    runner.egraph.rebuild();
    let root = runner.egraph.find(root);
    let cost = compute_min_costs::<PatternLang<F, O>>(&runner.egraph, &weights);
    let hashes = build_class_hashes::<PatternLang<F, O>>(&runner.egraph, &weights, &cost);
    *hashes.get(&root).expect("root eclass has no canonical hash")
}

/// Tree-walk a `RecExpr` from its last (root) node and produce a recursive
/// hash of `(discriminant, [child_hashes...])`. The vec layout is irrelevant —
/// we always walk children via id, so DAG-shared subtrees are unfolded into
/// the resulting tree hash just as the egraph extraction would.
fn hash_recexpr<L: StitchLanguage>(expr: &RecExpr<L>) -> u64 {
    let nodes = expr.as_ref();
    let root = Id::from(nodes.len() - 1);
    fn walk<L: StitchLanguage>(nodes: &[L], id: Id) -> u64 {
        let n = &nodes[usize::from(id)];
        let mut hasher = FxHasher::default();
        n.discriminant().hash(&mut hasher);
        for &c in n.children() {
            walk::<L>(nodes, c).hash(&mut hasher);
        }
        hasher.finish()
    }
    walk::<L>(nodes, root)
}

/// Standard fixed-point cost DP: per-eclass min weighted size over enodes whose
/// children all have costs. Eclasses reachable only via cycles get no entry.
fn compute_min_costs<L: StitchLanguage>(egraph: &EGraph<L, StitchAnalysis>, weights: &Weights) -> FxHashMap<Id, u64> {
    let mut cost: FxHashMap<Id, u64> = FxHashMap::default();
    loop {
        let mut changed = false;
        for ec in egraph.classes() {
            let id = egraph.find(ec.id);
            let best = ec.nodes.iter().filter_map(|n| node_cost::<L>(n, weights, &cost, egraph)).min();
            if let Some(b) = best {
                match cost.get(&id) {
                    Some(&prev) if prev <= b => {}
                    _ => {
                        cost.insert(id, b);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    cost
}

fn node_cost<L: StitchLanguage>(n: &L, weights: &Weights, cost: &FxHashMap<Id, u64>, egraph: &EGraph<L, StitchAnalysis>) -> Option<u64> {
    let mut total = n.discriminant().intrinsic_size(weights) as u64;
    for &c in n.children() {
        let c = egraph.find(c);
        total = total.checked_add(*cost.get(&c)?)?;
    }
    Some(total)
}

/// For each eclass that has a min cost, compute its canonical `u64` hash:
/// among min-cost enodes whose child hashes are all known, take the smallest
/// `hash(disc, [child_hash...])`. Fixed-point iteration — cyclic egraphs (e.g.
/// from productive rules) need multiple passes before all hashes stabilise.
fn build_class_hashes<L: StitchLanguage>(egraph: &EGraph<L, StitchAnalysis>, weights: &Weights, cost: &FxHashMap<Id, u64>) -> FxHashMap<Id, u64> {
    let mut memo: FxHashMap<Id, u64> = FxHashMap::default();
    loop {
        let mut changed = false;
        for ec in egraph.classes() {
            let id = egraph.find(ec.id);
            let Some(&target) = cost.get(&id) else { continue };
            let mut best: Option<u64> = None;
            for n in &ec.nodes {
                if node_cost::<L>(n, weights, cost, egraph) != Some(target) {
                    continue;
                }
                let mut hasher = FxHasher::default();
                n.discriminant().hash(&mut hasher);
                let mut ok = true;
                for &c in n.children() {
                    let c = egraph.find(c);
                    match memo.get(&c) {
                        Some(&h) => h.hash(&mut hasher),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let h = hasher.finish();
                match best {
                    None => best = Some(h),
                    Some(cur) if h < cur => best = Some(h),
                    _ => {}
                }
            }
            if let Some(h) = best
                && memo.get(&id) != Some(&h)
            {
                memo.insert(id, h);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    memo
}
