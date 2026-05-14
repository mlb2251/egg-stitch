use crate::cost::shift_free_egraph;
use crate::depth::max_depth_per_eclass;
use crate::lang::{LanguageFamily, StitchEgraph, StitchLanguage, StitchOp};
use egg::Id;
use rustc_hash::FxHashMap;

/// Lookup of shifted versions of each e-class.
///
/// `map[c][s]` is the e-class id of the version of `c` whose free DB indices
/// have all been decremented by `s`. Only populated for `1 ≤ s ≤ max_depth(c)`
/// and only for classes whose original fv is non-empty — a shift of an fv-empty
/// class is the class itself, so storing it is wasted indirection.
///
/// Negative shifted indices (re-wrap slots) live inside these variant
/// e-classes; consumers reach them via `nodes_with_shifts` rather than seeing
/// them in the original class's enodes.
#[derive(Debug, Default, Clone)]
pub struct ShiftedVariants {
    pub map: FxHashMap<Id, FxHashMap<u32, Id>>,
}

impl ShiftedVariants {
    /// Returns the shifted variant of `eclass` by `s` (or `None` if none was
    /// built — i.e. `s == 0`, `eclass` had empty fv, or `s` exceeds the
    /// class's max observed depth).
    pub fn get(&self, eclass: Id, s: u32) -> Option<Id> {
        if s == 0 {
            return Some(eclass);
        }
        self.map.get(&eclass).and_then(|m| m.get(&s)).copied()
    }

    /// Iterates `(shift, eclass_id)` pairs covering the original class
    /// (`shift = 0`) and every shifted variant that was built. Useful for
    /// search sites that want to walk enodes across all shift levels of a
    /// target.
    pub fn shifts_for<'a>(&'a self, eclass: Id) -> Box<dyn Iterator<Item = (u32, Id)> + 'a> {
        match self.map.get(&eclass) {
            Some(m) => Box::new(std::iter::once((0u32, eclass)).chain(m.iter().map(|(&s, &id)| (s, id)))),
            None => Box::new(std::iter::once((0u32, eclass))),
        }
    }

    /// Set of e-class ids that are *only* shifted variants (introduced by
    /// `build_shifted_variants`, never appearing in a real program). Used to
    /// keep them out of match-root enumeration and other "real subterm"
    /// pipelines: they're alternative DB-indexings of other classes, not
    /// independent corpus subterms.
    pub fn variant_ids(&self) -> rustc_hash::FxHashSet<Id> {
        let mut sources: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
        let mut variants: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
        for (&src, inner) in &self.map {
            sources.insert(src);
            for &v in inner.values() {
                variants.insert(v);
            }
        }
        // A class that's both a source (shifted from) and a variant (shifted
        // into) is a real subterm too — keep it in the corpus.
        variants.retain(|id| !sources.contains(id));
        variants
    }
}

/// Builds the shifted-variants index for `egraph` rooted at `root`.
///
/// For each e-class `C` with non-empty `data.fv` and observed max depth `d`
/// under `root`, calls `shift_free_egraph(C, -s, 0, …)` for each `s ∈ 1..=d`
/// and records the resulting e-class id under `map[C][s]`. A shared memo
/// passed across all (C, s) calls keeps the work proportional to the number of
/// distinct `(class, shift)` pairs reached, not their cross-product.
///
/// Mutates `egraph` (adds enodes for the shifted variants and calls
/// `rebuild()`). Post-rebuild, the recorded ids are canonicalized so callers
/// don't have to.
pub fn build_shifted_variants<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, root: Id) -> ShiftedVariants {
    let depths = max_depth_per_eclass(egraph, root);
    let class_ids: Vec<Id> = egraph.classes().map(|c| c.id).collect();
    let mut memo: FxHashMap<(Id, u32, i32), Id> = FxHashMap::default();
    let mut map: FxHashMap<Id, FxHashMap<u32, Id>> = FxHashMap::default();

    for c in class_ids {
        let canonical = egraph.find(c);
        let d = depths.get(&canonical).copied().unwrap_or(0);
        if d == 0 || egraph[canonical].data.fv.is_empty() {
            continue;
        }
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

    ShiftedVariants { map: canonical_map }
}

/// Yields enodes from `eclass` and from every shifted-variant e-class of
/// `eclass`. Used by search sites that must consider every shift level as a
/// candidate for metavariable expansion or refinement. The original e-class is
/// always yielded first; order of the shifted-variant enodes is unspecified.
pub fn enodes_across_shifts<'a, L: StitchLanguage>(egraph: &'a StitchEgraph<L>, shifted: &'a ShiftedVariants, eclass: Id) -> impl Iterator<Item = &'a L> + 'a {
    let canonical = egraph.find(eclass);
    let extra = shifted.map.get(&canonical).into_iter().flat_map(move |m| m.values().flat_map(move |&sid| egraph[egraph.find(sid)].nodes.iter()));
    egraph[canonical].nodes.iter().chain(extra)
}
