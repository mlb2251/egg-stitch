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

/// Enumerate every "meaningful" candidate `S` (the per-slot fv-sets), directly
/// from the factored match set — never materialising the product.
///
/// The candidate space is the OR-closure of the substs' per-slot capture masks
/// (see [`enumerate_kept_subst_subsets`] for the spec). Each subst's full mask
/// is the OR of its factors' per-row masks, so we OR-close the **per-(factor,
/// row) masks** instead — a `Σ`-sized generating set whose closure is a superset
/// of the per-subst closure. Every extra mask it contributes keeps a strict
/// subset of substs and is therefore cost-dominated by its canonical tightening
/// (`S'' = ⋃captures(kept) ⊆ S`, smaller body, same kept set), so the
/// cost-minimising selection is identical. The kept set for any candidate is
/// recovered later, per-factor, by `filter_factors_by_candidate`.
///
/// At least one candidate is always returned: the empty-`S` mask `0` is always
/// in the closure, and on lambda-free domains it's the sole candidate.
pub fn enumerate_candidates<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<CostCandidate> {
    let var_depth = &search_state.pattern.var_depth;
    let arity = var_depth.len();
    // Fast path: no slot can capture pattern-internal binders.
    if var_depth.iter().all(|&d| d == 0) {
        return vec![CostCandidate { variable_indices: vec![Vec::new(); arity] }];
    }
    // Per-slot sorted-unique captured fv — the candidate bit alphabet — gathered
    // straight from the factors. Captured fv of arg `v` at slot `k` is
    // `fv(v) ∩ [0, d_k)`.
    let mut vsets: Vec<FxHashSet<i32>> = vec![FxHashSet::default(); arity];
    for m in &search_state.matches {
        for f in &m.factors {
            for (pos, &k) in f.slots.iter().enumerate() {
                let d_k = var_depth[k];
                if d_k == 0 {
                    continue;
                }
                for row in &f.rows {
                    for &i in &egraph[row[pos]].data.fv {
                        if i >= 0 && (i as u32) < d_k {
                            vsets[k].insert(i);
                        }
                    }
                }
            }
        }
    }
    let v: Vec<Vec<i32>> = vsets
        .into_iter()
        .map(|s| {
            let mut x: Vec<i32> = s.into_iter().collect();
            x.sort_unstable();
            x
        })
        .collect();
    let total_bits: u32 = v.iter().map(|vk| vk.len() as u32).sum();
    if total_bits > MAX_PACKED_FV_BITS {
        // Fallback: too many distinct fv to enumerate — keep every subst, i.e.
        // `S_k = all captures at slot k`.
        return vec![CostCandidate { variable_indices: v }];
    }
    let mut slot_offset: Vec<u32> = Vec::with_capacity(arity);
    let mut off = 0u32;
    for vk in &v {
        slot_offset.push(off);
        off += vk.len() as u32;
    }
    // Generator masks: one per (factor, row), bits set for that row's captures.
    // Each touches only its factor's slots, so OR-closing these reproduces the
    // per-subst closure (see fn docs). Collect distinct.
    let mut generators: FxHashSet<u64> = FxHashSet::default();
    for m in &search_state.matches {
        for f in &m.factors {
            for row in &f.rows {
                let mut mask = 0u64;
                for (pos, &k) in f.slots.iter().enumerate() {
                    let d_k = var_depth[k];
                    if d_k == 0 {
                        continue;
                    }
                    for &i in &egraph[row[pos]].data.fv {
                        if i >= 0 && (i as u32) < d_k {
                            let b = v[k].binary_search(&i).expect("captured fv missing from v[k]");
                            mask |= 1u64 << (slot_offset[k] + b as u32);
                        }
                    }
                }
                generators.insert(mask);
            }
        }
    }
    // OR-closure (DFS from the empty mask).
    let distinct: Vec<u64> = generators.into_iter().collect();
    let mut canonical: Vec<u64> = Vec::new();
    let mut seen: FxHashSet<u64> = FxHashSet::default();
    let mut frontier: Vec<u64> = vec![0];
    seen.insert(0);
    while let Some(cur) = frontier.pop() {
        canonical.push(cur);
        for &g in &distinct {
            let nw = cur | g;
            if seen.insert(nw) {
                frontier.push(nw);
            }
        }
    }
    canonical.sort_unstable();
    // Decode each canonical mask into per-slot `S_k` (already sorted, since
    // `v[k]` is sorted and bits are visited in order).
    canonical
        .into_iter()
        .map(|s_mask| {
            let mut variable_indices: Vec<Vec<i32>> = vec![Vec::new(); arity];
            for (k, vk) in v.iter().enumerate() {
                for (b, &i) in vk.iter().enumerate() {
                    if s_mask & (1u64 << (slot_offset[k] + b as u32)) != 0 {
                        variable_indices[k].push(i);
                    }
                }
            }
            CostCandidate { variable_indices }
        })
        .collect()
}
