use std::fmt::{self, Display, Formatter};

use super::{StitchDisc, StitchOp, Weights};

/// An op-type wrapper that adds a De Bruijn-indexed variable variant.
///
/// Used to lift any leaf-Op type into a binding-aware one: programs of language
/// `L<O>` become `L<OpDB<O>>` once they need first-class `$n` references. The
/// fv analysis then picks `Var(n)` up via `StitchDisc::free_var`.
///
/// Composes with `OpWithVar` for the pattern side: `OpWithVar<OpDB<O>>` is the
/// pattern-leaf type that admits both pattern meta-vars (`?x`) and De Bruijn
/// vars (`$n`).
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub enum OpDB<O> {
    Node(O),
    Var(u32),
}

impl<O: Display> Display for OpDB<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(o) => Display::fmt(o, f),
            Self::Var(n) => write!(f, "${n}"),
        }
    }
}

impl<O: StitchDisc> StitchDisc for OpDB<O> {
    fn intrinsic_size(&self, weights: &Weights) -> u32 {
        match self {
            Self::Node(o) => o.intrinsic_size(weights),
            Self::Var(_) => weights.sym_var_cost,
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Node(o) => o.as_var(),
            Self::Var(_) => None,
        }
    }

    fn de_bruijn_index(&self) -> Option<u32> {
        match self {
            Self::Var(n) => Some(*n),
            Self::Node(o) => o.de_bruijn_index(),
        }
    }
}

impl<O: StitchOp> StitchOp for OpDB<O> {
    /// Parses `"$<u32>"` as `Var(n)`; anything else (including `"$foo"` with a
    /// non-numeric suffix) is delegated to the inner op type.
    fn from_name(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix('$')
            && let Ok(n) = rest.parse::<u32>()
        {
            Self::Var(n)
        } else {
            Self::Node(O::from_name(s))
        }
    }
}
