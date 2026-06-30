//! Exact footprint equivalence, used by check_slow.

use super::signature::compute_colors;
use super::{factorial, heap_permute, id_u32};
use crate::matching::MatchAtEClass;
use rustc_hash::FxHashMap;

/// Checks if two match sets are equivalent under a global column permutation. Can
/// return true even if they are not (when the search space is too large), but never false.
/// Does depend on factor structure, but so does the signature being checked.
pub(super) fn footprint_equivalent(a: &[MatchAtEClass], b: &[MatchAtEClass], arity: usize) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let (ca, cb) = (compute_colors(a, arity), compute_colors(b, arity));
    // A valid π maps each a-column to a b-column of equal colour; mismatched
    // colour multisets ⇒ no such π.
    let Some(groups) = pair_columns_by_color(&ca, &cb) else {
        return false;
    };
    let canon_b = canonicalize(b, &(0..arity).collect::<Vec<_>>());
    let mut perm = vec![0usize; arity];
    enumerate_perms(&groups, 0, &mut perm, &mut |perm| canonicalize(a, perm) == canon_b)
}

/// Pairs the two match sets' columns by colour into `(a-columns, b-columns)` per
/// colour — the colour groups a global colour-preserving permutation must map
/// within. Returns `None` when no such permutation can exist: a colour present on
/// one side but not the other, or a colour whose two groups differ in size.
fn pair_columns_by_color(ca: &[u64], cb: &[u64]) -> Option<Vec<(Vec<usize>, Vec<usize>)>> {
    let mut groups: FxHashMap<u64, (Vec<usize>, Vec<usize>)> = FxHashMap::default();
    for (v, &c) in ca.iter().enumerate() {
        groups.entry(c).or_default().0.push(v);
    }
    for (v, &c) in cb.iter().enumerate() {
        groups.get_mut(&c)?.1.push(v);
    }
    let groups: Vec<(Vec<usize>, Vec<usize>)> = groups.into_values().collect();
    groups.iter().all(|(ga, gb)| ga.len() == gb.len()).then_some(groups)
}

/// Comparable canonical form of a match set under column relabelling `perm`
/// (`perm[k]` is the new label of variable `k`): `{root ↦ sorted multiset of
/// factors}`, each factor a `(sorted new-slots, sorted rows aligned to that slot
/// order)`. Two match sets share a canonical form under some `perm` iff that one
/// global permutation makes them identical.
#[allow(clippy::type_complexity)]
fn canonicalize(matches: &[MatchAtEClass], perm: &[usize]) -> Vec<(u32, Vec<(Vec<usize>, Vec<Vec<u32>>)>)> {
    let mut out: Vec<(u32, Vec<(Vec<usize>, Vec<Vec<u32>>)>)> = matches
        .iter()
        .map(|m| {
            let mut factors: Vec<(Vec<usize>, Vec<Vec<u32>>)> = m
                .factors
                .iter()
                .map(|f| {
                    // Relabel slots, then sort columns by new label.
                    let mut cols: Vec<(usize, usize)> = f.slots.iter().enumerate().map(|(ci, &k)| (perm[k], ci)).collect();
                    cols.sort_unstable();
                    let slots: Vec<usize> = cols.iter().map(|&(s, _)| s).collect();
                    let mut rows: Vec<Vec<u32>> = f.rows.iter().map(|r| cols.iter().map(|&(_, ci)| id_u32(r[ci])).collect()).collect();
                    rows.sort_unstable();
                    (slots, rows)
                })
                .collect();
            factors.sort_unstable();
            (id_u32(m.root_eclass), factors)
        })
        .collect();
    out.sort_unstable();
    out
}

/// Enumerates every global colour-preserving permutation `perm[a_col] = b_col`,
/// one colour group at a time, returning `true` as soon as `test` accepts a
/// completed `perm` (short-circuits the remaining orderings).
fn enumerate_perms(groups: &[(Vec<usize>, Vec<usize>)], gi: usize, perm: &mut [usize], test: &mut dyn FnMut(&[usize]) -> bool) -> bool {
    let Some((ga, gb)) = groups.get(gi) else {
        return test(perm);
    };
    let mut order = gb.clone();
    let n = order.len();
    let mut found = false;
    heap_permute(&mut order, 0, n, &mut |order| {
        if found {
            return;
        }
        for (i, &av) in ga.iter().enumerate() {
            perm[av] = order[i];
        }
        found = enumerate_perms(groups, gi + 1, perm, test);
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::test_helpers::match_at;

    /// Exact slow-path check: a commutative swap is a genuine equivalence (a
    /// global column permutation witnesses it).
    #[test]
    fn equivalent_accepts_commutative_swap() {
        let a = vec![match_at(100, 2, vec![vec![10, 11]])];
        let b = vec![match_at(100, 2, vec![vec![11, 10]])];
        assert!(footprint_equivalent(&a, &b, 2));
    }

    /// Exact slow-path check: same per-column marginals but a different joint
    /// pairing is NOT equivalent — no global permutation maps one to the other.
    #[test]
    fn equivalent_rejects_different_joint() {
        let a = vec![match_at(100, 2, vec![vec![10, 11], vec![12, 13]])];
        let b = vec![match_at(100, 2, vec![vec![10, 13], vec![12, 11]])];
        assert!(!footprint_equivalent(&a, &b, 2));
    }

    /// Exact slow-path check: distinct roots are never equivalent (anchored ids).
    #[test]
    fn equivalent_rejects_distinct_roots() {
        let a = vec![match_at(100, 2, vec![vec![10, 11]])];
        let b = vec![match_at(200, 2, vec![vec![10, 11]])];
        assert!(!footprint_equivalent(&a, &b, 2));
    }
}
