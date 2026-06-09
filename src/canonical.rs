//! Canonical-form keying for best-first's seen-set.
//!
//! Given a candidate pattern P (a `RecExpr` in `F::Apply<OpWithVar<O>>`), compute
//! a deterministic `u64` "canonical key" that is invariant under the equational
//! theory the search should quotient by, so best-first can drop semantically
//! redundant duplicates from its seen-set. Two patterns with the same key are
//! treated as the same abstraction.
//!
//! ## Theory quotiented (structural, not via saturation)
//!
//! The previous implementation saturated each pattern in a throwaway egraph and
//! hashed the min-cost extraction. That was explosive (AC rewriting blows past
//! any node cap) and unreliable. This version instead computes the normal form
//! *structurally*, in O(size):
//!
//! - **`+` is associative-commutative with constant folding.** Nested sums are
//!   flattened, numeric-literal operands are summed (subsuming the `two..eight`
//!   number rules and `add_zero`), and the operand multiset is canonicalised.
//! - **Metavariables are α-renumbered** by (occurrence-count desc, first-
//!   occurrence asc), so e.g. `2·?a + ?b` and `?a + 2·?b` map to one key.
//!
//! `*` is left ordered: the current rule set has `mul_comm` but no `mult_assoc`,
//! so the corpus egraph is not `*`-associative-closed and flattening it would be
//! unsound. Extend [`is_ac`] to include `"*"` if/when `mult_assoc` is added.
//!
//! ## Soundness
//!
//! Dedup skips costing the dropped pattern, so it is sound only if AC-variant
//! patterns match the *same* e-classes — i.e. the corpus egraph is closed under
//! the same theory (`add_assoc`/`add_comm`). We assume that closure (the egraph
//! is built by saturating with these rules). α-renaming is unconditionally sound.
//! Non-arithmetic rule equivalences (`t_1`, `t_compose`, `repeat_unroll`, …) are
//! deliberately *not* modelled here; this normal form targets the AC arithmetic
//! blow-up only.
//!
//! No rule file → the seen-set is disabled ([`CanonicalChecker::trivial`]).

use crate::lang::{LanguageFamily, OpWithVar, StitchAnalysis, StitchDisc, StitchLanguage, StitchOp};
use egg::{Id, RecExpr, Rewrite, Var};
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};

/// Pattern-side language: program ops extended with metavariable leaves.
pub type PatternLang<F, O> = <F as LanguageFamily>::Apply<OpWithVar<O>>;
pub type PatternRules<F, O> = Vec<Rewrite<PatternLang<F, O>, StitchAnalysis>>;

/// True iff `name` is an associative-commutative operator we canonicalise.
/// Only `+` for now — see the module docs for why `*` is excluded.
fn is_ac(name: &str) -> bool {
    name == "+"
}

/// Holds whether canonical dedup is active (a rule file was supplied) plus a
/// memo from a pattern's plain structural hash → its canonical key, so the
/// normal form is computed once per distinct pattern.
pub struct CanonicalChecker<F: LanguageFamily, O: StitchOp> {
    enabled: bool,
    memo: FxHashMap<u64, u64>,
    /// Memo hits (canonicalisation work avoided).
    pub memo_hits: usize,
    _marker: std::marker::PhantomData<(F, O)>,
}

impl<F: LanguageFamily, O: StitchOp> CanonicalChecker<F, O> {
    /// `rules` is used only to decide whether to enable the seen-set; its
    /// content does not affect the (structural) canonical key. `weights` is
    /// unused — folding is exact arithmetic — but kept for call-site stability.
    pub fn new(rules: PatternRules<F, O>, _weights: crate::lang::Weights) -> Self {
        Self {
            enabled: !rules.is_empty(),
            memo: FxHashMap::default(),
            memo_hits: 0,
            _marker: std::marker::PhantomData,
        }
    }

    /// True when canonical dedup is off (no rule file): callers skip the check.
    pub fn trivial(&self) -> bool {
        !self.enabled
    }

    /// Canonical key of `pattern` under the AC + α normal form. Memoised on the
    /// pattern's structural hash.
    pub fn canonical_key(&mut self, pattern: &RecExpr<PatternLang<F, O>>) -> u64 {
        let pkey = hash_recexpr::<PatternLang<F, O>>(pattern);
        if let Some(&v) = self.memo.get(&pkey) {
            self.memo_hits += 1;
            return v;
        }
        let canonical = structural_canonical_hash::<PatternLang<F, O>>(pattern);
        self.memo.insert(pkey, canonical);
        canonical
    }
}

/// Intermediate canonical term: metavars keep their identity (`Var`) until the
/// global α-renumbering, constants are folded, and `+` nodes hold a canonicalised
/// operand multiset.
enum Term {
    Var(Var),
    Const(f64),
    Node { name: String, ac: bool, kids: Vec<Term> },
}

/// Structural AC + α + constant-fold canonical hash of `pattern`.
fn structural_canonical_hash<L: StitchLanguage>(pattern: &RecExpr<L>) -> u64 {
    let root = Id::from(pattern.as_ref().len() - 1);
    let term = build_term::<L>(pattern, root);
    // Canonical metavar ids: order by occurrence count (desc), ties broken by
    // first-occurrence order. `order` is already in first-occurrence order and
    // `sort_by` is stable, so the tie-break falls out for free.
    let mut order: Vec<Var> = Vec::new();
    let mut counts: FxHashMap<Var, usize> = FxHashMap::default();
    collect_vars(&term, &mut order, &mut counts);
    order.sort_by(|a, b| counts[b].cmp(&counts[a]));
    let var_id: FxHashMap<Var, u32> = order.iter().enumerate().map(|(i, v)| (*v, i as u32)).collect();
    hash_term(&term, &var_id)
}

/// Lowers `expr[id]` into a [`Term`], flattening/folding/sorting `+` nodes.
/// Children are built first, so each `+` child is already flattened — one level
/// of splicing suffices to fully flatten.
fn build_term<L: StitchLanguage>(expr: &RecExpr<L>, id: Id) -> Term {
    let n = &expr[id];
    let disc = n.discriminant();
    if let Some(v) = disc.as_var() {
        return Term::Var(v);
    }
    let name = disc.to_string();
    let kids_ids = n.children();
    if kids_ids.is_empty() {
        if let Ok(f) = name.parse::<f64>() {
            return Term::Const(f);
        }
        return Term::Node { name, ac: false, kids: Vec::new() };
    }
    let kids: Vec<Term> = kids_ids.iter().map(|&c| build_term::<L>(expr, c)).collect();
    if !is_ac(&name) {
        return Term::Node { name, ac: false, kids };
    }
    // Flatten same-op children.
    let mut flat: Vec<Term> = Vec::new();
    for k in kids {
        if let Term::Node { name: kn, ac: true, kids: kk } = k {
            if kn == name {
                flat.extend(kk);
            } else {
                flat.push(Term::Node { name: kn, ac: true, kids: kk });
            }
        } else {
            flat.push(k);
        }
    }
    // Fold constant operands (additive identity 0).
    let mut acc = 0.0;
    let mut saw_const = false;
    let mut rest: Vec<Term> = Vec::new();
    for k in flat {
        match k {
            Term::Const(c) => {
                saw_const = true;
                acc += c;
            }
            other => rest.push(other),
        }
    }
    if saw_const && (acc != 0.0 || rest.is_empty()) {
        rest.push(Term::Const(acc));
    }
    // Canonical operand order: var-blind shape hash (final multiset hash is
    // order-independent, but a deterministic order keeps the structure stable).
    rest.sort_by_key(shape_hash);
    if rest.len() == 1 {
        return rest.pop().expect("len checked");
    }
    Term::Node { name, ac: true, kids: rest }
}

/// Var-blind structural hash: every metavar collapses to one sentinel, so it
/// orders operands by shape without committing to metavar identities.
fn shape_hash(t: &Term) -> u64 {
    let mut h = FxHasher::default();
    match t {
        Term::Var(_) => 0u8.hash(&mut h),
        Term::Const(c) => {
            1u8.hash(&mut h);
            c.to_bits().hash(&mut h);
        }
        Term::Node { name, ac, kids } => {
            2u8.hash(&mut h);
            name.hash(&mut h);
            let mut ks: Vec<u64> = kids.iter().map(shape_hash).collect();
            if *ac {
                ks.sort_unstable();
            }
            ks.hash(&mut h);
        }
    }
    h.finish()
}

/// Records each metavar's first-occurrence order and total count.
fn collect_vars(t: &Term, order: &mut Vec<Var>, counts: &mut FxHashMap<Var, usize>) {
    match t {
        Term::Var(v) => {
            if !counts.contains_key(v) {
                order.push(*v);
            }
            *counts.entry(*v).or_insert(0) += 1;
        }
        Term::Const(_) => {}
        Term::Node { kids, .. } => {
            for k in kids {
                collect_vars(k, order, counts);
            }
        }
    }
}

/// Final canonical hash with metavars mapped to their canonical ids. AC nodes
/// hash the sorted multiset of child hashes (order-independent, multiplicity-
/// sensitive); other nodes hash children in order.
fn hash_term(t: &Term, var_id: &FxHashMap<Var, u32>) -> u64 {
    let mut h = FxHasher::default();
    match t {
        Term::Var(v) => {
            0u8.hash(&mut h);
            var_id[v].hash(&mut h);
        }
        Term::Const(c) => {
            1u8.hash(&mut h);
            c.to_bits().hash(&mut h);
        }
        Term::Node { name, ac, kids } => {
            2u8.hash(&mut h);
            name.hash(&mut h);
            let mut ks: Vec<u64> = kids.iter().map(|k| hash_term(k, var_id)).collect();
            if *ac {
                ks.sort_unstable();
            }
            ks.hash(&mut h);
        }
    }
    h.finish()
}

/// Tree-walk a `RecExpr` from its root and produce a recursive structural hash
/// of `(discriminant, [child_hashes...])`. Used to memoise canonical keys per
/// distinct pattern (no normalisation — plain syntactic identity).
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
