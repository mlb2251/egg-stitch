//! Permutation-invariant match-footprint deduplication for best-first search.
//!
//! Two candidate patterns have the same footprint when their set of match locations
//! M: {(r, sigma)} coincides modulo
//! 1. a permutation of the variables (i.e., sigma[v] = sigma'[perm[v]] for some global `perm`),
//! 2. reordering the substitutions within each factor
//!
//! As an example: `(+ ?#0 (* ?#1 2))` and `(+ (* ?#0 2) ?#1)` should be
//! considered equal, by swapping the variables.
//! 
//! We perform this check by computing an injective-in-practice "footprint signature"
//! for each candidate, which is a hash.
//! 
//! This is a double-order invariant and cannot be easily computed directly.
//! The obvious way would be to try every permutation of the variables
//! and then canonicalize the match sets relative to the variable order,
//! which is factorial in the number of variables.
//! 
//! We perform a more efficient variant of this where we first identify a
//! partial order on variables: specifically, we compute a "marginal identity hash"
//! for each variable that is invariant to the variable's position.
//! We then only consider permutations that respect the marginal identity hash order,
//! which is a much smaller set of permutations to consider. (We only have to
//! permute variables with an identical marginal identity hash).
//!
//! This process never expands out factors to the full cartesian product:
//! cartesian products are in general canonical and in our case mostly canonical
//! and close enough.
//! 
//! To avoid the expensive computation of the footprint hash in domains
//! where there are few true duplicates, we first compute a cheap hash that is not
//! injective-in-practice but does identify many unique patterns. If it does not
//! match any previous cheap hash, we store the cheap hash and move on without
//! needing to do any furthher computation.

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
