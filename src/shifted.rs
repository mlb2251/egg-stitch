use crate::cost::shift_free_egraph_uniform;
use crate::lang::{LanguageFamily, StitchDisc, StitchEgraph, StitchLanguage, StitchOp};
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
/// e-classes; consumers walk them through `ShiftedVariants::get` rather
/// than seeing them in the original class's enodes.
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

/// Returns the maximum binder nesting depth reachable from `root`. Computed
/// via post-order DFS counting `disc.binds_child(j)` along the path. Used as
/// the upper bound on shift amounts in `build_shifted_variants` so every
/// `?#k`-depth a pattern can reach has a corresponding shifted-to-depth-0
/// variant available.
pub fn corpus_max_binder_depth<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> u32 {
    // BFS-from-root, tracking max binder depth at which each class is seen.
    let root = egraph.find(root);
    let mut best: FxHashMap<Id, u32> = FxHashMap::default();
    best.insert(root, 0);
    let mut stack: Vec<Id> = vec![root];
    let mut max_seen: u32 = 0;
    while let Some(cid) = stack.pop() {
        let depth_here = best[&cid];
        max_seen = max_seen.max(depth_here);
        for enode in &egraph[cid].nodes {
            let disc = enode.discriminant();
            for (j, &child) in enode.children().iter().enumerate() {
                let child = egraph.find(child);
                let child_depth = depth_here + if disc.binds_child(j) { 1 } else { 0 };
                let entry = best.entry(child).or_insert(u32::MAX);
                if child_depth < *entry {
                    *entry = child_depth;
                    stack.push(child);
                }
            }
        }
    }
    max_seen
}

/// Builds the shifted-variants index for `egraph`, populating shifts
/// `1..=max_shift` for every e-class. With `max_shift = corpus_max_binder_depth`,
/// every `(eclass, d_k)` pair a pattern slot can query is covered.
///
/// Shifts beyond `max(fv)` legitimately produce negative-fv variants: the
/// extra shift past the topmost free index turns each `$i` (with `i < shift`)
/// into the re-wrap slot `$(i - shift)`. These variants are what
/// `compute_ho_arity` reads to determine ho-arity (`= union of distinct neg
/// magnitudes across matches at slot k`).
///
/// A shared memo across all `(C, s)` calls keeps the work proportional to the
/// number of distinct `(class, shift)` pairs reached.
///
/// Mutates `egraph` (adds enodes for the shifted variants and calls
/// `rebuild()`). Post-rebuild, the recorded ids are canonicalized so callers
/// don't have to.
pub fn build_shifted_variants<F: LanguageFamily, O: StitchOp>(egraph: &mut StitchEgraph<F::Apply<O>>, max_shift: u32) -> (ShiftedVariants, FxHashSet<Id>) {
    let class_ids: Vec<Id> = egraph.classes().map(|c| c.id).collect();
    let original_pre_rebuild = class_ids.clone();
    let mut memo: FxHashMap<(Id, u32, i32), Id> = FxHashMap::default();
    let mut map: FxHashMap<Id, FxHashMap<u32, Id>> = FxHashMap::default();

    for c in class_ids {
        let canonical = egraph.find(c);
        // Skip closed classes — `shift_free_egraph` returns the class
        // unchanged when fv is empty, so a variant entry would be redundant.
        if egraph[canonical].data.fv.is_empty() {
            continue;
        }
        let mut per_class: FxHashMap<u32, Id> = FxHashMap::default();
        for s in 1..=max_shift {
            let shifted_id = shift_free_egraph_uniform::<F, O>(egraph, canonical, -(s as i32), 0, &mut memo);
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
