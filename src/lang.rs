use egg::{Analysis, FromOp, Id, Language, Symbol};
use std::convert::Infallible;
use std::fmt::{self, Display, Formatter};

/// A simple language based on egg's SymbolLang.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct StitchLang {
    /// The operator for an enode
    pub op: Symbol,
    /// The enode's children `Id`s
    pub children: Vec<Id>,
}

impl StitchLang {
    // Create an enode with the given string and children
    // pub fn new(op: impl Into<Symbol>, children: Vec<Id>) -> Self {
    //     let op = op.into();
    //     Self { op, children }
    // }

    // /// Create childless enode with the given string
    // pub fn leaf(op: impl Into<Symbol>) -> Self {
    //     Self::new(op, vec![])
    // }
}

impl Language for StitchLang {
    /// Used for short-circuiting the search for equivalent nodes.
    type Discriminant = Symbol;

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
        Ok(Self {
            op: op.into(),
            children,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl Analysis<StitchLang> for StitchAnalysis {
    type Data = u32;

    fn make(egraph: &mut egg::EGraph<StitchLang, Self>, enode: &StitchLang, _id: Id) -> Self::Data {
        1 + enode.children.iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
    }

    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        if from < *to {
            *to = from;
            egg::DidMerge(true, false)
        } else if from == *to {
            egg::DidMerge(false, false)
        } else {
            // from = *to; but we don't do this because types; idk it seems like they don't want us to
            egg::DidMerge(false, true)
        }
    }
}

pub type StitchEgraph = egg::EGraph<StitchLang, StitchAnalysis>;
