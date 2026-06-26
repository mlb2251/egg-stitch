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
use std::collections::hash_map::Entry;
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

/// The permutation-invariant footprint of a candidate's match set.
pub struct Footprint {
    pub sig: u128,
    pub colors: Vec<u64>,
    pub capped: bool,
}

/// Computes the footprint of a search state.
pub fn footprint<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>) -> Footprint {
    footprint_of(&state.matches, state.pattern.vars.len())
}

/// Footprint of raw match data over `arity` slots. Split out for unit testing
/// without a full e-graph.
fn footprint_of(matches: &[MatchAtEClass], arity: usize) -> Footprint {
    let (sig, colors, capped) = compute(matches, arity);
    Footprint { sig, colors, capped }
}

/// Core signature computation. Returns `(signature, per-variable colours,
/// capped)`, where `capped` flags a tie-group that exceeded [`TIE_PERM_CAP`].
/// Buffers are reused across the inner factor loops but allocated per call —
/// they're small and same-sized, so the allocator serves them essentially free.
fn compute(matches: &[MatchAtEClass], arity: usize) -> (u128, Vec<u64>, bool) {
    // Pass 1: global per-variable colours. colour(v) fingerprints v's
    // root-anchored value distribution, read column-by-column off the stored
    // factors (no cartesian product, no HashMap).
    let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); arity];
    let mut col: Vec<u32> = Vec::new();
    for m in matches {
        let root = id_u32(m.root_eclass);
        for f in &m.factors {
            for (ci, &slot) in f.slots.iter().enumerate() {
                buckets[slot].push(column_hash(f, ci, root, &mut col));
            }
        }
    }
    let colors: Vec<u64> = buckets
        .iter_mut()
        .map(|b| {
            b.sort_unstable();
            h64(0x0C01_1EC7, b)
        })
        .collect();

    // Pass 2: hash each factor canonically, combine per root then over roots as
    // sorted multisets.
    let mut capped = false;
    let (mut fh, mut order, mut col_colors, mut perm, mut runs, mut keys) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut root_sigs: Vec<u64> = Vec::with_capacity(matches.len());
    for m in matches {
        fh.clear();
        for f in &m.factors {
            fh.push(factor_hash(f, &colors, &mut order, &mut col_colors, &mut perm, &mut runs, &mut keys, &mut capped));
        }
        fh.sort_unstable();
        root_sigs.push(h64(0x2007, &(id_u32(m.root_eclass), &fh)));
    }
    root_sigs.sort_unstable();
    (h128(&root_sigs), colors, capped)
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
/// The canonical matrix is encoded by hashing each reordered row to a `u64`,
/// sorting those, and hashing the list — order-invariant over rows and far
/// cheaper than allocating and lex-sorting a `Vec<Vec<u32>>` (the sort is over
/// scalars). Works for any column count.
#[allow(clippy::too_many_arguments)]
fn factor_hash(f: &Factor, colors: &[u64], order: &mut Vec<usize>, col_colors: &mut Vec<u64>, perm: &mut Vec<usize>, runs: &mut Vec<(usize, usize)>, keys: &mut Vec<u64>, capped: &mut bool) -> u64 {
    let n = f.slots.len();
    order.clear();
    order.extend(0..n);
    order.sort_by_key(|&p| colors[f.slots[p]]);
    col_colors.clear();
    col_colors.extend(order.iter().map(|&p| colors[f.slots[p]]));

    let mut h = FxHasher::default();
    0xFAC2u64.hash(&mut h);
    col_colors.hash(&mut h);

    // Maximal equal-colour runs of length ≥ 2 are the tie-groups to permute.
    runs.clear();
    let mut i = 0;
    while i < col_colors.len() {
        let mut j = i + 1;
        while j < col_colors.len() && col_colors[j] == col_colors[i] {
            j += 1;
        }
        if j - i >= 2 {
            runs.push((i, j - i));
        }
        i = j;
    }

    if runs.is_empty() {
        // No ties: the colour-sorted order is the single canonical one.
        row_hashes(keys, f, order);
        keys.hash(&mut h);
        return h.finish();
    }

    // Tie-group(s): canonical = the smallest sorted row-hash list over within-
    // group permutations. Enumerated in place (Heap's algorithm into `perm`), so
    // no per-permutation allocation. Bail to the plain order past the cap.
    let total = runs.iter().try_fold(1usize, |acc, &(_, l)| acc.checked_mul(factorial(l)));
    let mut best: Option<u64> = None;
    if total.is_some_and(|t| t <= TIE_PERM_CAP) {
        perm.clear();
        perm.extend_from_slice(order);
        permute_runs(perm, runs, 0, f, keys, &mut best);
    } else {
        *capped = true;
        row_hashes(keys, f, order);
        best = Some(hash_list(keys));
    }
    best.expect("≥1 ordering").hash(&mut h);
    h.finish()
}

/// Enumerates every column ordering reachable by permuting within each tie-run of
/// `perm` (Heap's algorithm, in place), tracking the smallest per-ordering
/// row-list hash in `best`. Fully reuses `keys` — no per-permutation allocation.
/// (The min over orderings of `hash(sorted row-keys)` is itself permutation-
/// canonical, so a scalar min suffices — no need to retain the winning list.)
fn permute_runs(perm: &mut Vec<usize>, runs: &[(usize, usize)], ri: usize, f: &Factor, keys: &mut Vec<u64>, best: &mut Option<u64>) {
    let Some(&(start, len)) = runs.get(ri) else {
        // All runs placed: evaluate this full ordering.
        row_hashes(keys, f, perm);
        let hk = hash_list(keys);
        if best.is_none_or(|b| hk < b) {
            *best = Some(hk);
        }
        return;
    };
    heap_permute(perm, start, len, &mut |perm| permute_runs(perm, runs, ri + 1, f, keys, best));
}

/// Order-sensitive 64-bit hash of a sorted key list.
fn hash_list(keys: &[u64]) -> u64 {
    let mut h = FxHasher::default();
    keys.hash(&mut h);
    h.finish()
}

/// Heap's algorithm over `perm[start..start+len]`, calling `emit` once per
/// permutation (the rest of `perm` is left untouched).
fn heap_permute(perm: &mut Vec<usize>, start: usize, len: usize, emit: &mut dyn FnMut(&mut Vec<usize>)) {
    if len <= 1 {
        emit(perm);
        return;
    }
    for i in 0..len {
        heap_permute(perm, start, len - 1, emit);
        let swap = if len.is_multiple_of(2) { start + i } else { start };
        perm.swap(swap, start + len - 1);
    }
}

/// Hashes each row of `f` (columns projected through `perm`) to a `u64` into
/// `keys`, then sorts — an order-invariant fingerprint of the canonical matrix.
/// `u64` is ample here: a factor holds at most a few dozen rows, so a row-key
/// collision is negligible (the cross-candidate signature stays 128-bit).
fn row_hashes(keys: &mut Vec<u64>, f: &Factor, perm: &[usize]) {
    keys.clear();
    keys.extend(f.rows.iter().map(|r| {
        let mut h = FxHasher::default();
        for &p in perm {
            id_u32(r[p]).hash(&mut h);
        }
        h.finish()
    }));
    keys.sort_unstable();
}

/// `n!` (callers cap first, so no overflow concern in practice).
fn factorial(n: usize) -> usize {
    (1..=n).product::<usize>().max(1)
}

/// SplitMix64 finaliser — cheap avalanche for the proxy mixing.
fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A cheap, permutation-invariant *proxy* for a candidate's footprint: a hash of
/// `(arity, num_matches, ⊕ over matches of mix(root | num_substs<<32))`. It
/// depends only on quantities the full signature also quotients invariant (root
/// ids, per-root subst counts, arity, match count) and never on variable identity
/// or order, so **footprint-equal ⇒ equal proxy**. One `mix64` per match, no
/// per-factor/row work — far cheaper than [`compute`].
fn cheap_proxy<F: LanguageFamily, O: StitchOp>(state: &SearchState<F, O>) -> u64 {
    let mut acc: u64 = 0;
    for m in &state.matches {
        // Commutative combine (wrapping add) keeps it order-invariant.
        acc = acc.wrapping_add(mix64(id_u32(m.root_eclass) as u64 | ((m.num_substs() as u64) << 32)));
    }
    h64(0x9001_9001, &(state.pattern.vars.len(), state.matches.len(), acc))
}

/// Tracks already-seen match footprints to dedupe successors, mirroring
/// [`crate::search::SeenTracker`] but keyed on the permutation-invariant
/// signature instead of pattern structure.
///
/// Two-level: a cheap invariant [`cheap_proxy`] gates the expensive 128-bit
/// [`compute`]. The first candidate to claim a proxy is kept without computing
/// its signature at all (a fresh proxy *guarantees* it differs from everything
/// seen). Only on a proxy collision do we compute the real signature and dedup
/// within that proxy's bucket. Each bucket entry is `(sig, frozen colours)`; the
/// frozen colours are the most-flexible set seen for that sig (a *colour
/// multiset*, compared up to the variable permutation the sig quotients out), and
/// a repeat is a hit when a recorded entry's frozen colours are a sub-multiset of
/// the new ones — it was at least as flexible, so this visit's successors are
/// already reachable.
///
/// Tradeoff: the very first true-duplicate within a colliding bucket isn't
/// pruned (the representative has no stored sig), a small recall loss; soundness
/// is unaffected since pruning still requires a full 128-bit sig match.
#[derive(Default)]
pub struct FootprintTracker {
    buckets: FxHashMap<u64, Vec<(u128, Vec<u64>)>>,
    pub hits: usize,
    /// Candidates kept via the unique-proxy fast path (full signatures avoided).
    pub proxy_skips: usize,
    pub capped: usize,
    pub time: Duration,
}

impl FootprintTracker {
    pub fn new() -> Self {
        Self::default()
    }
    /// Number of distinct proxy buckets recorded.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Dedupes `state` at `frozen`. Returns `true` (skip) on a dominated repeat.
    pub fn check_state<F: LanguageFamily, O: StitchOp>(&mut self, state: &SearchState<F, O>, frozen: &[bool]) -> bool {
        let t = Instant::now();
        let proxy = cheap_proxy(state);
        // Fresh proxy: provably unique, so keep without computing a signature.
        if let Entry::Vacant(e) = self.buckets.entry(proxy) {
            e.insert(Vec::new());
            self.proxy_skips += 1;
            self.time += t.elapsed();
            return false;
        }
        // Collision: fall back to the real signature.
        let (sig, colors, capped) = compute(&state.matches, state.pattern.vars.len());
        if capped {
            self.capped += 1;
        }
        let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| colors[v]).collect();
        fc.sort_unstable();
        let skip = dedup_in_bucket(self.buckets.get_mut(&proxy).expect("present"), sig, fc);
        self.time += t.elapsed();
        if skip {
            self.hits += 1;
        }
        skip
    }

    /// Records a precomputed footprint (used by tests; always exercises the
    /// signature path via a single shared bucket).
    pub fn check_and_insert(&mut self, fp: &Footprint, frozen: &[bool]) -> bool {
        if fp.capped {
            self.capped += 1;
        }
        let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| fp.colors[v]).collect();
        fc.sort_unstable();
        let skip = dedup_in_bucket(self.buckets.entry(0).or_default(), fp.sig, fc);
        if skip {
            self.hits += 1;
        }
        skip
    }
}

/// Sub-multiset subsumption within one proxy bucket: skip if an entry with this
/// `sig` has frozen colours that are a sub-multiset of `fc` (was at least as
/// flexible); otherwise record `fc` (overwriting a non-dominated same-sig entry,
/// mirroring the seen-set) and keep.
fn dedup_in_bucket(bucket: &mut Vec<(u128, Vec<u64>)>, sig: u128, fc: Vec<u64>) -> bool {
    match bucket.iter_mut().find(|(s, _)| *s == sig) {
        Some(entry) if submultiset(&entry.1, &fc) => true,
        Some(entry) => {
            entry.1 = fc;
            false
        }
        None => {
            bucket.push((sig, fc));
            false
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
