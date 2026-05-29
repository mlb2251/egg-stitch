use crate::cost::CostCandidate;
use crate::lang::{LanguageFamily, StitchEgraph, StitchOp};
use crate::search::SearchState;
use rustc_hash::{FxHashMap, FxHashSet};

/// Maximum number of distinct (slot, fv) pairs `enumerate_kept_subst_subsets`
/// is willing to enumerate. The OR-closure that builds canonical S-masks can
/// in the worst case reach `2^MAX_PACKED_FV_BITS` distinct masks, so this caps
/// the per-call work.
const MAX_PACKED_FV_BITS: u32 = 20;
const _: () = assert!(
    MAX_PACKED_FV_BITS <= 64,
    "MAX_PACKED_FV_BITS must be ≤ 64: the OR-closure encodes (slot, fv) pairs into a u64 bit-mask, and shifting `1u64 << b` for b ≥ 64 is undefined behaviour in Rust."
);

/// Core enumeration: given each subst's per-slot captured-fv set, return the
/// canonical subsets of subst indices to consider as rewrite candidates.
///
/// `var_captures[k][s]` is the sorted-unique pattern-internal fv referenced
/// by subst `s` at variable slot `k`. A subst `s` is compatible with a
/// candidate `S` iff `var_captures[k][s] ⊆ S_k` for every slot `k`. The
/// returned subsets are exactly the compatibility sets of canonical `S`
/// tuples — those for which `S_k = ⋃ var_captures[k][s]` over the
/// compatible substs. Equivalently, they are the OR-closure of the distinct
/// per-subst capture-masks. Empty subsets are dropped.
///
/// Falls back to a single "keep everything" subset when the packed mask
/// would exceed [`MAX_PACKED_FV_BITS`], so callers stay bounded.
pub fn enumerate_kept_subst_subsets(var_captures: &[Vec<Vec<i32>>]) -> Vec<Vec<usize>> {
    let arity = var_captures.len();
    let n_substs = if arity == 0 { 0 } else { var_captures[0].len() };
    // Per-slot sorted-ascending union of all referenced fv. Bit positions in
    // the packed mask are assigned by binary-searching into these.
    let v: Vec<Vec<i32>> = (0..arity)
        .map(|k| {
            let mut s: FxHashSet<i32> = FxHashSet::default();
            for caps in &var_captures[k] {
                s.extend(caps.iter().copied());
            }
            let mut x: Vec<i32> = s.into_iter().collect();
            x.sort_unstable();
            x
        })
        .collect();
    let total_bits: u32 = v.iter().map(|vk| vk.len() as u32).sum();
    if total_bits > MAX_PACKED_FV_BITS {
        // Fallback: too many distinct fv to enumerate — keep every subst.
        return vec![(0..n_substs).collect()];
    }
    let mut slot_offset: Vec<u32> = Vec::with_capacity(arity);
    let mut off = 0u32;
    for vk in &v {
        slot_offset.push(off);
        off += vk.len() as u32;
    }
    // Bucket substs by their R-mask. Walking distinct R-masks (typically few)
    // is cheaper than rewalking every subst per candidate.
    let mut bucket: FxHashMap<u64, Vec<usize>> = FxHashMap::default();
    #[allow(clippy::needless_range_loop)]
    for s_idx in 0..n_substs {
        let mut mask: u64 = 0;
        for k in 0..arity {
            for &i in &var_captures[k][s_idx] {
                let b = v[k].binary_search(&i).expect("captured fv missing from v[k]");
                mask |= 1u64 << (slot_offset[k] + b as u32);
            }
        }
        bucket.entry(mask).or_default().push(s_idx);
    }
    let distinct: Vec<u64> = bucket.keys().copied().collect();
    // Canonical s-masks are the OR-closure of distinct R-masks. DFS from 0;
    // far smaller than 2^total_bits in practice since R-masks share bits.
    let mut canonical_masks: Vec<u64> = Vec::new();
    let mut seen: FxHashSet<u64> = FxHashSet::default();
    let mut frontier: Vec<u64> = vec![0];
    seen.insert(0);
    while let Some(cur) = frontier.pop() {
        canonical_masks.push(cur);
        for &rm in &distinct {
            let new = cur | rm;
            if seen.insert(new) {
                frontier.push(new);
            }
        }
    }
    // Deterministic order: bucket HashMap doesn't promise iteration order,
    // and `compute_cost_and_select` picks the first candidate on ties.
    canonical_masks.sort_unstable();
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(canonical_masks.len());
    for &s_mask in &canonical_masks {
        let mut kept: Vec<usize> = Vec::new();
        for (&rm, idxs) in &bucket {
            if rm | s_mask == s_mask {
                kept.extend(idxs);
            }
        }
        if kept.is_empty() {
            // s_mask=0 with no all-empty-R subst falls through here.
            continue;
        }
        kept.sort_unstable();
        out.push(kept);
    }
    out
}

/// Enumerate every "meaningful" candidate. Thin wrapper around
/// [`enumerate_kept_subst_subsets`] that extracts per-(match, subst)
/// capture sets from the egraph and translates returned flat-subst-index
/// subsets back into `CostCandidate`s.
///
/// At least one candidate is always returned: when no slot can capture
/// pattern-internal binders, the result is a single empty-`S` candidate
/// with every subst kept (signalled by the `None` sentinel).
pub fn enumerate_candidates<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<CostCandidate> {
    let arity = search_state.pattern.var_depth.len();
    let var_depth = &search_state.pattern.var_depth;
    // Fast path: no slot can capture pattern-internal binders. Emit the
    // `None` sentinel to skip allocating the dense (0..len) kept list —
    // this fires on lambda-free domains and most calls elsewhere.
    if var_depth.iter().all(|&d| d == 0) {
        return vec![CostCandidate {
            variable_indices: vec![Vec::new(); arity],
            kept_substs: None,
        }];
    }
    // Flatten (match_idx, subst_idx) to a single index and collect per-slot
    // captures. `var_captures[k][flat] = sorted-unique pattern-internal fv
    // referenced by that subst's arg at slot k`.
    let n_substs: usize = search_state.matches.iter().map(|m| m.substs.len()).sum();
    let mut flat_to_pair: Vec<(usize, usize)> = Vec::with_capacity(n_substs);
    let mut var_captures: Vec<Vec<Vec<i32>>> = (0..arity).map(|_| Vec::with_capacity(n_substs)).collect();
    for (mi, m) in search_state.matches.iter().enumerate() {
        for (si, subst) in m.substs.iter().enumerate() {
            flat_to_pair.push((mi, si));
            for (k, &arg_id) in subst.vars.iter().enumerate() {
                let d_k = var_depth[k];
                // `data.fv` is already a set, so no dedup needed; sort to
                // give `enumerate_kept_subst_subsets` the canonical order it
                // expects.
                let mut caps: Vec<i32> = if d_k == 0 { Vec::new() } else { egraph[arg_id].data.fv.iter().copied().filter(|&i| i >= 0 && (i as u32) < d_k).collect() };
                caps.sort_unstable();
                var_captures[k].push(caps);
            }
        }
    }
    let n_matches = search_state.matches.len();
    enumerate_kept_subst_subsets(&var_captures)
        .into_iter()
        .map(|subset| {
            // Re-derive `variable_indices` as the union of captures across
            // the kept substs, and unflatten the kept indices per match.
            let mut variable_indices: Vec<Vec<i32>> = vec![Vec::new(); arity];
            let mut kept_per_match: Vec<Vec<usize>> = vec![Vec::new(); n_matches];
            for &flat in &subset {
                let (mi, si) = flat_to_pair[flat];
                kept_per_match[mi].push(si);
                for k in 0..arity {
                    variable_indices[k].extend(&var_captures[k][flat]);
                }
            }
            for vk in &mut variable_indices {
                vk.sort_unstable();
                vk.dedup();
            }
            for ks in &mut kept_per_match {
                ks.sort_unstable();
            }
            CostCandidate { variable_indices, kept_substs: Some(kept_per_match) }
        })
        .collect()
}
