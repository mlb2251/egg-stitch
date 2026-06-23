use egg::Id;
use rustc_hash::{FxHashMap, FxHashSet};

/// Default minimum factor row count before [`Factor::decompose`] attempts a
/// split (overridable via `--decompose-min-rows`). Detecting independence costs
/// `O(slots² · rows)`; below this the saved cost-evaluation work doesn't repay
/// that scan, so we keep the factor whole (correctness is independent of
/// factoring granularity). Large factors — the ones whose `∏` blow-up actually
/// hurts — are well above this.
///
/// Must stay `≤` the `--max-match-set` row cap (asserted in `setup_search`):
/// the cap prunes factors with more than `cap` rows, so every prunable factor
/// must first have been offered decomposition, else a benign independent
/// product just under the decompose threshold is pruned as if it were entangled.
pub const DEFAULT_DECOMPOSE_MIN_ROWS: usize = 48;

/// One factor of a match location's substitution set: a relation over a subset
/// of the pattern's variable slots. The full substitution set at a match
/// location is the cartesian product of its factors (see
/// [`crate::matching::MatchAtEClass`]).
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
    /// cartesian product equals `self.rows` (as a set), returning `[self]` when
    /// the relation doesn't decompose (the common small-factor case).
    ///
    /// Two slot-positions are "entangled" when their joint projection has fewer
    /// rows than the product of their individual projections (i.e. not every
    /// value-pair occurs). Connected components of the pairwise-entanglement
    /// graph are the candidate blocks; the split is committed only after the
    /// exact check `∏|proj_block| == |rows|` (which holds iff `rows` equals the
    /// product of the blocks' projections, since `rows ⊆ ∏ proj` always).
    pub fn decompose(self, min_rows: usize) -> Vec<Factor> {
        let n = self.slots.len();
        let nrows = self.rows.len();
        if n <= 1 || nrows < min_rows {
            return vec![self];
        }
        // Packed-int projections on the hot pairwise scan: an `Id` is a `u32`,
        // so a single column packs into a `u64` and a column pair into a `u64`
        // (`a << 32 | b`), avoiding a `Vec` allocation per row per projection.
        let id = |x: Id| usize::from(x) as u64;
        let single: Vec<usize> = (0..n)
            .map(|p| {
                let mut set: FxHashSet<u64> = FxHashSet::default();
                for r in &self.rows {
                    set.insert(id(r[p]));
                }
                set.len()
            })
            .collect();
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
                let mut set: FxHashSet<u64> = FxHashSet::default();
                for r in &self.rows {
                    set.insert((id(r[p]) << 32) | id(r[q]));
                }
                if set.len() < single[p] * single[q] {
                    parent[rp] = rq;
                }
            }
        }
        // Gather positions into components, preserving ascending order.
        let mut blocks: Vec<Vec<usize>> = Vec::new();
        let mut root_to_block: FxHashMap<usize, usize> = FxHashMap::default();
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
        // Multi-slot block projection needs a `Vec` key, but this runs only
        // once a split candidate exists — off the hot pairwise path.
        let proj_block = |cols: &[usize]| -> usize {
            let mut set: FxHashSet<Vec<Id>> = FxHashSet::default();
            for r in &self.rows {
                set.insert(cols.iter().map(|&c| r[c]).collect());
            }
            set.len()
        };
        let prod = blocks.iter().map(|b| proj_block(b)).try_fold(1usize, |acc, x| acc.checked_mul(x));
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

/// Separable minimum of an additive per-slot cost over a factored substitution
/// set: `Σ over factors of (min over that factor's rows of Σ over its slots of
/// cost(slot, value))`. Since the cost is additive across slots, this equals the
/// minimum total cost over the full cartesian product — computed without
/// materialising it. Factors are non-empty, so every `min` is defined.
pub fn factored_min(factors: &[Factor], cost: impl Fn(usize, Id) -> i64) -> i64 {
    factors.iter().map(|f| f.rows.iter().map(|row| f.slots.iter().zip(row).map(|(&k, &v)| cost(k, v)).sum::<i64>()).min().expect("factor rows are non-empty")).sum()
}

/// Materialises the cartesian product of `factors` as flat slot-indexed
/// substitutions. The factors' slots must partition `0..Σ|slots|`.
pub fn factors_product(factors: &[Factor]) -> Vec<Vec<Id>> {
    let arity: usize = factors.iter().map(|f| f.slots.len()).sum();
    let mut acc: Vec<Vec<Id>> = vec![vec![Id::from(0); arity]];
    for f in factors {
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

/// Rebuilds a factor's row set with `build` mapping each input row to zero or
/// more output rows, then runs decomposition so any independence the
/// transformation exposed is captured. Returns the resulting (possibly split)
/// factors, or `None` when no output rows survive (caller drops the match).
///
/// The caller (expand) is responsible for producing distinct rows — expanding a
/// slot to an enode's children can't collide two rows, since an enode lives in a
/// single e-class — so this skips the sort/dedup `Factor::new` does. (Row order
/// is unobservable: every consumer treats the rows as a set.)
pub fn rebuild_factor(slots: Vec<usize>, src_rows: &[Vec<Id>], min_rows: usize, mut build: impl FnMut(&[Id], &mut Vec<Vec<Id>>)) -> Option<Vec<Factor>> {
    let mut rows: Vec<Vec<Id>> = Vec::new();
    for r in src_rows {
        build(r, &mut rows);
    }
    if rows.is_empty() {
        return None;
    }
    Some(Factor { slots, rows }.decompose(min_rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a row of `Id`s from `u32` values.
    fn ids(vs: &[u32]) -> Vec<Id> {
        vs.iter().map(|&v| Id::from(v as usize)).collect()
    }

    /// Returns each factor as `(slots, rows-as-u32)` for order-independent assertions.
    fn shape(factors: &[Factor]) -> Vec<(Vec<usize>, Vec<Vec<u32>>)> {
        factors.iter().map(|f| (f.slots.clone(), f.rows.iter().map(|r| r.iter().map(|&v| usize::from(v) as u32).collect()).collect())).collect()
    }

    /// `Factor::new` sorts rows, deduplicates them, and rejects an empty set.
    #[test]
    fn new_sorts_dedups_and_rejects_empty() {
        let f = Factor::new(vec![0], vec![ids(&[2]), ids(&[1]), ids(&[2])]).unwrap();
        assert_eq!(shape(&[f]), vec![(vec![0], vec![vec![1], vec![2]])]);
        assert!(Factor::new(vec![0], vec![]).is_none());
    }

    /// `pos_of` finds a slot's column index and reports absence.
    #[test]
    fn pos_of_locates_slots() {
        let f = Factor::new(vec![1, 3, 5], vec![ids(&[0, 0, 0])]).unwrap();
        assert_eq!(f.pos_of(3), Some(1));
        assert_eq!(f.pos_of(2), None);
    }

    /// Small factors (`< DEFAULT_DECOMPOSE_MIN_ROWS`) are kept whole — the scan
    /// cost wouldn't repay itself, and correctness is independent of granularity.
    #[test]
    fn decompose_keeps_small_factor_whole() {
        let rows: Vec<Vec<Id>> = (0..8).map(|a| ids(&[a, a + 100])).collect();
        let out = Factor::new(vec![0, 1], rows).unwrap().decompose(DEFAULT_DECOMPOSE_MIN_ROWS);
        assert_eq!(out.len(), 1);
    }

    /// A genuine product of two independent columns splits into the finest
    /// single-slot factors.
    #[test]
    fn decompose_splits_independent_columns() {
        let rows: Vec<Vec<Id>> = (0..8).flat_map(|a| (0..8).map(move |b| ids(&[a, b]))).collect();
        let out = Factor::new(vec![0, 1], rows).unwrap().decompose(DEFAULT_DECOMPOSE_MIN_ROWS);
        let cols: Vec<Vec<u32>> = (0..8).map(|x| vec![x]).collect();
        assert_eq!(shape(&out), vec![(vec![0], cols.clone()), (vec![1], cols)]);
    }

    /// A relation that's a product of one coupled block and one free column
    /// splits into exactly those two blocks.
    #[test]
    fn decompose_separates_coupled_block_from_free_column() {
        // slot0 == slot1 (a coupled diagonal), slot2 free over 0..8.
        let rows: Vec<Vec<Id>> = (0..8).flat_map(|a| (0..8).map(move |b| ids(&[a, a, b]))).collect();
        let out = Factor::new(vec![0, 1, 2], rows).unwrap().decompose(DEFAULT_DECOMPOSE_MIN_ROWS);
        let diag: Vec<Vec<u32>> = (0..8).map(|a| vec![a, a]).collect();
        let free: Vec<Vec<u32>> = (0..8).map(|b| vec![b]).collect();
        assert_eq!(shape(&out), vec![(vec![0, 1], diag), (vec![2], free)]);
    }

    /// A fully-coupled relation (the diagonal) above the threshold stays whole:
    /// every pair is entangled, so the single block is kept.
    #[test]
    fn decompose_keeps_entangled_relation_whole() {
        let rows: Vec<Vec<Id>> = (0..50).map(|a| ids(&[a, a])).collect();
        let out = Factor::new(vec![0, 1], rows).unwrap().decompose(DEFAULT_DECOMPOSE_MIN_ROWS);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slots, vec![0, 1]);
    }

    /// Higher-order entanglement the pairwise scan can't see: `c = (a+b) mod 8`
    /// makes every *pair* of columns independent (each takes all 64 combos), yet
    /// the triple is not a product. The exact `∏|proj| == nrows` check rejects
    /// the bogus 3-way split and keeps the factor whole.
    #[test]
    fn decompose_rejects_higher_order_split_via_exact_check() {
        let rows: Vec<Vec<Id>> = (0..8).flat_map(|a| (0..8).map(move |b| ids(&[a, b, (a + b) % 8]))).collect();
        assert_eq!(rows.len(), 64);
        let out = Factor::new(vec![0, 1, 2], rows).unwrap().decompose(DEFAULT_DECOMPOSE_MIN_ROWS);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slots, vec![0, 1, 2]);
    }

    /// `factored_min` is the separable minimum: `Σ over factors of (min row sum)`,
    /// equal to the min total cost over the full product without materialising it.
    #[test]
    fn factored_min_is_separable() {
        let f0 = Factor::new(vec![0], vec![ids(&[3]), ids(&[1]), ids(&[2])]).unwrap();
        let f1 = Factor::new(vec![1], vec![ids(&[20]), ids(&[10])]).unwrap();
        // cost(slot, value) = value; min is 1 (slot 0) + 10 (slot 1).
        assert_eq!(factored_min(&[f0, f1], |_, v| usize::from(v) as i64), 11);
    }

    /// `factors_product` materialises the cartesian product as flat slot-indexed
    /// rows; the factors' slots must partition `0..arity`.
    #[test]
    fn factors_product_materialises_cartesian() {
        let f0 = Factor::new(vec![0], vec![ids(&[5]), ids(&[6])]).unwrap();
        let f1 = Factor::new(vec![1], vec![ids(&[7]), ids(&[8])]).unwrap();
        let mut got: Vec<Vec<u32>> = factors_product(&[f0, f1]).iter().map(|r| r.iter().map(|&v| usize::from(v) as u32).collect()).collect();
        got.sort();
        assert_eq!(got, vec![vec![5, 7], vec![5, 8], vec![6, 7], vec![6, 8]]);
    }

    /// `factors_product` honours slot positions when factors are out of slot
    /// order: a factor over slot 2 fills column 2 regardless of factor order.
    #[test]
    fn factors_product_places_by_slot_not_factor_order() {
        let hi = Factor::new(vec![2], vec![ids(&[9])]).unwrap();
        let lo = Factor::new(vec![0, 1], vec![ids(&[1, 2])]).unwrap();
        assert_eq!(factors_product(&[hi, lo]), vec![ids(&[1, 2, 9])]);
    }

    /// `rebuild_factor` maps each source row to zero+ output rows, then
    /// re-decomposes; here each row fans out to two, staying one factor.
    #[test]
    fn rebuild_factor_expands_rows() {
        let src = vec![ids(&[1]), ids(&[2])];
        let out = rebuild_factor(vec![0], &src, DEFAULT_DECOMPOSE_MIN_ROWS, |row, rows| {
            rows.push(row.to_vec());
            rows.push(vec![Id::from(usize::from(row[0]) + 10)]);
        })
        .unwrap();
        // rebuild_factor preserves insertion order (no sort/dedup): each src row
        // emits itself then its +10 variant, in turn.
        assert_eq!(shape(&out), vec![(vec![0], vec![vec![1], vec![11], vec![2], vec![12]])]);
    }

    /// `rebuild_factor` returns `None` when the transform yields no rows, so the
    /// caller drops the match.
    #[test]
    fn rebuild_factor_drops_when_empty() {
        let src = vec![ids(&[1]), ids(&[2])];
        assert!(rebuild_factor(vec![0], &src, DEFAULT_DECOMPOSE_MIN_ROWS, |_, _| {}).is_none());
    }
}
