//! Permutation-invariant *match-footprint* deduplication for best-first search.
//!
//! Two candidate patterns are "the same footprint" when their match data —
//! `{ root e-class ↦ set of substitutions }` — coincides modulo (1) one global
//! variable-column permutation, (2) per-root substitution reordering, and
//! (3) factorisation grouping. Root e-class ids and bound value ids are literal
//! anchors (the e-graph isn't unioned during search), so only those three
//! symmetries are quotiented out. Such candidates yield identical compression
//! and reach the same refined footprints, so keeping one is sound — the same
//! reasoning the structural seen-set ([`crate::search::SeenTracker`]) relies on.
//!
//! Example made equal: `(+ ?#0 (* ?#1 2))` and `(+ (* ?#0 2) ?#1)` over a
//! commutative e-graph match the same roots and capture the same argument
//! multiset with the two variables' roles swapped (a single global permutation).
//!
//! The signature is computed straight off the factored match set (never the
//! materialised cartesian product) in three steps: (0) refactor each match to
//! its unique finest cartesian form; (1) give each variable a global *colour*
//! from its root-anchored value projection so corresponding variables across
//! permuted candidates get equal colours; (2) hash each fine factor as a unit
//! in a column-permutation-canonical way (so within-root cross-column
//! correlation is preserved — what a per-column scheme would lose); (3) combine
//! over roots as a sorted multiset. A 128-bit hash makes accidental collisions
//! negligible, so we never store or compare the (large) match sets themselves.

use crate::factor::Factor;
use crate::lang::{LanguageFamily, StitchOp};
use crate::matching::MatchAtEClass;
use crate::search::SearchState;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Max within-colour-group permutations brute-forced per factor when
/// canonicalising its column order (Step 2). Tie-groups this large only arise
/// under wide genuine symmetry (e.g. many interchangeable args binding identical
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

/// The permutation-invariant footprint of a candidate's match set.
pub struct Footprint {
    /// 128-bit signature; the dedup key.
    pub sig: u128,
    /// Per-variable global colour (Step 1), indexed by slot. Lets the tracker
    /// compare frozen masks up to the same permutation the signature quotients
    /// out: corresponding variables across permuted candidates share a colour.
    pub colors: Vec<u64>,
    /// Whether any factor hit [`TIE_PERM_CAP`] and fell back to a non-canonical
    /// (possibly over-merging) order. Reported for diagnostics.
    pub capped: bool,
}

/// Computes the footprint of a search state.
pub fn footprint<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>) -> Footprint {
    footprint_of(&state.matches, state.pattern.vars.len())
}

/// Footprint of raw match data over `arity` variable slots. Split from
/// [`footprint`] so it can be unit-tested without building a full e-graph.
fn footprint_of(matches: &[MatchAtEClass], arity: usize) -> Footprint {
    // Step 0: refactor every match to its unique finest cartesian form.
    let fine: Vec<Vec<Factor>> = matches.iter().map(|m| m.factors.iter().flat_map(|f| f.clone().decompose(1)).collect()).collect();

    // Step 1: global per-variable colours. colour(v) fingerprints v's
    // root-anchored value distribution, read off its owning fine factor as a
    // within-factor run-length encoding (no cartesian product materialised).
    let colors: Vec<u64> = (0..arity)
        .map(|v| {
            let mut per_root: Vec<(u32, Vec<(u32, u32)>)> = Vec::with_capacity(matches.len());
            for (m, ff) in matches.iter().zip(&fine) {
                let (fi, pos) = locate(ff, v);
                per_root.push((id_u32(m.root_eclass), column_rle(&ff[fi], pos)));
            }
            per_root.sort_unstable();
            h64(0xC011_EC7, &per_root)
        })
        .collect();

    // Steps 2 & 3: hash each fine factor canonically, combine per root as a
    // sorted multiset, then combine over roots as a sorted multiset.
    let mut capped = false;
    let mut root_sigs: Vec<u64> = matches
        .iter()
        .zip(&fine)
        .map(|(m, ff)| {
            let mut fh: Vec<u64> = ff.iter().map(|f| factor_hash(f, &colors, &mut capped)).collect();
            fh.sort_unstable();
            h64(0x2007, &(id_u32(m.root_eclass), fh))
        })
        .collect();
    root_sigs.sort_unstable();

    Footprint { sig: h128(&root_sigs), colors, capped }
}

/// `(factor_index, position)` of `slot` within `factors`. Panics on a broken
/// partition (every slot is covered by exactly one factor).
fn locate(factors: &[Factor], slot: usize) -> (usize, usize) {
    for (fi, f) in factors.iter().enumerate() {
        if let Some(p) = f.pos_of(slot) {
            return (fi, p);
        }
    }
    panic!("slot {slot} not covered by any factor");
}

/// Sorted `(value, count)` run-length encoding of column `pos` of `f`.
fn column_rle(f: &Factor, pos: usize) -> Vec<(u32, u32)> {
    let mut counts: FxHashMap<u32, u32> = FxHashMap::default();
    for row in &f.rows {
        *counts.entry(id_u32(row[pos])).or_default() += 1;
    }
    let mut v: Vec<(u32, u32)> = counts.into_iter().collect();
    v.sort_unstable();
    v
}

/// Canonical 64-bit hash of one fine factor (Step 2): invariant to permuting
/// columns within equal-colour groups. Columns are ordered by colour; equal
/// colours form tie-groups whose within-group permutations are brute-forced to
/// find the lexicographically-smallest sorted-row matrix. Sets `*capped` and
/// falls back to the plain colour order if a tie-group exceeds [`TIE_PERM_CAP`].
fn factor_hash(f: &Factor, colors: &[u64], capped: &mut bool) -> u64 {
    let mut order: Vec<usize> = (0..f.slots.len()).collect();
    order.sort_by_key(|&p| colors[f.slots[p]]);
    let col_colors: Vec<u64> = order.iter().map(|&p| colors[f.slots[p]]).collect();
    let perms = match group_orderings(&order, &col_colors) {
        Some(ps) => ps,
        None => {
            *capped = true;
            vec![order.clone()]
        }
    };
    let best = perms.iter().map(|perm| canonical_matrix(f, perm)).min().expect("≥1 ordering");
    h64(0xFAC2, &(col_colors, best))
}

/// Rows of `f` projected through column order `perm`, then sorted — the canonical
/// matrix for that column order.
fn canonical_matrix(f: &Factor, perm: &[usize]) -> Vec<Vec<u32>> {
    let mut rows: Vec<Vec<u32>> = f.rows.iter().map(|r| perm.iter().map(|&p| id_u32(r[p])).collect()).collect();
    rows.sort_unstable();
    rows
}

/// All column orderings reachable by permuting within each equal-colour run of
/// `base` (positions pre-sorted by colour, `col_colors` parallel). Returns
/// `None` if the total `∏ run_len!` exceeds [`TIE_PERM_CAP`].
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

/// `n!`, saturating to a large value is unnecessary since callers cap first.
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

/// `Id` as a `u32` (egg ids are `u32`-backed), for hashing.
fn id_u32(id: egg::Id) -> u32 {
    usize::from(id) as u32
}

/// Tracks already-seen match footprints to dedupe successors, mirroring
/// [`crate::search::SeenTracker`] but keyed on the permutation-invariant
/// signature instead of pattern structure. The stored value is the most-flexible
/// frozen set ever seen for a signature, expressed as a *colour multiset* (so it
/// is compared up to the same variable permutation the signature quotients out);
/// a repeat is a hit when a recorded visit's frozen colours are a sub-multiset of
/// the new ones — that visit was at least as flexible, so all of this visit's
/// successors are already reachable. Wrap in `Option<…>`; `None` disables it.
#[derive(Default)]
pub struct FootprintTracker {
    map: FxHashMap<u128, Vec<u64>>,
    pub hits: usize,
    pub capped: usize,
    pub time: Duration,
}

impl FootprintTracker {
    pub fn new() -> Self {
        Self::default()
    }
    /// Number of distinct footprints recorded.
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// Records `fp` at its `frozen` mask if this is the first or a not-dominated
    /// visit; returns `true` (skip) if a recorded visit's frozen-colour multiset
    /// is a sub-multiset of this one (it was at least as flexible).
    pub fn check_and_insert(&mut self, fp: &Footprint, frozen: &[bool]) -> bool {
        let t = Instant::now();
        if fp.capped {
            self.capped += 1;
        }
        let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| fp.colors[v]).collect();
        fc.sort_unstable();
        let skip = match self.map.get(&fp.sig) {
            Some(existing) if submultiset(existing, &fc) => true,
            _ => {
                self.map.insert(fp.sig, fc);
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
    /// `0..arity`, as a single (pre-decomposition) factor.
    fn match_at(root: usize, arity: usize, rows: Vec<Vec<usize>>) -> MatchAtEClass {
        let slots: Vec<usize> = (0..arity).collect();
        let rows: Vec<Vec<Id>> = rows.into_iter().map(|r| r.into_iter().map(id).collect()).collect();
        MatchAtEClass {
            root_eclass: id(root),
            factors: vec![Factor::new(slots, rows).unwrap()],
        }
    }

    /// `(+ ?#0 (* ?#1 2))` and `(+ (* ?#0 2) ?#1)` over a commutative e-graph
    /// match the same root binding the same args with the two roles swapped —
    /// a single global variable permutation — so footprints must coincide.
    #[test]
    fn commutative_swap_is_equal() {
        // root 100: { #0=a(10), #1=b(11) }  vs  { #0=b(11), #1=a(10) }
        let p1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11]])], 2);
        let p2 = footprint_of(&[match_at(100, 2, vec![vec![11, 10]])], 2);
        assert_eq!(p1.sig, p2.sig);
    }

    /// Same per-column marginals but a different *joint* pairing must NOT merge:
    /// `{(a,b),(c,d)}` vs `{(a,d),(c,b)}` are genuinely different abstractions.
    #[test]
    fn different_joint_is_distinct() {
        let q1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11], vec![12, 13]])], 2);
        let q2 = footprint_of(&[match_at(100, 2, vec![vec![10, 13], vec![12, 11]])], 2);
        assert_ne!(q1.sig, q2.sig);
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
        // First visit: #0 frozen in p1 (colour of value 10's column).
        assert!(!t.check_and_insert(&p1, &[true, false]));
        // Second visit of the swapped pattern with #1 frozen — same colour set.
        assert!(t.check_and_insert(&p2, &[false, true]));
    }
}
