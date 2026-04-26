use std::fmt::{self, Display, Formatter};

use super::StitchOp;

/// An op-type wrapper that adds a pattern-variable variant.
///
/// Allows a variable to be used wherever a node would be in a
/// pattern
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum OpWithVar<O> {
    Node(O),
    Var(egg::Var),
}

impl<O: Display> Display for OpWithVar<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(o) => Display::fmt(o, f),
            Self::Var(v) => Display::fmt(v, f),
        }
    }
}

impl<O: StitchOp> StitchOp for OpWithVar<O> {
    fn from_name(s: &str) -> Self {
        if let Ok(v) = s.parse::<egg::Var>() { Self::Var(v) } else { Self::Node(O::from_name(s)) }
    }

    fn intrinsic_size(&self) -> u32 {
        match self {
            Self::Node(o) => o.intrinsic_size(),
            Self::Var(_) => 1,
        }
    }
}
