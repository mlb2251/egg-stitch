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
//! global *marginal* from its root-anchored value projection so corresponding
//! variables across permuted candidates get equal marginals; (2) hash each factor
//! as a unit in a column-permutation-canonical way (so within-root cross-column
//! correlation is preserved — what a per-column scheme would lose) and combine
//! over factors/roots as sorted multisets into a 128-bit signature.
//!
//! We deliberately do *not* re-decompose factors to a canonical finest form: the
//! search already decomposes consistently, so footprint-equal candidates almost
//! always share a factorisation. Skipping it only ever *misses* a merge (lower
//! recall), never causes a false merge (a different match set still hashes
//! differently), and avoids a per-factor clone + `O(slots²·rows)` decompose on
//! the hot path. Buffers are reused across candidates via a shared `FootprintScratch`.
//!
//! The module is split into [`signature`] (the two-pass signature `compute`),
//! [`equivalence`] (the exact slow-path oracle validating signature hits), and
//! [`tracker`] (the stateful [`FootprintTracker`] that dedupes successors).

mod equivalence;
mod signature;
mod tracker;

pub use tracker::FootprintTracker;

/// `Id` as a `u32` (egg ids are `u32`-backed), for hashing.
fn id_u32(id: egg::Id) -> u32 {
    usize::from(id) as u32
}

/// `n!` (callers cap first, so no overflow concern in practice).
fn factorial(n: usize) -> usize {
    (1..=n).product::<usize>().max(1)
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

#[cfg(test)]
mod test_helpers {
    use crate::factor::Factor;
    use crate::matching::MatchAtEClass;
    use egg::Id;

    fn id(n: usize) -> Id {
        Id::from(n)
    }

    /// One match at `root` whose substitution set is the given rows over slots
    /// `0..arity`, as a single factor.
    pub fn match_at(root: usize, arity: usize, rows: Vec<Vec<usize>>) -> MatchAtEClass {
        let slots: Vec<usize> = (0..arity).collect();
        let rows: Vec<Vec<Id>> = rows.into_iter().map(|r| r.into_iter().map(id).collect()).collect();
        MatchAtEClass {
            root_eclass: id(root),
            factors: vec![Factor::new(slots, rows).unwrap()],
        }
    }
}
