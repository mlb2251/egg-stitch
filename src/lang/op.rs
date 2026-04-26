use egg::Symbol;
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
