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

/// Per-e-class analysis data: minimum AST size and the De Bruijn indices that
/// are free in *every* representative of the class.
///
/// `fv` is the intersection across enodes: an index `n` is in `fv` iff every
/// member of the class mentions `$n` freely. Equivalently, `n ∉ fv` means at
/// least one representative doesn't mention `$n` freely.
///
/// In general, the minimal term should have exactly this set of free variables,
/// so long as the rewrites do not introduce new free variables. We will
/// prove a theorem to this effect, but for now, we have a live assertion
/// that checks this whenever extracting a term.
///
/// Languages without binders or De Bruijn leaves leave `fv` empty everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StitchData {
    /// Minimum AST size among e-nodes in this e-class.
    pub size: u32,
    /// Free-variable set (intersection of members' free-var sets).
    pub fv: FxHashSet<i32>,
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
    /// - `fv`   = via `enode_fv`: `{n | disc.de_bruijn_index() == Some(n)} ∪ ⋃_j shift(child[j].fv, disc.binds_child(j))`,
    ///   where `shift(s, true)` decrements every index ≥ 1 by one and drops `0`.
    ///   A bare `Var(n)` leaf has fv `{n}` because nothing above it has bound `n` yet;
    ///   `Lam` is what removes bound indices on the way up.
    fn make(egraph: &mut egg::EGraph<L, Self>, enode: &L, _id: Id) -> Self::Data {
        let weights = egraph.analysis.weights;
        let disc = enode.discriminant();
        let size = disc.intrinsic_size(&weights) + enode.children().iter().map(|&c| egraph[c].data.size).sum::<u32>();
        let fv = enode_fv(enode, |c| &egraph[c].data.fv);
        StitchData { size, fv }
    }

    /// On merge: keep the minimum size, take the intersection of the two fv
    /// sets. Intersection reflects "free in every representative" — when two
    /// classes unify, an index that wasn't free in one of them is no longer
    /// guaranteed to be free in the merged class.
    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        let size_to_changed = from.size < to.size;
        let size_from_changed = from.size > to.size;
        if size_to_changed {
            to.size = from.size;
        }
        let to_had_extra = to.fv.iter().any(|x| !from.fv.contains(x));
        let from_had_extra = from.fv.iter().any(|x| !to.fv.contains(x));
        to.fv.retain(|x| from.fv.contains(x));
        egg::DidMerge(size_to_changed || to_had_extra, size_from_changed || from_had_extra)
    }
}

/// Type alias for the e-graph used throughout this codebase. Cost weights are
/// runtime state on the analysis, so the egraph type is no longer parameterized
/// by them.
pub type StitchEgraph<L> = egg::EGraph<L, StitchAnalysis>;

/// Per-enode free-variable rule, parameterised over how to look up child fv
/// sets. Used by both `StitchAnalysis::make` (children live in the egraph)
/// and `cost::recexpr_fv` (children live in a `RecExpr`'s flat node array).
///
/// `fv(node) = {n | disc.de_bruijn_index() == Some(n)} ∪ ⋃_j shift_j(child_fv(c_j))`,
/// where `shift_j` drops `0` and decrements ≥ 1 iff `disc.binds_child(j)`.
pub fn enode_fv<'a, L: StitchLanguage>(node: &L, child_fv: impl Fn(Id) -> &'a FxHashSet<i32>) -> FxHashSet<i32> {
    let disc = node.discriminant();
    let mut fv: FxHashSet<i32> = FxHashSet::default();
    if let Some(n) = disc.de_bruijn_index() {
        fv.insert(n);
    }
    for (j, &c) in node.children().iter().enumerate() {
        let cf = child_fv(c);
        if disc.binds_child(j) {
            fv.extend(cf.iter().filter_map(|&i| if i >= 1 { Some(i - 1) } else { None }));
        } else {
            fv.extend(cf.iter().copied());
        }
    }
    fv
}
