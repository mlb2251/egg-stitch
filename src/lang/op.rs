use egg::Symbol;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::Hash;

use super::Weights;

/// Trait for any "thing that names an enode shape" — what egg calls a
/// `Discriminant`. Used for hash-consing, equality, cost analysis, and
/// pattern-var detection. Doesn't require parsing from a string, so structural
/// discriminants and leaf ops both fit.
pub trait StitchDisc: Hash + Eq + Clone + Ord + Display + Debug + Send + Sync + 'static {
    /// Cost of an enode with this discriminant under the given weights. The
    /// default treats the node as a leaf and scales by `sym_cost`; structural
    /// discriminants (e.g. `LambdaCalcDisc::App`/`Lam`) override to read the
    /// matching field.
    fn intrinsic_size(&self, weights: &Weights) -> u32 {
        weights.sym_var_cost
    }
    /// If this op represents a pattern variable, returns it. Default: not a var.
    /// Var-aware op types (`OpWithVar` and wrappers around it) override.
    fn as_var(&self) -> Option<egg::Var> {
        None
    }
    /// If this op is a De Bruijn variable leaf, returns its index. Whether the
    /// occurrence is *free* in some enclosing context is decided elsewhere — this
    /// just reports the shape of the leaf.
    fn de_bruijn_index(&self) -> Option<u32> {
        None
    }
    /// True iff this op binds a fresh De Bruijn slot for its `j`th child — i.e.,
    /// indices in `child[j]`'s fv set should be decremented (and `0` dropped)
    /// before bubbling up.
    fn binds_child(&self, _j: usize) -> bool {
        false
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
