use egg::{Analysis, ENodeOrVar, FromOp, Id, Language, RecExpr};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;

mod family;
mod op;
mod op_children;
mod op_with_var;

pub use family::{LanguageFamily, OpChildren};
pub use op::{Op, StitchDisc, StitchOp};
pub use op_children::OpChildrenLanguage;
pub use op_with_var::OpWithVar;

/// Trait covering every language usable with the search machinery.
///
/// The default implementations are written for "flat" languages whose `from_op`
/// can build any-arity applications in a single enode (e.g. `OpChildrenLanguage`).
/// Languages with more constrained shapes can override the parse/display hooks
/// to bridge between the user-facing flat syntax and their internal representation.
pub trait StitchLanguage: Language<Discriminant: StitchDisc> + FromOp<Error: Debug + Send + Sync + std::error::Error> + Display + Clone + Send + Sync + 'static {
    /// Returns true if this operator represents a `programs` node, which is used as the root of the egraph and has special handling in `apply_abstraction`.
    fn is_programs_node(&self) -> bool;

    /// Parses a program s-expression in user-facing flat form.
    fn parse_program(s: &str) -> anyhow::Result<RecExpr<Self>> {
        s.parse().map_err(|e| anyhow::anyhow!("parse {s:?}: {e:?}"))
    }

    /// Parses a pattern s-expression (with `?x` variables) in user-facing flat form.
    fn parse_pattern_ast(s: &str) -> anyhow::Result<RecExpr<ENodeOrVar<Self>>> {
        let pat: egg::Pattern<Self> = s.parse().map_err(|e| anyhow::anyhow!("parse pattern {s:?}: {e:?}"))?;
        Ok(pat.ast)
    }

    /// Renders a `RecExpr` back to user-facing flat form. Used both for programs
    /// and (via `Pattern: Display`) for patterns.
    fn display_recexpr(expr: &RecExpr<Self>) -> String {
        expr.to_string()
    }
}

/// Cost model. `W: Weights<S>` assigns a `size` to a structural classifier `S`.
/// `S` is the family's O-independent variant tag (see `LanguageFamily::Structural`),
/// so a single `Weights` impl applies across every leaf-op instantiation of the family.
///
/// `DefaultWeights` is the universal fallback: every variant counts as 1.
pub trait Weights<S>: 'static {
    fn size(s: &S) -> u32;
}

#[derive(Clone, Debug, Default)]
pub struct DefaultWeights;

impl<S> Weights<S> for DefaultWeights {
    fn size(_s: &S) -> u32 {
        1
    }
}

/// Egg analysis that tracks the minimum weighted AST size of each e-class.
/// Generic over the family `F` and leaf-op `O` so it can dispatch through
/// `F::structural` / `F::Weights` to compute per-enode size.
pub struct StitchAnalysis<F, O>(PhantomData<fn() -> (F, O)>);

impl<F, O> Default for StitchAnalysis<F, O> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<F, O> Clone for StitchAnalysis<F, O> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<F, O> Debug for StitchAnalysis<F, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StitchAnalysis")
    }
}

impl<F: LanguageFamily, O: StitchOp> Analysis<F::Apply<O>> for StitchAnalysis<F, O> {
    type Data = u32;

    fn make(egraph: &mut egg::EGraph<F::Apply<O>, Self>, enode: &F::Apply<O>, _id: Id) -> Self::Data {
        enode_size::<F, O>(enode) + enode.children().iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
    }

    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        if from < *to {
            *to = from;
            egg::DidMerge(true, false)
        } else if from == *to {
            egg::DidMerge(false, false)
        } else {
            egg::DidMerge(false, true)
        }
    }
}

/// Weighted size of a single enode under the family's cost model.
pub fn enode_size<F: LanguageFamily, O: StitchOp>(enode: &F::Apply<O>) -> u32 {
    F::Weights::size(&F::structural::<O>(&enode.discriminant()))
}

/// Type alias for the e-graph used throughout this codebase, parameterized by
/// language family `F` and leaf-op `O`.
pub type StitchEgraph<F, O> = egg::EGraph<<F as LanguageFamily>::Apply<O>, StitchAnalysis<F, O>>;
