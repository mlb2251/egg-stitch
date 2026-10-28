use super::{Op, StitchDisc, StitchOp, Weights};
use std::fmt::{self, Display, Formatter};

/// Discriminant for the `TypeScript` family. The structural ops the family
/// itself builds and reasons about (`define`, `pass`, `app`, `lam{n}`) are
/// variants here, and every other op is the leaf-op `O` in `Node`. This keeps
/// binders out of the leaf-op type, so `TypeScript` works for any `O`
/// (`Op`, `OpDB<Op>`, `OpWithVar<OpDB<Op>>`, …) just like `LambdaCalcDisc`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum TsDisc<O = Op> {
    Define,
    Pass,
    App,
    Lam(u32),
    Node(O),
}

impl<O: Display> Display for TsDisc<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Define => f.write_str("define"),
            Self::Pass => f.write_str("pass"),
            Self::App => f.write_str("app"),
            Self::Lam(n) => write!(f, "lam{n}"),
            Self::Node(o) => Display::fmt(o, f),
        }
    }
}

impl<O: StitchDisc> StitchDisc for TsDisc<O> {
    fn intrinsic_size(&self, weights: &Weights) -> u32 {
        match self {
            Self::App => weights.app_cost,
            Self::Lam(_) => weights.lam_cost,
            Self::Define | Self::Pass => weights.sym_var_cost,
            Self::Node(o) => o.intrinsic_size(weights),
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Node(o) => o.as_var(),
            _ => None,
        }
    }

    fn de_bruijn_index(&self) -> Option<i32> {
        match self {
            Self::Node(o) => o.de_bruijn_index(),
            _ => None,
        }
    }

    /// `lam{n}` binds `n` slots in its body; `define` binds one in its
    /// continuation (child 1) but not in its value (child 0).
    fn binds_child(&self, j: usize) -> u32 {
        match (self, j) {
            (Self::Define, 1) => 1,
            (Self::Lam(n), 0) => *n,
            (Self::Node(o), j) => o.binds_child(j),
            _ => 0,
        }
    }
}

impl<O: StitchOp> StitchOp for TsDisc<O> {
    fn from_name(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("lam")
            && let Ok(n) = rest.parse::<u32>()
        {
            return Self::Lam(n);
        }
        match s {
            "define" => Self::Define,
            "pass" => Self::Pass,
            "app" => Self::App,
            _ => Self::Node(O::from_name(s)),
        }
    }

    fn make_db_var(n: i32) -> Option<Self> {
        O::make_db_var(n).map(Self::Node)
    }
}
