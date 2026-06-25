//! Permutation-invariant *match-footprint* deduplication for best-first search.
//!
//! Two candidate patterns are "the same footprint" when their match data —
//! `{ root e-class ↦ set of substitutions }` — coincides modulo (1) one global
//! variable-column permutation, (2) per-root substitution reordering, and
//! (3) factor order. Root e-class ids and bound value ids are literal anchors
//! (the e-graph isn't unioned during search), so only those symmetries are
//! quotiented out. Such candidates yield identical compression and reach the
//! same refined footprints, so keeping one is sound — the same reasoning the
//! structural seen-set ([`crate::search::SeenTracker`]) relies on.
//!
//! Example made equal: `(+ ?#0 (* ?#1 2))` and `(+ (* ?#0 2) ?#1)` over a
//! commutative e-graph match the same roots and capture the same argument
//! multiset with the two variables' roles swapped (a single global permutation).
//!
//! The signature is computed straight off the *stored* factored match set (never
//! the materialised cartesian product) in two passes: (1) give each variable a
//! global *colour* from its root-anchored value projection so corresponding
//! variables across permuted candidates get equal colours; (2) hash each factor
//! as a unit in a column-permutation-canonical way (so within-root cross-column
//! correlation is preserved — what a per-column scheme would lose) and combine
//! over factors/roots as sorted multisets into a 128-bit signature.
//!
//! We deliberately do *not* re-decompose factors to a canonical finest form: the
//! search already decomposes consistently, so footprint-equal candidates almost
//! always share a factorisation. Skipping it only ever *misses* a merge (lower
//! recall), never causes a false merge (a different match set still hashes
//! differently), and avoids a per-factor clone + `O(slots²·rows)` decompose on
//! the hot path. Buffers are reused across candidates via [`FootprintScratch`].

use crate::factor::Factor;
use crate::lang::{LanguageFamily, StitchOp};
use crate::matching::MatchAtEClass;
use crate::search::SearchState;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Max within-colour-group permutations brute-forced per factor when
/// canonicalising its column order (pass 2). Tie-groups this large only arise
/// under wide genuine symmetry (many interchangeable args binding identical
/// value sets); beyond the cap we fall back to a plain colour-sorted order for
/// that factor — which can over-merge it — and surface the count as `capped`.
const TIE_PERM_CAP: usize = 720; // 6!

/// Deterministic salted 64-bit hash. FxHasher is stable across runs (unlike the
/// std default's randomised keys), which the dedup needs to be reproducible.
fn h64<T: Hash>(salt: u64, val: &T) -> u64 {
    let mut h = FxHasher::default();
    salt.hash(&mut h);
    val.hash(&mut h);
    h.finish()
}

/// 128-bit hash: two independently-salted 64-bit hashes concatenated. Negligible
/// accidental-collision probability over millions of candidates.
fn h128<T: Hash>(val: &T) -> u128 {
    ((h64(0x9E37_79B9_7F4A_7C15, val) as u128) << 64) | h64(0x85EB_CA6B_C2B2_AE35, val) as u128
}

/// `Id` as a `u32` (egg ids are `u32`-backed), for hashing.
fn id_u32(id: egg::Id) -> u32 {
    usize::from(id) as u32
}

/// Reusable buffers for [`compute`], owned by [`FootprintTracker`] so the hot
/// path allocates nothing per candidate.
#[derive(Default)]
pub struct FootprintScratch {
    /// Per-variable colour contributions (one bucket per slot), then the colours.
    buckets: Vec<Vec<u64>>,
    pub colors: Vec<u64>,
    /// Scratch column / per-factor / per-root hash buffers.
    col: Vec<u32>,
    fh: Vec<u64>,
    root_sigs: Vec<u64>,
    /// Scratch for canonical column ordering.
    order: Vec<usize>,
    col_colors: Vec<u64>,
    /// Packed row keys for the canonical-matrix hash (see [`factor_hash`]).
    keys: Vec<u128>,
}

/// The permutation-invariant footprint of a candidate's match set.
pub struct Footprint {
    pub sig: u128,
    pub colors: Vec<u64>,
    pub capped: bool,
}

/// Computes the footprint of a search state (allocates its own scratch; used by
/// tests). The hot path goes through [`FootprintTracker::check_state`] instead.
pub fn footprint<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>) -> Footprint {
    footprint_of(&state.matches, state.pattern.vars.len())
}

/// Footprint of raw match data over `arity` slots. Split out for unit testing
/// without a full e-graph.
fn footprint_of(matches: &[MatchAtEClass], arity: usize) -> Footprint {
    let mut s = FootprintScratch::default();
    let (sig, capped) = compute(matches, arity, &mut s);
    Footprint { sig, colors: s.colors, capped }
}

/// Core signature computation, writing per-variable colours into `s.colors`.
/// Returns `(signature, capped)` where `capped` flags a tie-group that exceeded
/// [`TIE_PERM_CAP`].
fn compute(matches: &[MatchAtEClass], arity: usize, s: &mut FootprintScratch) -> (u128, bool) {
    // Pass 1: global per-variable colours. colour(v) fingerprints v's
    // root-anchored value distribution, read column-by-column off the stored
    // factors (no cartesian product, no HashMap).
    s.buckets.resize_with(arity, Vec::new);
    for b in s.buckets.iter_mut() {
        b.clear();
    }
    for m in matches {
        let root = id_u32(m.root_eclass);
        for f in &m.factors {
            for (ci, &slot) in f.slots.iter().enumerate() {
                s.buckets[slot].push(column_hash(f, ci, root, &mut s.col));
            }
        }
    }
    s.colors.clear();
    for b in s.buckets.iter_mut() {
        b.sort_unstable();
        s.colors.push(h64(0x0C01_1EC7, b));
    }

    // Pass 2: hash each factor canonically, combine per root then over roots as
    // sorted multisets.
    let mut capped = false;
    s.root_sigs.clear();
    for m in matches {
        s.fh.clear();
        for f in &m.factors {
            s.fh.push(factor_hash(f, &s.colors, &mut s.order, &mut s.col_colors, &mut s.keys, &mut capped));
        }
        s.fh.sort_unstable();
        s.root_sigs.push(h64(0x2007, &(id_u32(m.root_eclass), &s.fh)));
    }
    s.root_sigs.sort_unstable();
    (h128(&s.root_sigs), capped)
}

/// Hash of column `ci` of `f` at `root` as a root-anchored value multiset. For a
/// single-slot factor the rows are already the sorted, deduped column, so we hash
/// them directly; otherwise we extract, sort, and run-length-hash the column.
fn column_hash(f: &Factor, ci: usize, root: u32, col: &mut Vec<u32>) -> u64 {
    if f.slots.len() == 1 {
        return h64(0xC01C, &(root, &f.rows));
    }
    col.clear();
    col.extend(f.rows.iter().map(|r| id_u32(r[ci])));
    col.sort_unstable();
    let mut h = FxHasher::default();
    root.hash(&mut h);
    let mut i = 0;
    while i < col.len() {
        let v = col[i];
        let mut c = 1usize;
        while i + c < col.len() && col[i + c] == v {
            c += 1;
        }
        v.hash(&mut h);
        c.hash(&mut h);
        i += c;
    }
    h.finish()
}

/// Canonical 64-bit hash of one factor (pass 2): invariant to permuting columns
/// within equal-colour groups. Columns are ordered by colour; equal colours form
/// tie-groups whose within-group permutations are brute-forced to the canonical
/// (smallest) sorted row-hash list. Sets `*capped` and falls back to the plain
/// colour order if a tie-group exceeds [`TIE_PERM_CAP`].
///
/// The canonical matrix is encoded by hashing each reordered row to a `u128`,
/// sorting those, and hashing the list — order-invariant over rows and far
/// cheaper than allocating and lex-sorting a `Vec<Vec<u32>>` (the sort is over
/// scalars). 128-bit row hashes make a row-level collision (which would merge two
/// distinct matrices) negligible. Works for any column count.
fn factor_hash(f: &Factor, colors: &[u64], order: &mut Vec<usize>, col_colors: &mut Vec<u64>, keys: &mut Vec<u128>, capped: &mut bool) -> u64 {
    let n = f.slots.len();
    order.clear();
    order.extend(0..n);
    order.sort_by_key(|&p| colors[f.slots[p]]);
    col_colors.clear();
    col_colors.extend(order.iter().map(|&p| colors[f.slots[p]]));

    let mut h = FxHasher::default();
    0xFAC2u64.hash(&mut h);
    col_colors.hash(&mut h);

    let has_tie = col_colors.windows(2).any(|w| w[0] == w[1]);
    if !has_tie {
        // Single canonical column order; hash + sort its rows.
        row_hashes(keys, f, order);
        keys.hash(&mut h);
    } else {
        // Tie-group(s): pick the permutation with the smallest sorted row-hash list.
        let orderings = group_orderings(order, col_colors).unwrap_or_else(|| {
            *capped = true;
            vec![order.to_vec()]
        });
        let mut best: Option<Vec<u128>> = None;
        for p in orderings {
            row_hashes(keys, f, &p);
            if best.as_ref().is_none_or(|b| keys.as_slice() < b.as_slice()) {
                best = Some(keys.clone());
            }
        }
        best.expect("≥1 ordering").hash(&mut h);
    }
    h.finish()
}

/// Hashes each row of `f` (columns projected through `perm`) to a `u128` into
/// `keys`, then sorts — an order-invariant fingerprint of the canonical matrix.
/// The two independently-salted `FxHasher`s mirror [`h128`].
fn row_hashes(keys: &mut Vec<u128>, f: &Factor, perm: &[usize]) {
    keys.clear();
    keys.extend(f.rows.iter().map(|r| {
        let mut h1 = FxHasher::default();
        let mut h2 = FxHasher::default();
        0x9E37_79B9_7F4A_7C15u64.hash(&mut h1);
        0x85EB_CA6B_C2B2_AE35u64.hash(&mut h2);
        for &p in perm {
            let v = id_u32(r[p]);
            v.hash(&mut h1);
            v.hash(&mut h2);
        }
        ((h1.finish() as u128) << 64) | h2.finish() as u128
    }));
    keys.sort_unstable();
}

/// All column orderings reachable by permuting within each equal-colour run of
/// `base` (positions pre-sorted by colour, `col_colors` parallel). Returns `None`
/// if the total `∏ run_len!` exceeds [`TIE_PERM_CAP`].
fn group_orderings(base: &[usize], col_colors: &[u64]) -> Option<Vec<Vec<usize>>> {
    let mut runs: Vec<&[usize]> = Vec::new();
    let mut i = 0;
    while i < base.len() {
        let mut j = i + 1;
        while j < base.len() && col_colors[j] == col_colors[i] {
            j += 1;
        }
        runs.push(&base[i..j]);
        i = j;
    }
    let mut total: usize = 1;
    for r in &runs {
        total = total.checked_mul(factorial(r.len()))?;
        if total > TIE_PERM_CAP {
            return None;
        }
    }
    let mut result: Vec<Vec<usize>> = vec![Vec::with_capacity(base.len())];
    for r in &runs {
        let rp = all_perms(r);
        let mut next = Vec::with_capacity(result.len() * rp.len());
        for prefix in &result {
            for p in &rp {
                let mut v = prefix.clone();
                v.extend_from_slice(p);
                next.push(v);
            }
        }
        result = next;
    }
    Some(result)
}

/// `n!` (callers cap first, so no overflow concern in practice).
fn factorial(n: usize) -> usize {
    (1..=n).product::<usize>().max(1)
}

/// Every permutation of `items` (recursive; only ever called on tiny slices).
fn all_perms(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in all_perms(&rest) {
            p.insert(0, x);
            out.push(p);
        }
    }
    out
}

/// Tracks already-seen match footprints to dedupe successors, mirroring
/// [`crate::search::SeenTracker`] but keyed on the permutation-invariant
/// signature instead of pattern structure. The stored value is the most-flexible
/// frozen set ever seen for a signature, as a *colour multiset* (compared up to
/// the same variable permutation the signature quotients out); a repeat is a hit
/// when a recorded visit's frozen colours are a sub-multiset of the new ones — it
/// was at least as flexible, so this visit's successors are already reachable.
#[derive(Default)]
pub struct FootprintTracker {
    map: FxHashMap<u128, Vec<u64>>,
    scratch: FootprintScratch,
    pub hits: usize,
    pub capped: usize,
    pub time: Duration,
}

impl FootprintTracker {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Computes `state`'s footprint (reusing scratch) and records/dedupes it at
    /// `frozen`. Returns `true` (skip) on a dominated repeat.
    pub fn check_state<F: LanguageFamily, O: StitchOp>(&mut self, state: &SearchState<F, O>, frozen: &[bool]) -> bool {
        let t = Instant::now();
        let (sig, capped) = compute(&state.matches, state.pattern.vars.len(), &mut self.scratch);
        if capped {
            self.capped += 1;
        }
        // Disjoint field borrows: `scratch.colors` (read) and `map` (mutated).
        let colors = &self.scratch.colors;
        let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| colors[v]).collect();
        fc.sort_unstable();
        let skip = match self.map.get(&sig) {
            Some(existing) if submultiset(existing, &fc) => true,
            _ => {
                self.map.insert(sig, fc);
                false
            }
        };
        self.time += t.elapsed();
        if skip {
            self.hits += 1;
        }
        skip
    }

    /// Records a precomputed footprint (used by tests).
    pub fn check_and_insert(&mut self, fp: &Footprint, frozen: &[bool]) -> bool {
        let skip = self.record(fp.sig, fp.capped, frozen, &fp.colors);
        if skip {
            self.hits += 1;
        }
        skip
    }

    /// Shared record/dedup step: builds the frozen-colour multiset and applies
    /// the sub-multiset subsumption rule.
    fn record(&mut self, sig: u128, capped: bool, frozen: &[bool], colors: &[u64]) -> bool {
        if capped {
            self.capped += 1;
        }
        let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| colors[v]).collect();
        fc.sort_unstable();
        match self.map.get(&sig) {
            Some(existing) if submultiset(existing, &fc) => true,
            _ => {
                self.map.insert(sig, fc);
                false
            }
        }
    }
}

/// True iff sorted `a` is a sub-multiset of sorted `b` (the colour-space analogue
/// of `frozen_subset`: a smaller frozen set is at least as flexible).
fn submultiset(a: &[u64], b: &[u64]) -> bool {
    let mut j = 0;
    for &x in a {
        while j < b.len() && b[j] < x {
            j += 1;
        }
        if j == b.len() || b[j] != x {
            return false;
        }
        j += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egg::Id;

    fn id(n: usize) -> Id {
        Id::from(n)
    }

    /// One match at `root` whose substitution set is the given rows over slots
    /// `0..arity`, as a single factor.
    fn match_at(root: usize, arity: usize, rows: Vec<Vec<usize>>) -> MatchAtEClass {
        let slots: Vec<usize> = (0..arity).collect();
        let rows: Vec<Vec<Id>> = rows.into_iter().map(|r| r.into_iter().map(id).collect()).collect();
        MatchAtEClass {
            root_eclass: id(root),
            factors: vec![Factor::new(slots, rows).unwrap()],
        }
    }

    /// `(+ ?#0 (* ?#1 2))` and `(+ (* ?#0 2) ?#1)` over a commutative e-graph
    /// match the same root binding the same args with the roles swapped — a
    /// single global permutation — so footprints must coincide.
    #[test]
    fn commutative_swap_is_equal() {
        let p1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11]])], 2);
        let p2 = footprint_of(&[match_at(100, 2, vec![vec![11, 10]])], 2);
        assert_eq!(p1.sig, p2.sig);
    }

    /// Same per-column marginals but a different *joint* pairing must NOT merge.
    #[test]
    fn different_joint_is_distinct() {
        let q1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11], vec![12, 13]])], 2);
        let q2 = footprint_of(&[match_at(100, 2, vec![vec![10, 13], vec![12, 11]])], 2);
        assert_ne!(q1.sig, q2.sig);
    }

    /// A genuinely symmetric commutative match `{(a,b),(b,a)}` canonicalises
    /// regardless of which subst comes first (tie-group brute force).
    #[test]
    fn symmetric_tie_canonicalises() {
        let a = footprint_of(&[match_at(100, 2, vec![vec![10, 11], vec![11, 10]])], 2);
        let b = footprint_of(&[match_at(100, 2, vec![vec![11, 10], vec![10, 11]])], 2);
        assert_eq!(a.sig, b.sig);
    }

    /// Different roots ⇒ different footprint (root ids are literal anchors).
    #[test]
    fn root_is_anchored() {
        let a = footprint_of(&[match_at(100, 2, vec![vec![10, 11]])], 2);
        let b = footprint_of(&[match_at(200, 2, vec![vec![10, 11]])], 2);
        assert_ne!(a.sig, b.sig);
    }

    /// Subst order within a root is irrelevant (rows are a set).
    #[test]
    fn subst_order_irrelevant() {
        let a = footprint_of(&[match_at(100, 2, vec![vec![10, 11], vec![12, 13]])], 2);
        let b = footprint_of(&[match_at(100, 2, vec![vec![12, 13], vec![10, 11]])], 2);
        assert_eq!(a.sig, b.sig);
    }

    /// Frozen-colour subsumption: freezing the *other* of two swapped variables
    /// is the same frozen set up to the permutation, so the second visit is a hit.
    #[test]
    fn frozen_subsumption_up_to_permutation() {
        let mut t = FootprintTracker::new();
        let p1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11]])], 2);
        let p2 = footprint_of(&[match_at(100, 2, vec![vec![11, 10]])], 2);
        assert!(!t.check_and_insert(&p1, &[true, false]));
        assert!(t.check_and_insert(&p2, &[false, true]));
    }
}
