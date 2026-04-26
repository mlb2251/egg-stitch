use egg::Symbol;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::Hash;

/// Trait for any "thing that names an enode shape" — what egg calls a
/// `Discriminant`. Used for hash-consing, equality, cost analysis, and
/// pattern-var detection. Doesn't require parsing from a string, so structural
/// discriminants and leaf ops both fit.
pub trait StitchDisc: Hash + Eq + Clone + Ord + Display + Debug + Send + Sync + 'static {
    /// The intrinsic size of this operator, used for AST size analysis.
    fn intrinsic_size(&self) -> u32 {
        1
    }
    /// If this op represents a pattern variable, returns it. Default: not a var.
    /// Var-aware op types (`OpWithVar` and wrappers around it) override.
    fn as_var(&self) -> Option<egg::Var> {
        None
    }
}

/// A leaf-op slot: a `StitchDisc` that can additionally be parsed from a name.
/// Used wherever we need to construct an op from an arbitrary input string
/// (egg's RecExpr parser, `add_stub_application`, etc.).
pub trait StitchOp: StitchDisc {
    /// Builds an op from its display name. Must succeed for every input string.
    fn from_name(s: &str) -> Self;
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

impl StitchDisc for Op {}

impl StitchOp for Op {
    fn from_name(s: &str) -> Self {
        Op::Sym(Symbol::from(s))
    }
}
