use crate::cost::shift_free_egraph;
use crate::lang::{LanguageFamily, StitchEgraph, StitchLanguage, StitchOp};
use egg::Id;
use rustc_hash::{FxHashMap, FxHashSet};

/// Lookup of shifted versions of each e-class.
///
/// `map[c][s]` is the e-class id of the version of `c` whose free DB indices
/// have all been decremented by `s`. Only populated for `1 ≤ s ≤ max_depth(c)`
/// and only for classes whose original fv is non-empty — a shift of an fv-empty
/// class is the class itself, so storing it is wasted indirection.
///
/// Negative shifted indices (re-wrap slots) live inside these variant
/// e-classes; consumers reach them via `enodes_across_shifts` rather than
/// seeing them in the original class's enodes.
#[derive(Debug, Default, Clone)]
pub struct ShiftedVariants {
    pub(crate) map: FxHashMap<Id, FxHashMap<u32, Id>>,
}

impl ShiftedVariants {
    /// Returns the shifted variant of `eclass` by `s` (or `None` if none was
    /// built — i.e. `eclass` had empty fv or `s` exceeds the class's max
    /// observed depth). `s == 0` returns `eclass` itself.
    pub fn get(&self, eclass: Id, s: u32) -> Option<Id> {
        if s == 0 {
            return Some(eclass);
        }
        self.map.get(&eclass).and_then(|m| m.get(&s)).copied()
    }
}

/// Builds the shifted-variants index for `egraph`.
///
/// For each e-class `C` whose `data.fv` has a maximum index `m ≥ 1`, calls
/// `shift_free_egraph(C, -s, 0, …)` for each `s ∈ 1..=m` and records the
/// resulting e-class id under `map[C][s]`. A shared memo across all `(C, s)`
/// calls keeps the work proportional to the number of distinct `(class,
/// shift)` pairs reached.
///
/// Why `m` is the right upper bound: shifts in `1..=m` keep `max(fv) ≥ 0`,
/// so the shifted enodes have only non-negative DB-var leaves and structurally
/// agree with original-space classes (via egg's enode dedup, equal-content
/// classes union after `rebuild`). A shift of `m + 1` would produce a leaf
/// like `$-1` — a variant-only class with no original counterpart. Those
/// leaves leak into final extracted programs via captured metavar
/// substitutions in `subset_matches`, so we don't build them.
///
/// Mutates `egraph` (adds enodes for the shifted variants and calls
/// `rebuild()`). Post-rebuild, the recorded ids are canonicalized so callers
/// don't have to.
pub fn build_shifted_variants<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>) -> (ShiftedVariants, FxHashSet<Id>) {
    let class_ids: Vec<Id> = egraph.classes().map(|c| c.id).collect();
    let original_pre_rebuild = class_ids.clone();
    let mut memo: FxHashMap<(Id, u32, i32), Id> = FxHashMap::default();
    let mut map: FxHashMap<Id, FxHashMap<u32, Id>> = FxHashMap::default();

    for c in class_ids {
        let canonical = egraph.find(c);
        let max_fv = egraph[canonical].data.fv.iter().copied().max();
        let d = match max_fv {
            Some(m) if m >= 1 => m as u32,
            _ => continue,
        };
        let mut per_class: FxHashMap<u32, Id> = FxHashMap::default();
        for s in 1..=d {
            let shifted_id = shift_free_egraph::<F, O>(egraph, canonical, -(s as i32), 0, &mut memo);
            per_class.insert(s, shifted_id);
        }
        map.insert(canonical, per_class);
    }

    egraph.rebuild();

    // Canonicalize keys + values after rebuild — unions during the loop or
    // during rebuild may have merged classes.
    let mut canonical_map: FxHashMap<Id, FxHashMap<u32, Id>> = FxHashMap::default();
    for (src, inner) in map {
        let src = egraph.find(src);
        let entry = canonical_map.entry(src).or_default();
        for (s, dst) in inner {
            entry.insert(s, egraph.find(dst));
        }
    }

    let original_eclasses: FxHashSet<Id> = original_pre_rebuild.into_iter().map(|c| egraph.find(c)).collect();
    (ShiftedVariants { map: canonical_map }, original_eclasses)
}

/// Yields enodes from `eclass` and from every shifted-variant e-class of
/// `eclass`. Used by search sites that must consider every shift level as a
/// candidate for metavariable expansion or refinement. The original e-class is
/// yielded first, then variants in ascending order of shift amount — the order
/// is fully determined by the egraph + variant table so that RNG-seeded search
/// is reproducible across runs (the underlying `FxHashMap` iteration order is
/// stable for a given build but depends on insertion order and hash).
pub fn enodes_across_shifts<'a, L: StitchLanguage>(egraph: &'a StitchEgraph<L>, shifted: &'a ShiftedVariants, eclass: Id) -> impl Iterator<Item = &'a L> + 'a {
    nodes_across_shifts(egraph, shifted, eclass).map(|(_, n)| n)
}

/// Like `enodes_across_shifts`, but also yields the canonical id of the source
/// e-class each enode came from. Callers that need to distinguish "original
/// class" from "variant class" matches (e.g. to reject leaf-specialization
/// via a variant) use this; `enodes_across_shifts` is the convenience wrapper
/// that discards the source id.
pub fn nodes_across_shifts<'a, L: StitchLanguage>(egraph: &'a StitchEgraph<L>, shifted: &'a ShiftedVariants, eclass: Id) -> impl Iterator<Item = (Id, &'a L)> + 'a {
    let canonical = egraph.find(eclass);
    let mut variants: Vec<(u32, Id)> = shifted.map.get(&canonical).into_iter().flat_map(|m| m.iter().map(|(&s, &id)| (s, id))).collect();
    variants.sort_by_key(|&(s, _)| s);
    let extra = variants.into_iter().flat_map(move |(_, sid)| {
        let cid = egraph.find(sid);
        egraph[cid].nodes.iter().map(move |n| (cid, n))
    });
    egraph[canonical].nodes.iter().map(move |n| (canonical, n)).chain(extra)
}
