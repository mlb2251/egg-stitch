use egg::{Analysis, FromOp, Id, Language, Symbol};
use std::convert::Infallible;
use std::fmt::{self, Display, Formatter};

/// A simple language based on egg's SymbolLang.

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

#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct StitchLang {
    /// The operator for an enode.
    pub op: Op,
    /// The enode's children `Id`s.
    pub children: Vec<Id>,
}

impl Language for StitchLang {
    /// Used for short-circuiting the search for equivalent nodes.
    type Discriminant = Op;

    fn discriminant(&self) -> Self::Discriminant {
        self.op
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

impl Display for StitchLang {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.op, f)
    }
}

impl FromOp for StitchLang {
    type Error = Infallible;

    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        let parsed_op = Op::Sym(op.into());
        Ok(Self { op: parsed_op, children })
    }
}

/// Egg analysis that tracks the minimum AST size of each e-class.
#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl Analysis<StitchLang> for StitchAnalysis {
    type Data = u32;

    /// Computes the minimum AST size of a new enode as 1 + sum of children's sizes.
    fn make(egraph: &mut egg::EGraph<StitchLang, Self>, enode: &StitchLang, _id: Id) -> Self::Data {
        1 + enode.children.iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
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
pub type StitchEgraph = egg::EGraph<StitchLang, StitchAnalysis>;
