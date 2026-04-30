use egg::{Analysis, ENodeOrVar, FromOp, Id, Language, RecExpr};
use rustc_hash::FxHashSet;
use std::fmt::{Debug, Display};

mod family;
mod lambda_calc;
mod op;
mod op_children;
mod op_db;
mod op_with_var;

pub use family::{LambdaCalc, LanguageFamily, OpChildren};
pub use lambda_calc::{LambdaCalcDisc, LambdaCalcLanguage};
pub use op::{Op, StitchDisc, StitchOp};
pub use op_children::OpChildrenLanguage;
pub use op_db::OpDB;
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

/// Runtime cost configuration. Every enode size is computed by
/// `StitchDisc::size(&disc, weights)` against this struct.
///
/// Defaults to `{1, 1, 1}`, which matches babble's `egg::AstSize` for
/// `LambdaCalc` (and is the only meaningful setting for `OpChildren`, where
/// the lambda fields are unused). Override via the CLI flags below for
/// alternative profiles, e.g. zero-cost wrappers (`--app-cost 0 --lam-cost 0`)
/// or stitch compatibility (`--sym-cost 100`).
#[derive(clap::Args, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Weights {
    /// For symbols and variables
    #[arg(long, default_value_t = 1)]
    pub sym_var_cost: u32,
    /// Cost of an `App` enode in `LambdaCalc`. Unused for `OpChildren`.
    #[arg(long, default_value_t = 1)]
    pub app_cost: u32,
    /// Cost of a `Lam` enode in `LambdaCalc`. Unused for `OpChildren`.
    #[arg(long, default_value_t = 1)]
    pub lam_cost: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Self { sym_var_cost: 1, app_cost: 1, lam_cost: 1 }
    }
}

/// Per-e-class analysis data: minimum AST size and the union of free De Bruijn
/// indices across the class's representatives.
///
/// `fv` is an over-approximation: an index `n` is in `fv` iff *some* member of
/// the class mentions `$n` freely. This is the right semantics for downstream
/// capture handling — extraction may pick any representative, so anything that
/// could be free in any of them must be assumed free.
///
/// Languages without binders or De Bruijn leaves leave `fv` empty everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StitchData {
    /// Minimum AST size among e-nodes in this e-class.
    pub size: u32,
    /// Free-variable set (union of members' free-var sets).
    pub fv: FxHashSet<u32>,
}

/// Egg analysis that tracks size and free-variable set of each e-class,
/// weighted by the `Weights` value carried on the analysis itself.
#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis {
    pub weights: Weights,
}

impl StitchAnalysis {
    pub fn new(weights: Weights) -> Self {
        Self { weights }
    }
}

impl<L: StitchLanguage> Analysis<L> for StitchAnalysis {
    type Data = StitchData;

    /// Computes per-class data for a fresh enode:
    /// - `size` = `disc.intrinsic_size(weights) + Σ child.size`
    /// - `fv`   = `{n | disc.de_bruijn_index() == Some(n)} ∪ ⋃_j shift(child[j].fv, disc.binds_child(j))`,
    ///   where `shift(s, true)` decrements every index ≥ 1 by one and drops `0`.
    ///   A bare `Var(n)` leaf has fv `{n}` because nothing above it has bound `n` yet;
    ///   `Lam` is what removes bound indices on the way up.
    fn make(egraph: &mut egg::EGraph<L, Self>, enode: &L, _id: Id) -> Self::Data {
        let weights = egraph.analysis.weights;
        let disc = enode.discriminant();
        let size = disc.intrinsic_size(&weights) + enode.children().iter().map(|&c| egraph[c].data.size).sum::<u32>();
        let mut fv: FxHashSet<u32> = FxHashSet::default();
        if let Some(n) = disc.de_bruijn_index() {
            fv.insert(n);
        }
        for (j, &c) in enode.children().iter().enumerate() {
            let child_fv = &egraph[c].data.fv;
            if disc.binds_child(j) {
                fv.extend(child_fv.iter().filter_map(|&i| if i >= 1 { Some(i - 1) } else { None }));
            } else {
                fv.extend(child_fv.iter().copied());
            }
        }
        StitchData { size, fv }
    }

    /// On merge: keep the minimum size, take the union of the two fv sets.
    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        let size_to_changed = from.size < to.size;
        let size_from_changed = from.size > to.size;
        if size_to_changed {
            to.size = from.size;
        }
        let fv_from_changed = from.fv.iter().any(|x| !to.fv.contains(x));
        let before_len = to.fv.len();
        to.fv.extend(from.fv.iter().copied());
        let fv_to_changed = to.fv.len() != before_len;
        egg::DidMerge(size_to_changed || fv_to_changed, size_from_changed || fv_from_changed)
    }
}

/// Type alias for the e-graph used throughout this codebase. Cost weights are
/// runtime state on the analysis, so the egraph type is no longer parameterized
/// by them.
pub type StitchEgraph<L> = egg::EGraph<L, StitchAnalysis>;
