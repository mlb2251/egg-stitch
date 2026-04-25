use egg::{Analysis, FromOp, Id, Language};
use std::fmt::{Debug, Display};

mod op;
mod op_children;

pub use op::{Op, StitchOp};
pub use op_children::OpChildrenLanguage;

/// Trait covering every language usable with the search machinery.
pub trait StitchLanguage: Language<Discriminant: StitchOp> + FromOp<Error: Debug + Send + Sync + std::error::Error> + Display + Clone + Send + Sync + 'static {
    /// Returns true if this operator represents a `programs` node, which is used as the root of the egraph and has special handling in `apply_abstraction`.
    fn is_programs_node(&self) -> bool;
}

/// Egg analysis that tracks the minimum AST size of each e-class.
#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl<L: StitchLanguage> Analysis<L> for StitchAnalysis {
    type Data = u32;

    /// Computes the minimum AST size of a new enode as op size + sum of children's sizes.
    fn make(egraph: &mut egg::EGraph<L, Self>, enode: &L, _id: Id) -> Self::Data {
        enode.discriminant().intrinsic_size() + enode.children().iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
    }

    /// Keeps the minimum size when two e-classes are merged.
    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        if from < *to {
            *to = from;
            egg::DidMerge(true, false)
        } else if from == *to {
            egg::DidMerge(false, false)
        } else {
            egg::DidMerge(false, true)
        }
    }
}

/// Type alias for the e-graph used throughout this codebase.
pub type StitchEgraph<L> = egg::EGraph<L, StitchAnalysis>;
