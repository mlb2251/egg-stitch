use egg::{Analysis, FromOp, Id, Language, Symbol};
use std::convert::Infallible;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::Hash;

/// Trait for the operator stored inside a language node.
///
/// Anything that names enodes works: a single `Symbol`, an enum of typed
/// constants, etc. The `from_name` constructor must be infallible because
/// `OpChildrenLanguage::from_op` parses arbitrary strings via egg's RecExpr parser.
pub trait StitchOp: Hash + Eq + Clone + Ord + Display + Debug + Send + Sync + 'static {
    /// Builds an op from its display name. Must succeed for every input string.
    fn from_name(s: &str) -> Self;
    /// The intrinsic size of this operator, used for AST size analysis.
    fn intrinsic_size(&self) -> u32 {
        1
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub enum Op {
    /// Opaque symbolic operator.
    Sym(Symbol),
}

impl Display for Op {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Op::Sym(s) => Display::fmt(s, f),
        }
    }
}

impl Op {
    pub fn as_str(&self) -> String {
        format!("{}", self)
    }
}

impl StitchOp for Op {
    fn from_name(s: &str) -> Self {
        Op::Sym(Symbol::from(s))
    }
}

/// Language where each enode is an operator plus a list of child Ids.
/// This language does not have currying-by-default but is more efficient
/// due to a smaller graph.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct OpChildrenLanguage<O = Op> {
    /// The operator for an enode.
    pub op: O,
    /// The enode's children `Id`s.
    pub children: Vec<Id>,
}

impl<O: StitchOp> Language for OpChildrenLanguage<O> {
    /// Used for short-circuiting the search for equivalent nodes.
    type Discriminant = O;

    fn discriminant(&self) -> Self::Discriminant {
        self.op.clone()
    }

    /// Returns true if this enode matches another enode.
    /// This should only consider the operator and the arity,
    /// not the children `Id`s.
    fn matches(&self, other: &Self) -> bool {
        self.op == other.op && self.len() == other.len()
    }

    fn children(&self) -> &[Id] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Id] {
        &mut self.children
    }
}

impl<O: StitchOp> Display for OpChildrenLanguage<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.op, f)
    }
}

impl<O: StitchOp> FromOp for OpChildrenLanguage<O> {
    type Error = Infallible;

    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        Ok(Self { op: O::from_name(op), children })
    }
}

/// Trait covering every language usable with the search machinery.
pub trait StitchLanguage: Language<Discriminant: StitchOp> + FromOp<Error: Debug + Send + Sync + std::error::Error> + Display + Clone + Send + Sync + 'static {
        /// Returns true if this operator represents a `programs` node, which is used as the root of the egraph and has special handling in `apply_abstraction`.
    fn is_programs_node(&self) -> bool;
}

impl StitchLanguage for OpChildrenLanguage<Op> {
    fn is_programs_node(&self) -> bool {
        self.op.to_string() == "programs"
    }
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
