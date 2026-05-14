use crate::lang::{LanguageFamily, StitchEgraph, StitchOp};
use crate::shifted::ShiftedVariants;
use egg::Id;
use rustc_hash::FxHashSet;

/// The bundle that's threaded through every search entry point: the e-graph,
/// its corpus root, the side table of shifted-variant e-classes, and the set
/// of original (pre-enrichment) e-class ids. Bundling them keeps signatures
/// from sprouting parallel parameters at every layer and reflects that they
/// produce and consume together (e.g. `apply_abstraction` rebuilds the whole
/// bundle from the rewritten programs of the previous round).
#[derive(Debug, Clone)]
pub struct SharedData<F: LanguageFamily, O: StitchOp> {
    pub egraph: StitchEgraph<F::Apply<O>>,
    pub root: Id,
    pub shifted: ShiftedVariants,
    /// Canonical ids of the e-classes that existed *before*
    /// `build_shifted_variants` enriched the e-graph. Consumers like
    /// `CostCache` use this to exclude shifted-variant e-classes from
    /// dirty-bit propagation without re-deriving the set via a from-root DFS.
    pub original_eclasses: FxHashSet<Id>,
}

impl<F: LanguageFamily, O: StitchOp> SharedData<F, O> {
    pub fn new(egraph: StitchEgraph<F::Apply<O>>, root: Id, shifted: ShiftedVariants, original_eclasses: FxHashSet<Id>) -> Self {
        Self { egraph, root, shifted, original_eclasses }
    }
}
