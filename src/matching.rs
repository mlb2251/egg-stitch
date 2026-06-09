use crate::lang::{StitchEgraph, StitchLanguage};
use egg::Id;
use rustc_hash::FxHashSet;

/// One factor of a match location's substitution set: a relation over a subset
/// of the pattern's variable slots. The full substitution set at a match
/// location is the cartesian product of its factors (see [`MatchAtEClass`]).
///
/// Storing the factors instead of the flattened product keeps memory and
/// cost-evaluation proportional to `Σ|factor|` rather than `∏|factor|` whenever
/// the slots decompose into independent groups (which they very often do — two
/// pattern branches with no shared variable yield independent factors).
///
/// Invariants, all upheld by [`Factor::new`]:
/// - `slots` is strictly ascending.
/// - every row in `rows` has length `slots.len()`.
/// - `rows` is a *set*: no duplicate rows, sorted for determinism, non-empty.
#[derive(Debug, Clone)]
pub struct Factor {
    pub slots: Vec<usize>,
    pub rows: Vec<Vec<Id>>,
}

impl Factor {
    /// Builds a factor from `slots` (assumed ascending) and `rows`, sorting and
    /// deduplicating the rows so the set invariant holds. Returns `None` when
    /// `rows` is empty — an empty factor makes the whole match's product empty,
    /// so callers drop the match.
    pub fn new(slots: Vec<usize>, mut rows: Vec<Vec<Id>>) -> Option<Self> {
        if rows.is_empty() {
            return None;
        }
        rows.sort_unstable();
        rows.dedup();
        Some(Self { slots, rows })
    }

    /// Position of `slot` within `self.slots`, or `None` if not covered.
    pub fn pos_of(&self, slot: usize) -> Option<usize> {
        self.slots.iter().position(|&s| s == slot)
    }

    /// Splits `self` into the finest set of independent sub-factors whose
    /// cartesian product equals `self.rows` (as a set), falling back to
    /// `[self]` when the relation doesn't decompose.
    ///
    /// Two slot-positions are "entangled" when their joint projection has fewer
    /// rows than the product of their individual projections (i.e. not every
    /// value-pair occurs). Connected components of the pairwise-entanglement
    /// graph are the candidate blocks; the split is committed only after the
    /// exact check `∏|proj_block| == |rows|` (which holds iff `rows` equals the
    /// product of the blocks' projections, since `rows ⊆ ∏ proj` always).
    fn decompose(self) -> Vec<Factor> {
        let n = self.slots.len();
        if n <= 1 {
            return vec![self];
        }
        let nrows = self.rows.len();
        // Distinct value count of the projection onto `cols` (positions).
        let proj_size = |cols: &[usize]| -> usize {
            let mut set: FxHashSet<Vec<Id>> = FxHashSet::default();
            for r in &self.rows {
                set.insert(cols.iter().map(|&c| r[c]).collect());
            }
            set.len()
        };
        let single: Vec<usize> = (0..n).map(|p| proj_size(&[p])).collect();
        // Union-find over positions, joining entangled pairs.
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for p in 0..n {
            for q in (p + 1)..n {
                let (rp, rq) = (find(&mut parent, p), find(&mut parent, q));
                if rp == rq {
                    continue;
                }
                if proj_size(&[p, q]) < single[p] * single[q] {
                    parent[rp] = rq;
                }
            }
        }
        // Gather positions into components, preserving ascending order.
        let mut blocks: Vec<Vec<usize>> = Vec::new();
        let mut root_to_block: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for p in 0..n {
            let r = find(&mut parent, p);
            let idx = *root_to_block.entry(r).or_insert_with(|| {
                blocks.push(Vec::new());
                blocks.len() - 1
            });
            blocks[idx].push(p);
        }
        if blocks.len() <= 1 {
            return vec![self];
        }
        // Exact decomposition check: bail (keep coarse) on higher-order
        // entanglement that pairwise independence misses. A valid split has
        // `∏|proj_block| == nrows` exactly (no overflow, since it equals
        // `nrows`); `checked_mul` returning `None` means the product already
        // exceeds `usize`, which is `> nrows`, so we bail in that case too.
        let prod = blocks.iter().map(|b| proj_size(b)).try_fold(1usize, |acc, x| acc.checked_mul(x));
        if prod != Some(nrows) {
            return vec![self];
        }
        blocks
            .into_iter()
            .map(|b| {
                let slots: Vec<usize> = b.iter().map(|&p| self.slots[p]).collect();
                let rows: Vec<Vec<Id>> = self.rows.iter().map(|r| b.iter().map(|&p| r[p]).collect()).collect();
                Factor::new(slots, rows).expect("non-empty source rows ⇒ non-empty projection")
            })
            .collect()
    }
}

/// All the ways the current pattern can match at a specific e-class, stored as a
/// cartesian factoring of the substitution set (see [`Factor`]).
/// Invariant: `root_eclass` and all ids in every factor row are canonical
/// (the egraph isn't unioned during search). The factors' `slots` partition
/// `0..arity` where `arity` is the pattern's variable count.
#[derive(Debug, Clone)]
pub struct MatchAtEClass {
    pub root_eclass: Id,
    pub factors: Vec<Factor>,
}

impl MatchAtEClass {
    /// Creates a match for e-class `c` with a single-variable pattern: one
    /// factor over slot 0 with the lone row `[c]`.
    pub fn identity_match(c: Id) -> Self {
        Self {
            root_eclass: c,
            factors: vec![Factor { slots: vec![0], rows: vec![vec![c]] }],
        }
    }

    /// Number of full substitutions = product of the factors' row counts.
    /// Factors are never empty, so this is `≥ 1`.
    pub fn num_substs(&self) -> usize {
        self.factors.iter().map(|f| f.rows.len()).product()
    }

    /// The pattern arity implied by this match: the total number of slots
    /// across all factors (they partition `0..arity`).
    pub fn arity(&self) -> usize {
        self.factors.iter().map(|f| f.slots.len()).sum()
    }

    /// `(factor_index, position_within_factor)` for `slot`. Panics if no factor
    /// covers it (a broken partition invariant).
    pub fn locate_slot(&self, slot: usize) -> (usize, usize) {
        for (fi, f) in self.factors.iter().enumerate() {
            if let Some(pos) = f.pos_of(slot) {
                return (fi, pos);
            }
        }
        panic!("slot {slot} not covered by any factor");
    }

    /// Materialises the full cartesian product as flat `Vec<Id>` substitutions,
    /// each indexed by slot. Used by the lambda-capture candidate path, the
    /// rewrite builder, and `check_slow` — i.e. consumers that genuinely need
    /// every joint assignment. Hot search/cost paths read factors directly.
    pub fn all_substs(&self) -> Vec<Vec<Id>> {
        let arity = self.arity();
        let mut acc: Vec<Vec<Id>> = vec![vec![Id::from(0); arity]];
        for f in &self.factors {
            let mut next: Vec<Vec<Id>> = Vec::with_capacity(acc.len() * f.rows.len());
            for base in &acc {
                for row in &f.rows {
                    let mut t = base.clone();
                    for (p, &slot) in f.slots.iter().enumerate() {
                        t[slot] = row[p];
                    }
                    next.push(t);
                }
            }
            acc = next;
        }
        acc
    }
}

/// Returns one identity match per e-class in the egraph, skipping the root
/// e-class. The root holds the synthetic `(programs ...)` node that wraps the
/// whole corpus; letting the search match there produces abstractions like
/// `(programs ?#0 ?#0)` that collapse the program list itself, which is never
/// what we want.
pub fn identity_matches<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> Vec<MatchAtEClass> {
    let root = egraph.find(root);
    egraph.classes().filter(|c| c.id != root).map(|c| MatchAtEClass::identity_match(c.id)).collect()
}

/// Rebuilds a factor's row set with `build` mapping each input row to zero or
/// more output rows, then runs decomposition so any independence the
/// transformation exposed is captured. Returns the resulting (possibly split)
/// factors, or `None` when no output rows survive (caller drops the match).
pub fn rebuild_factor(slots: Vec<usize>, src_rows: &[Vec<Id>], mut build: impl FnMut(&[Id], &mut Vec<Vec<Id>>)) -> Option<Vec<Factor>> {
    let mut rows: Vec<Vec<Id>> = Vec::new();
    for r in src_rows {
        build(r, &mut rows);
    }
    Factor::new(slots, rows).map(Factor::decompose)
}

/// Public wrapper so search code can decompose a freshly merged factor.
pub fn decompose_factor(f: Factor) -> Vec<Factor> {
    f.decompose()
}
