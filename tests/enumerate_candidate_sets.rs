//! Unit tests for `candidates::enumerate_candidate_sets`. Input is a list of
//! *generators* — one per `(factor, row)` — each a list of `(slot, captured-fv)`
//! pairs. Output is the candidate capture-sets `S`: the OR-closure of the
//! generators under per-slot union, each as a per-slot fv list `S[k]`, sorted by
//! packed mask (slot-major, fv-ascending). The empty candidate is always first.
//!
//! These cover the same scenarios as the old per-subst `enumerate_kept_subst_subsets`
//! tests, but assert the candidate `S`-sets the factored code actually consumes
//! (rather than kept subst-index subsets, which no longer exist).

use egg_stitch::candidates::enumerate_candidate_sets;

/// Convenience: a single-slot generator capturing `fv`.
fn g0(fv: &[i32]) -> Vec<(usize, Vec<i32>)> {
    if fv.is_empty() { vec![] } else { vec![(0, fv.to_vec())] }
}

/// No generator captures anything: the only candidate is the empty `S`.
#[test]
fn no_fv_anywhere_single_empty_candidate() {
    let gens = vec![g0(&[]), g0(&[]), g0(&[])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Zero slots: exactly one candidate, the empty tuple.
#[test]
fn zero_slots_single_empty_candidate() {
    let want: Vec<Vec<Vec<i32>>> = vec![vec![]];
    assert_eq!(enumerate_candidate_sets(0, &[]), want);
}

/// All generators share one fv: closure is `{∅, {$0}}`.
#[test]
fn all_share_one_fv() {
    let gens = vec![g0(&[0]), g0(&[0]), g0(&[0])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// One generator captures `$0`, others nothing: `{∅, {$0}}`.
#[test]
fn one_generator_with_fv() {
    let gens = vec![g0(&[]), g0(&[0]), g0(&[])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Disjoint single fvs `{$0}`, `{$1}`: full lattice `∅,{$0},{$1},{$0,$1}`.
#[test]
fn disjoint_single_fvs() {
    let gens = vec![g0(&[]), g0(&[0]), g0(&[1])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]], vec![vec![1]], vec![vec![0, 1]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Two slots, one fv-bearing generator each (plus an empty one): the two slot
/// choices combine independently into four candidates.
#[test]
fn two_slots_independent() {
    let gens = vec![vec![(0, vec![0])], vec![(1, vec![0])], vec![]];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![], vec![]], vec![vec![0], vec![]], vec![vec![], vec![0]], vec![vec![0], vec![0]]];
    assert_eq!(enumerate_candidate_sets(2, &gens), want);
}

/// Generators with identical fv collapse (deduped masks): `{∅, {$0}}`.
#[test]
fn identical_fv_collapses() {
    let gens = vec![g0(&[0]), g0(&[0]), g0(&[])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Too many distinct fv: fall back to the single keep-everything candidate.
#[test]
fn many_distinct_fvs_falls_back() {
    let gens: Vec<Vec<(usize, Vec<i32>)>> = (0..21i32).map(|i| vec![(0, vec![i])]).collect();
    let want: Vec<Vec<Vec<i32>>> = vec![vec![(0..21).collect()]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Subsumption chain `∅ ⊂ {$0} ⊂ {$0,$1}`: candidates are the chain prefixes.
#[test]
fn subsumption_chain() {
    let gens = vec![g0(&[]), g0(&[0]), g0(&[0, 1])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]], vec![vec![0, 1]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// A single generator with several fv: `{∅, {$0,$1,$2}}`.
#[test]
fn single_generator_multiple_fv() {
    let gens = vec![g0(&[0, 1, 2])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0, 1, 2]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Overlapping multi-fv `{$0}`, `{$1}`, `{$0,$1}`: the joint generator is the
/// union of the other two, so the closure is the full lattice.
#[test]
fn overlapping_multi_fv() {
    let gens = vec![g0(&[0]), g0(&[1]), g0(&[0, 1])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]], vec![vec![1]], vec![vec![0, 1]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// One generator captures at two slots at once; the cross-slot mask only arises
/// when both slots are turned on.
#[test]
fn cross_slot_generator() {
    let gens = vec![vec![(0, vec![0]), (1, vec![0])], vec![(1, vec![0])], vec![(0, vec![0])], vec![]];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![], vec![]], vec![vec![0], vec![]], vec![vec![], vec![0]], vec![vec![0], vec![0]]];
    assert_eq!(enumerate_candidate_sets(2, &gens), want);
}

/// Three slots, one fv-bearing generator each: every subset of the three slot
/// choices is a candidate (8 total).
#[test]
fn three_slots_independent() {
    let gens = vec![vec![(0, vec![0])], vec![(1, vec![0])], vec![(2, vec![0])]];
    let want: Vec<Vec<Vec<i32>>> = vec![
        vec![vec![], vec![], vec![]],
        vec![vec![0], vec![], vec![]],
        vec![vec![], vec![0], vec![]],
        vec![vec![0], vec![0], vec![]],
        vec![vec![], vec![], vec![0]],
        vec![vec![0], vec![], vec![0]],
        vec![vec![], vec![0], vec![0]],
        vec![vec![0], vec![0], vec![0]],
    ];
    assert_eq!(enumerate_candidate_sets(3, &gens), want);
}

/// fv values themselves are irrelevant — only the comparison structure matters.
#[test]
fn fv_values_do_not_affect_structure() {
    let gens = vec![g0(&[]), g0(&[42]), g0(&[42, 100])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![42]], vec![vec![42, 100]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}

/// Every generator captures `$0` at both slots: only `∅` and `{$0}×2`.
#[test]
fn every_generator_captures_at_every_slot() {
    let row = vec![(0, vec![0]), (1, vec![0])];
    let gens = vec![row.clone(), row.clone(), row];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![], vec![]], vec![vec![0], vec![0]]];
    assert_eq!(enumerate_candidate_sets(2, &gens), want);
}

/// Many identical-fv generators plus one empty: still just `{∅, {$0}}`.
#[test]
fn one_empty_among_many_identical() {
    let gens = vec![g0(&[0]), g0(&[0]), g0(&[0]), g0(&[]), g0(&[0])];
    let want: Vec<Vec<Vec<i32>>> = vec![vec![vec![]], vec![vec![0]]];
    assert_eq!(enumerate_candidate_sets(1, &gens), want);
}
