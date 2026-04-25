use egg::{FromOp, Id, Language};
use std::convert::Infallible;
use std::fmt::{self, Display, Formatter};

use super::{Op, StitchLanguage, StitchOp};

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

impl StitchLanguage for OpChildrenLanguage<Op> {
    fn is_programs_node(&self) -> bool {
        self.op.to_string() == "programs"
    }
}
