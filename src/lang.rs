use egg::{Analysis, FromOp, Id, Language, Symbol};
use rustc_hash::FxHashSet;
use std::convert::Infallible;
use std::fmt::{self, Display, Formatter};

/// A tagged operator for `StitchLang` enodes.
///
/// `Lam` and `Var` are binding-aware; `Sym` is an opaque symbolic operator
/// (e.g. `+`, `cos`, `programs`, learned `fn_0`, …).
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub enum Op {
    /// Lambda abstraction (arity 1).
    Lam,
    /// De Bruijn-indexed bound variable (arity 0).
    Var(u32),
    /// Opaque symbolic operator.
    Sym(Symbol),
}

impl Display for Op {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Op::Lam => f.write_str("lam"),
            Op::Var(n) => write!(f, "${}", n),
            Op::Sym(s) => Display::fmt(s, f),
        }
    }
}

/// A simple language based on egg's SymbolLang, with first-class lambda/variable nodes.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct StitchLang {
    /// The operator for an enode.
    pub op: Op,
    /// The enode's children `Id`s.
    pub children: Vec<Id>,
}

impl Language for StitchLang {
    /// Used for short-circuiting the search for equivalent nodes.
    type Discriminant = Op;

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

    /// Parses `"lam"` as `Op::Lam`, `"$n"` as `Op::Var(n)` (when the suffix is a valid `u32`),
    /// and anything else as `Op::Sym`.
    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        let parsed_op = if op == "lam" {
            Op::Lam
        } else if let Some(rest) = op.strip_prefix('$')
            && let Ok(n) = rest.parse::<u32>()
        {
            Op::Var(n)
        } else {
            Op::Sym(op.into())
        };
        Ok(Self { op: parsed_op, children })
    }
}

/// Per-e-class analysis data: minimum AST size and free-variable set.
///
/// `fv` is the *union* of free-variable sets across all e-nodes in the class: a De
/// Bruijn index `n` appears in `fv` if any representative of the class mentions `$n`
/// freely. This is the over-approximation we need for sound capture-handling at
/// extraction — if any member *could* carry `$n` free, the extractor might pick that
/// member, and the downstream wrapping needs to bind `$n`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StitchData {
    /// Minimum AST size among e-nodes in this e-class.
    pub size: u32,
    /// Free-variable set (intersection of members' free-var sets).
    pub fv: FxHashSet<u32>,
}

/// Egg analysis that tracks per-e-class size and free-variable set.
#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl Analysis<StitchLang> for StitchAnalysis {
    type Data = StitchData;

    /// Computes the data for a new enode:
    /// - `size` = 1 + sum of children's sizes.
    /// - `fv`   = `{n}` for `$n`, `{i-1 | i ∈ fv(body), i ≥ 1}` for `lam`, else union of children.
    fn make(egraph: &mut egg::EGraph<StitchLang, Self>, enode: &StitchLang, _id: Id) -> Self::Data {
        let size = 1 + enode.children.iter().map(|&c| egraph[c].data.size).sum::<u32>();
        let fv = match enode.op {
            Op::Lam => egraph[enode.children[0]].data.fv.iter().filter_map(|&i| if i >= 1 { Some(i - 1) } else { None }).collect(),
            Op::Var(n) => {
                // `($n child...)` is `$n` applied to children; fv = {n} ∪ ⋃ fv(children).
                let mut s = FxHashSet::default();
                s.insert(n);
                for &c in &enode.children {
                    s.extend(egraph[c].data.fv.iter().copied());
                }
                s
            }
            Op::Sym(_) => {
                let mut s = FxHashSet::default();
                for &c in &enode.children {
                    s.extend(egraph[c].data.fv.iter().copied());
                }
                s
            }
        };
        StitchData { size, fv }
    }

    /// On merge: keep the minimum size, and take the *union* of the two fv sets.
    /// Union is the over-approximation: `n ∈ fv` iff some representative carries `$n`
    /// free. Any extraction-time wrapping must cover everything in `fv`.
    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        let size_to_changed = from.size < to.size;
        let size_from_changed = from.size > to.size;
        if size_to_changed {
            to.size = from.size;
        }
        let fv_from_changed = to.fv.iter().any(|x| !from.fv.contains(x));
        let before_len = to.fv.len();
        to.fv.extend(from.fv.iter().copied());
        let fv_to_changed = to.fv.len() != before_len;
        egg::DidMerge(size_to_changed || fv_to_changed, size_from_changed || fv_from_changed)
    }
}

/// Type alias for the e-graph used throughout this codebase.
pub type StitchEgraph = egg::EGraph<StitchLang, StitchAnalysis>;
