use crate::cost::CostCandidate;
use crate::lang::{LanguageFamily, StitchEgraph, StitchOp};
use crate::search::SearchState;
use rustc_hash::FxHashSet;

/// Maximum number of distinct (slot, fv) pairs [`enumerate_candidate_sets`] is
/// willing to enumerate. The OR-closure that builds candidate `S`-masks can in
/// the worst case reach `2^MAX_PACKED_FV_BITS` distinct masks, so this caps the
/// per-call work.
const MAX_PACKED_FV_BITS: u32 = 20;
const _: () = assert!(
    MAX_PACKED_FV_BITS <= 64,
    "MAX_PACKED_FV_BITS must be ≤ 64: the OR-closure encodes (slot, fv) pairs into a u64 bit-mask, and shifting `1u64 << b` for b ≥ 64 is undefined behaviour in Rust."
);

/// Enumerate the candidate capture-sets `S` from a set of *generator*
/// capture-sets over `arity` slots. Each generator lists the captured
/// pattern-internal fv at each slot it covers, as `(slot, fv)` pairs;
/// [`enumerate_candidates`] passes one generator per `(factor, row)`, so the
/// input is `Σ`-sized rather than the `∏`-sized flattened product.
///
/// The candidates are the OR-closure of the generators under per-slot union,
/// each returned as a per-slot sorted-unique fv list (`candidate[k]` = `S_k`).
/// A subst is compatible with candidate `S` iff its per-slot captures `R^k ⊆
/// S_k` at every slot; the closure is exactly the set of `S` reachable as a
/// union of some generator subset — the only non-dominated choices (any other
/// `S` keeps the same substs as its tightening `⋃captures(kept) ⊆ S` but has a
/// larger body). The empty candidate `S = ∅` is always included.
///
/// A generator's mask is the OR of its slots' bits, so closing the per-(factor,
/// row) masks reproduces the per-subst closure: a subst's mask is the OR of its
/// factors' row masks, hence already in the closure of those rows' masks.
///
/// Bounded: if the total number of distinct (slot, fv) pairs exceeds
/// [`MAX_PACKED_FV_BITS`], returns the single keep-everything candidate
/// `S_k = ⋃_g gen[k]` so callers stay within `2^MAX_PACKED_FV_BITS` masks.
pub fn enumerate_candidate_sets(arity: usize, generators: &[Vec<(usize, Vec<i32>)>]) -> Vec<Vec<Vec<i32>>> {
    // Per-slot sorted-ascending fv alphabet; bit positions are assigned by
    // binary-searching into these.
    let mut vsets: Vec<FxHashSet<i32>> = vec![FxHashSet::default(); arity];
    for g in generators {
        for (k, fvs) in g {
            vsets[*k].extend(fvs.iter().copied());
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
        // Fallback: too many distinct fv to enumerate — keep everything.
        return vec![v];
    }
    let mut slot_offset: Vec<u32> = Vec::with_capacity(arity);
    let mut off = 0u32;
    for vk in &v {
        slot_offset.push(off);
        off += vk.len() as u32;
    }
    // One mask per generator; collect distinct.
    let mut gens: FxHashSet<u64> = FxHashSet::default();
    for g in generators {
        let mut mask = 0u64;
        for (k, fvs) in g {
            for &i in fvs {
                let b = v[*k].binary_search(&i).expect("captured fv missing from v[k]");
                mask |= 1u64 << (slot_offset[*k] + b as u32);
            }
        }
        gens.insert(mask);
    }
    // OR-closure (DFS from the empty mask), sorted so `compute_cost_and_select`
    // sees a deterministic order.
    let distinct: Vec<u64> = gens.into_iter().collect();
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
    // Decode each mask into per-slot `S_k` (already sorted, since `v[k]` is
    // sorted and bits are visited in ascending order).
    canonical
        .into_iter()
        .map(|s_mask| {
            let mut s: Vec<Vec<i32>> = vec![Vec::new(); arity];
            for (k, vk) in v.iter().enumerate() {
                for (b, &i) in vk.iter().enumerate() {
                    if s_mask & (1u64 << (slot_offset[k] + b as u32)) != 0 {
                        s[k].push(i);
                    }
                }
            }
            s
        })
        .collect()
}

/// Enumerate every "meaningful" candidate `S` (the per-slot fv-sets), directly
/// from the factored match set — never materialising the product. Thin egraph
/// adapter over [`enumerate_candidate_sets`]: it turns each `(factor, row)` into
/// a generator (its captured pattern-internal fv per slot) and wraps each
/// returned `S` into a [`CostCandidate`]. The kept set for any candidate is
/// recovered later, per-factor, by `filter_factors_by_candidate`.
///
/// At least one candidate is always returned: the empty-`S` candidate, which on
/// lambda-free domains is the sole one (fast-pathed below).
pub fn enumerate_candidates<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, search_state: &SearchState<F, O>) -> Vec<CostCandidate> {
    let var_depth = &search_state.pattern.var_depth;
    let arity = var_depth.len();
    // Fast path: no slot can capture pattern-internal binders (lambda-free
    // domains and most calls) — skip scanning the factors entirely.
    if var_depth.iter().all(|&d| d == 0) {
        return vec![CostCandidate { variable_indices: vec![Vec::new(); arity] }];
    }
    // One generator per (factor, row): its captured fv per covered slot, where
    // captured fv of arg `v` at slot `k` is `fv(v) ∩ [0, d_k)`.
    let mut generators: Vec<Vec<(usize, Vec<i32>)>> = Vec::new();
    for m in &search_state.matches {
        for f in &m.factors {
            for row in &f.rows {
                let mut g: Vec<(usize, Vec<i32>)> = Vec::new();
                for (pos, &k) in f.slots.iter().enumerate() {
                    let d_k = var_depth[k];
                    if d_k == 0 {
                        continue;
                    }
                    let caps: Vec<i32> = egraph[row[pos]].data.fv.iter().copied().filter(|&i| i >= 0 && (i as u32) < d_k).collect();
                    if !caps.is_empty() {
                        g.push((k, caps));
                    }
                }
                generators.push(g);
            }
        }
    }
    enumerate_candidate_sets(arity, &generators).into_iter().map(|variable_indices| CostCandidate { variable_indices }).collect()
}
