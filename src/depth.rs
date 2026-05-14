use crate::lang::{StitchDisc, StitchEgraph, StitchLanguage};
use egg::Id;
use rustc_hash::FxHashMap;

/// For each e-class reachable from `root` along its size-minimal
/// representative tree, the number of binders enclosing it on that path.
///
/// We restrict to the size-minimal rep (the same enode chosen by
/// `shift_free_egraph` and by AstSize extraction) instead of walking every
/// enode in each class, for two reasons:
/// (1) DSR-induced unions can make the full enode graph cyclic — a class can
/// transitively contain itself via a binder edge — so a "max over all paths"
/// traversal does not terminate; (2) shifted variants are only useful at
/// depths the canonical extraction actually exhibits, since that's the
/// extraction the cost model and downstream apply path consume.
///
/// The rep graph is a DAG (size is well-defined: a class can't be its own
/// size-minimal sub-rep without contradiction), so the traversal terminates.
/// Classes unreachable from `root` along the rep tree are absent from the
/// result.
pub fn max_depth_per_eclass<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> FxHashMap<Id, u32> {
    let root = egraph.find(root);
    let weights = egraph.analysis.weights;
    let mut max_depth: FxHashMap<Id, u32> = FxHashMap::default();
    let mut stack: Vec<(Id, u32)> = vec![(root, 0)];
    while let Some((id, d)) = stack.pop() {
        match max_depth.get(&id).copied() {
            Some(prev) if prev >= d => continue,
            _ => max_depth.insert(id, d),
        };
        let rep = egraph[id]
            .nodes
            .iter()
            .min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&c| egraph[c].data.size as u64).sum::<u64>())
            .expect("non-empty eclass");
        let disc = rep.discriminant();
        for (j, &child) in rep.children().iter().enumerate() {
            let cd = d + u32::from(disc.binds_child(j));
            let child = egraph.find(child);
            if max_depth.get(&child).copied().is_none_or(|prev| cd > prev) {
                stack.push((child, cd));
            }
        }
    }
    max_depth
}
