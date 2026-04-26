use std::fmt::{self, Display, Formatter};

use super::{StitchDisc, StitchOp};

/// An op-type wrapper that adds a pattern-variable variant.
///
/// Used to lift any program-side op into a pattern-side op: programs of language
/// `L<O>` correspond to patterns of language `L<OpWithVar<O>>` (same Language
/// shape, leaf-Op extended with `Var`).
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

impl<O: StitchDisc> StitchDisc for OpWithVar<O> {
    fn intrinsic_size(&self) -> u32 {
        match self {
            Self::Node(o) => o.intrinsic_size(),
            Self::Var(_) => 1,
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Var(v) => Some(*v),
            Self::Node(o) => o.as_var(),
        }
    }
}

impl<O: StitchOp> StitchOp for OpWithVar<O> {
    fn from_name(s: &str) -> Self {
        if let Ok(v) = s.parse::<egg::Var>() { Self::Var(v) } else { Self::Node(O::from_name(s)) }
    }
}
