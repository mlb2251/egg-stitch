//! Unit tests for `candidates::enumerate_kept_subst_subsets`. Input is
//! `caps[k][s]` — the sorted-unique pattern-internal fv that subst `s`
//! references at variable slot `k`. Output is the list of distinct
//! "compatibility subsets" — sets of subst indices `S` for which there
//! exists an assignment `Sₖ` (chosen per slot) such that `S` is exactly
//! `{s : ∀k. caps[k][s] ⊆ Sₖ}`. Each returned kept-list is sorted ascending;
//! the outer list has a deterministic order but tests below assert the
//! exact ordering the function produces today.

use egg_stitch::candidates::enumerate_kept_subst_subsets;

/// When no subst references any fv, every subst is compatible with the
/// trivial `Sₖ = ∅` choice, and there is only one compatibility subset:
/// everyone.
#[test]
fn no_fv_anywhere_keeps_everyone() {
    let caps = vec![vec![vec![], vec![], vec![]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0, 1, 2]]);
}

/// With no slots there are no substs to keep, so no compatibility subsets
/// are returned.
#[test]
fn zero_slots_returns_nothing() {
    let caps: Vec<Vec<Vec<i32>>> = vec![];
    assert!(enumerate_kept_subst_subsets(&caps).is_empty());
}

/// Every subst references the same single fv. The only compatibility
/// subset is everyone — there is no way to keep a proper subset while
/// still satisfying the "exactly compatible" condition.
#[test]
fn all_substs_share_one_fv() {
    let caps = vec![vec![vec![0], vec![0], vec![0]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0, 1, 2]]);
}

/// Subst 1 references `$0`; substs 0 and 2 reference nothing. Two
/// compatibility subsets: keep only the empty-fv substs, or keep everyone.
#[test]
fn one_subst_with_fv_others_empty() {
    let caps = vec![vec![vec![], vec![0], vec![]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0, 2], vec![0, 1, 2]]);
}

/// Three substs at one slot: empty, `{$0}`, `{$1}`. Four compatibility
/// subsets — the empty-fv subst alone, plus either fv-bearing subst added
/// individually, plus everyone.
#[test]
fn two_substs_with_disjoint_single_fvs() {
    let caps = vec![vec![vec![], vec![0], vec![1]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0], vec![0, 1], vec![0, 2], vec![0, 1, 2]]);
}

/// Two slots, each contributing one fv-bearing subst, with one all-empty
/// subst in common. The two slot choices combine independently, giving
/// four compatibility subsets covering every combination of "include the
/// fv-bearing subst at this slot or not".
#[test]
fn two_slots_each_with_one_fv_bearing_subst() {
    // Slot 0: subst 0 captures `$0`.
    // Slot 1: subst 1 captures `$0`.
    let caps = vec![vec![vec![0], vec![], vec![]], vec![vec![], vec![0], vec![]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![2], vec![0, 2], vec![1, 2], vec![0, 1, 2]]);
}

/// Two substs reference identical fv. They are not distinguishable by any
/// compatibility condition, so they always appear together in any subset
/// that contains either of them.
#[test]
fn substs_with_identical_fv_always_grouped() {
    let caps = vec![vec![vec![0], vec![0], vec![]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![2], vec![0, 1, 2]]);
}

/// Pathologically wide inputs (21 substs each referencing a distinct fv)
/// fall back to a single "keep everyone" subset — the enumeration is
/// bounded so callers don't blow up on degenerate patterns.
#[test]
fn many_distinct_fvs_falls_back_to_keep_everyone() {
    let caps = vec![(0..21i32).map(|i| vec![i]).collect::<Vec<_>>()];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![(0..21).collect::<Vec<usize>>()]);
}

/// Subsumption chain at one slot: substs with caps `[]`, `{$0}`, `{$0,$1}`.
/// Each compatibility subset is a prefix of the chain — the wider-caps
/// subst can't be included without also satisfying the narrower ones, and
/// once it is included the strictly larger fv set has to fully appear.
#[test]
fn caps_form_subsumption_chain() {
    let caps = vec![vec![vec![], vec![0], vec![0, 1]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0], vec![0, 1], vec![0, 1, 2]]);
}

/// Single subst whose caps already contain several fv. Only one
/// compatibility subset is meaningful: keep that subst. The empty
/// alternative would yield an empty kept-list, which is pruned.
#[test]
fn single_subst_with_multiple_fv() {
    let caps = vec![vec![vec![0, 1, 2]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0]]);
}

/// Two substs with overlapping multi-fv caps `{$0}`, `{$1}`, `{$0,$1}`.
/// The cross-coverage subst forces the other two together; we get three
/// compatibility subsets corresponding to keeping each single-fv subst
/// alone or both plus the joint one.
#[test]
fn overlapping_multi_fv() {
    let caps = vec![vec![vec![0], vec![1], vec![0, 1]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0], vec![1], vec![0, 1, 2]]);
}

/// One subst captures fv at two slots simultaneously; others capture at
/// most one slot, and one captures nothing. Picking a slot's fv set "on"
/// or "off" is independent — but the cross-slot subst only appears when
/// both slots are turned on.
#[test]
fn cross_slot_fv_on_same_subst() {
    // Subst 0: fv at both slots. Subst 1: fv only at slot 1.
    // Subst 2: fv only at slot 0. Subst 3: empty.
    let caps = vec![vec![vec![0], vec![], vec![0], vec![]], vec![vec![0], vec![0], vec![], vec![]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![3], vec![2, 3], vec![1, 3], vec![0, 1, 2, 3]]);
}

/// Three slots, each with one subst that fv-captures at that slot
/// (different fv at each, but their values are irrelevant). Every
/// non-empty subset of the three substs is a compatibility subset —
/// turning on a slot's fv adds exactly its subst.
#[test]
fn three_slots_each_with_one_fv_subst() {
    let caps = vec![vec![vec![0], vec![], vec![]], vec![vec![], vec![0], vec![]], vec![vec![], vec![], vec![0]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0], vec![1], vec![0, 1], vec![2], vec![0, 2], vec![1, 2], vec![0, 1, 2]]);
}

/// The fv values themselves never matter — only the comparison structure.
/// Substituting large/irregular fv indices into the subsumption chain
/// produces the same output as the small-index version.
#[test]
fn fv_values_do_not_affect_output() {
    let caps = vec![vec![vec![], vec![42], vec![42, 100]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0], vec![0, 1], vec![0, 1, 2]]);
}

/// Two slots, each fully covered by every subst — every subst has caps
/// `{$0}` at both. There's no "drop a subst to shrink an fv set" choice
/// available, so only one compatibility subset emerges: everyone.
#[test]
fn every_subst_captures_at_every_slot() {
    let caps = vec![vec![vec![0], vec![0], vec![0]], vec![vec![0], vec![0], vec![0]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![0, 1, 2]]);
}

/// Many substs share the same nonzero caps at a slot, with one outlier
/// having empty caps. Two compatibility subsets: just the outlier, or
/// everyone. The grouped substs are inseparable.
#[test]
fn one_empty_subst_among_many_identical() {
    let caps = vec![vec![vec![0], vec![0], vec![0], vec![], vec![0]]];
    let got = enumerate_kept_subst_subsets(&caps);
    assert_eq!(got, vec![vec![3], vec![0, 1, 2, 3, 4]]);
}
