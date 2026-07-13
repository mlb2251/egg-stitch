use crate::lang::{LanguageFamily, StitchEgraph, StitchOp};
use egg::Id;

/// The pair that's threaded through every search entry point: the e-graph and
/// its corpus root. Bundling them keeps signatures from sprouting two parallel
/// parameters at every layer and reflects that they're produced and consumed
/// together (e.g. `apply_abstraction` rebuilds both from the rewritten programs
/// of the previous round).
#[derive(Debug, Clone)]
pub struct SharedData<F: LanguageFamily, O: StitchOp> {
    pub egraph: StitchEgraph<F::Apply<O>>,
    pub root: Id,
}

impl<F: LanguageFamily, O: StitchOp> SharedData<F, O> {
    pub fn new(egraph: StitchEgraph<F::Apply<O>>, root: Id) -> Self {
        Self { egraph, root }
    }
}
