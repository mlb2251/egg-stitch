use std::fmt::{self, Display, Formatter};

use super::{StitchDisc, StitchOp, Weights};

/// Like [`super::OpWithVar`], but with a third leaf variant that distinguishes a
/// *frozen* pattern variable from an ordinary one.
///
/// Used only by the experimental egraph-backed seen-tracker
/// (`search::SeenTracker`'s egraph side): encoding the per-var freeze bit
/// structurally — as a distinct enode — lets a pattern *and* its freeze mask be
/// a single term, so the seen-egraph hash-conses on `(syntax, frozen-set)`
/// together instead of needing a side value. Nothing in the normal search uses
/// this language; patterns are converted into it on the spot during
/// `check_and_insert`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum OpWithFrozenVar<O> {
    Node(O),
    Var(egg::Var),
    FrozenVar(egg::Var),
}

impl<O: Display> Display for OpWithFrozenVar<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(o) => Display::fmt(o, f),
            Self::Var(v) => Display::fmt(v, f),
            // `!` prefix keeps frozen vars distinct from ordinary ones on display
            // and round-trips through `from_name`.
            Self::FrozenVar(v) => write!(f, "!{v}"),
        }
    }
}

impl<O: StitchDisc> StitchDisc for OpWithFrozenVar<O> {
    fn intrinsic_size(&self, weights: &Weights) -> u32 {
        match self {
            Self::Node(o) => o.intrinsic_size(weights),
            Self::Var(_) | Self::FrozenVar(_) => weights.sym_var_cost,
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Var(v) | Self::FrozenVar(v) => Some(*v),
            Self::Node(o) => o.as_var(),
        }
    }

    fn de_bruijn_index(&self) -> Option<i32> {
        match self {
            Self::Node(o) => o.de_bruijn_index(),
            Self::Var(_) | Self::FrozenVar(_) => None,
        }
    }
}

impl<O: StitchOp> StitchOp for OpWithFrozenVar<O> {
    fn from_name(s: &str) -> Self {
        // Frozen vars are produced by conversion, never parsed in practice, but
        // keep `from_name` total and round-tripping with `Display`.
        if let Some(rest) = s.strip_prefix('!')
            && let Ok(v) = rest.parse::<egg::Var>()
        {
            return Self::FrozenVar(v);
        }
        if let Ok(v) = s.parse::<egg::Var>() {
            Self::Var(v)
        } else {
            Self::Node(O::from_name(s))
        }
    }

    fn make_db_var(n: i32) -> Option<Self> {
        O::make_db_var(n).map(Self::Node)
    }
}
