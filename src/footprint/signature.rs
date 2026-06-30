use super::{heap_permute, id_u32};
use crate::factor::Factor;
use crate::hashing::{h64, h128};
use crate::matching::MatchAtEClass;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

/// Max within-marginal-group permutations brute-forced per factor when
/// canonicalising its column order (pass 2). If this triggers its
/// because a bunch of variables have the same marginal.
/// In this case we fall back to the plain marginal-sorted order, and maybe
/// create a false under-merge, which doesn't impact soundness.
const TIE_PERM_CAP: usize = 720;

// Domain-separation salts
const SALT_COLUMN: u64 = 0xC01C;
const SALT_MARGINAL: u64 = 0x0C01_1EC7;
const SALT_FACTOR: u64 = 0xFAC2;
const SALT_ROOT: u64 = 0x2007;
const SALT_ROW_LIST: u64 = 0x0072_6F77;

/// The permutation-invariant footprint of a candidate's match set.
#[cfg(test)]
pub(super) struct Footprint {
    pub(super) sig: u128,
    pub(super) marginals: Vec<u64>,
    pub(super) capped: bool,
}

/// Footprint of raw match data over `arity` slots. Test-only entry point that
/// exercises [`compute`] without a full e-graph.
#[cfg(test)]
pub(super) fn footprint_of(matches: &[MatchAtEClass], arity: usize) -> Footprint {
    let (sig, marginals, capped) = compute(matches, arity);
    Footprint { sig, marginals, capped }
}

/// Scratch buffers for the pass-2 signature computation, allocated once per
/// [`compute`] call and reused across that candidate's factors so the factor loop
/// doesn't reallocate.
#[derive(Default)]
struct FootprintScratch {
    /// Per-root factor hashes, sorted then folded into the root signature.
    fh: Vec<u64>,
    /// Column indices of a factor, sorted by marginal.
    order: Vec<usize>,
    /// Marginal of each column in `order`.
    col_marginals: Vec<u64>,
    /// Working column permutation for the tie-group brute force.
    perm: Vec<usize>,
    /// Maximal equal-marginal runs `(start, len)` with `len ≥ 2`.
    runs: Vec<(usize, usize)>,
    /// Per-row hash keys for the factor being hashed.
    keys: Vec<u64>,
}

/// Core signature computation. Returns `(signature, per-variable marginals,
/// capped)`, where `capped` flags a tie-group that exceeded [`TIE_PERM_CAP`].
pub(super) fn compute(matches: &[MatchAtEClass], arity: usize) -> (u128, Vec<u64>, bool) {
    // Pass 1: global per-variable marginals.
    let marginals = compute_marginals(matches, arity);

    // Pass 2: hash each factor canonically, combine per root then over roots as
    // sorted multisets. The scratch buffers live for this call, reused across the
    // factor loop.
    let mut scratch = FootprintScratch::default();
    let mut capped = false;
    let mut root_sigs: Vec<u64> = Vec::with_capacity(matches.len());
    for m in matches {
        scratch.fh.clear();
        for f in &m.factors {
            let fh = factor_hash(f, &marginals, &mut scratch, &mut capped);
            scratch.fh.push(fh);
        }
        scratch.fh.sort_unstable();
        root_sigs.push(h64(SALT_ROOT, &(id_u32(m.root_eclass), &scratch.fh)));
    }
    root_sigs.sort_unstable();
    (h128(&root_sigs), marginals, capped)
}

/// Pass 1: global per-variable marginals. marginal(v) is a fingerprint of v's
/// *marginal* value distribution: all pairs (r, sigma(v)) with every other
/// variable projected out, read column-by-column off the stored factors.
/// Corresponding variables across a single global permutation get equal marginals.
///
/// Note: this depends on the other variables to some degree still, but only
/// in the way that they modify the factor structure and the number of ocurrences
/// of the given variable (inflated by other variables' presence in a factor).
pub(super) fn compute_marginals(matches: &[MatchAtEClass], arity: usize) -> Vec<u64> {
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
    buckets
        .iter_mut()
        .map(|b| {
            b.sort_unstable();
            h64(SALT_MARGINAL, b)
        })
        .collect()
}

/// Hash of column `ci` of `f` at `root` as a root-anchored value multiset. For a
/// single-slot factor the rows are already the sorted, deduped column, so we hash
/// them directly; otherwise we extract, sort, and run-length-hash the column.
fn column_hash(f: &Factor, ci: usize, root: u32, col: &mut Vec<u32>) -> u64 {
    if f.slots.len() == 1 {
        return h64(SALT_COLUMN, &(root, &f.rows));
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
/// within equal-marginal groups. Columns are ordered by marginal; equal marginals form
/// tie-groups whose within-group permutations are brute-forced to the canonical
/// (smallest) sorted row-hash list.
fn factor_hash(f: &Factor, marginals: &[u64], scratch: &mut FootprintScratch, capped: &mut bool) -> u64 {
    let n = f.slots.len();
    scratch.order.clear();
    scratch.order.extend(0..n);
    scratch.order.sort_by_key(|&p| marginals[f.slots[p]]);
    scratch.col_marginals.clear();
    scratch.col_marginals.extend(scratch.order.iter().map(|&p| marginals[f.slots[p]]));

    let mut h = FxHasher::default();
    SALT_FACTOR.hash(&mut h);
    scratch.col_marginals.hash(&mut h);

    // Maximal equal-marginal runs of length ≥ 2 are the tie-groups to permute.
    scratch.runs.clear();
    let mut i = 0;
    while i < scratch.col_marginals.len() {
        let mut j = i + 1;
        while j < scratch.col_marginals.len() && scratch.col_marginals[j] == scratch.col_marginals[i] {
            j += 1;
        }
        if j - i >= 2 {
            scratch.runs.push((i, j - i));
        }
        i = j;
    }

    if scratch.runs.is_empty() {
        // No ties: the marginal-sorted order is the single canonical one.
        row_hashes(&mut scratch.keys, f, &scratch.order);
        scratch.keys.hash(&mut h);
        return h.finish();
    }

    // Tie-group(s): canonical = the smallest sorted row-hash list over within-
    // group permutations. Enumerated in place (Heap's algorithm into `perm`), so
    // no per-permutation allocation. Bail to the plain order past the cap.
    let mut best: Option<u64> = None;
    if tie_perms_within_cap(&scratch.runs) {
        scratch.perm.clear();
        scratch.perm.extend_from_slice(&scratch.order);
        permute_runs(&mut scratch.perm, &scratch.runs, 0, f, &mut scratch.keys, &mut best);
    } else {
        *capped = true;
        row_hashes(&mut scratch.keys, f, &scratch.order);
        best = Some(hash_list(&scratch.keys));
    }
    best.expect("≥1 ordering").hash(&mut h);
    h.finish()
}

/// Whether the within-tie-group column permutations number at most [`TIE_PERM_CAP`].
/// The count is `∏ len!` over `runs`; we accumulate it incrementally (multiplying
/// in each factor `2..=len`) and bail the instant it exceeds the cap, so a large
/// run can never overflow `usize` — the cap is enforced before the product grows.
fn tie_perms_within_cap(runs: &[(usize, usize)]) -> bool {
    let mut total = 1usize;
    for &(_, len) in runs {
        for k in 2..=len {
            total *= k;
            if total > TIE_PERM_CAP {
                return false;
            }
        }
    }
    true
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
    h64(SALT_ROW_LIST, keys)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::test_helpers::match_at;

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
}
