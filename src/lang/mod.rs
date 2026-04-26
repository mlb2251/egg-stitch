use egg::{Analysis, ENodeOrVar, FromOp, Id, Language, RecExpr};
use std::fmt::{Debug, Display};

mod lambda_calc;
mod op;
mod op_children;
mod op_with_var;

pub use lambda_calc::LambdaCalcLanguage;
pub use op::{Op, StitchOp};
pub use op_children::OpChildrenLanguage;
pub use op_with_var::OpWithVar;

use crate::pattern::Pattern;

/// Trait covering every language usable with the search machinery.
///
/// The default implementations are written for "flat" languages whose `from_op`
/// can build any-arity applications in a single enode (e.g. `OpChildrenLanguage`).
/// Languages with more constrained shapes (e.g. `LambdaCalcLanguage`, where every
/// application is binary) override the parse/display/stub hooks to bridge between
/// the user-facing flat syntax and their internal representation.
pub trait StitchLanguage: Language<Discriminant: StitchOp> + FromOp<Error: Debug + Send + Sync + std::error::Error> + Display + Clone + Send + Sync + 'static {
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

    /// Renders a program back to user-facing flat form.
    fn display_program(expr: &RecExpr<Self>) -> String {
        expr.to_string()
    }

    /// Renders a search pattern back to user-facing flat form.
    fn display_pattern(pat: &Pattern<Self>) -> String {
        pat.to_string()
    }

    /// Adds an `name(children...)` application to the egraph and returns its Id.
    /// For curried languages this builds the whole application chain, not a single node.
    fn add_stub_application(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self>) -> Id {
        let node = Self::from_op(name, children).expect("from_op should be infallible for stitch languages");
        egraph.add(node)
    }
}

/// Egg analysis that tracks the minimum AST size of each e-class.
#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl<L: StitchLanguage> Analysis<L> for StitchAnalysis {
    type Data = u32;

    /// Computes the minimum AST size of a new enode as op size + sum of children's sizes.
    fn make(egraph: &mut egg::EGraph<L, Self>, enode: &L, _id: Id) -> Self::Data {
        enode.discriminant().intrinsic_size() + enode.children().iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
    }

    /// Keeps the minimum size when two e-classes are merged.
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

/// Type alias for the e-graph used throughout this codebase.
pub type StitchEgraph<L> = egg::EGraph<L, StitchAnalysis>;
