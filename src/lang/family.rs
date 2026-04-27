use egg::Id;

use std::marker::PhantomData;

use crate::lang::lambda_calc::StitchWeights;

use super::lambda_calc::{AstWeights, LambdaCalcWeights, UnitWeights};
use super::{DefaultWeights, LambdaCalcDisc, LambdaCalcLanguage, OpChildrenLanguage, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};

/// A type-level type constructor `L<_>` for a language family.
///
/// Rust has no first-class higher-kinded types, so we simulate "the same
/// language `L<_>` instantiated with a different leaf-Op type" with this trait
/// plus GATs: `F::Apply<O>` is the spelling of `L<O>`. Programs are built over
/// `F::Apply<O>`; the corresponding pattern AST is built over
/// `F::Apply<OpWithVar<O>>`. Both share the same `F` (the language constructor),
/// and only the leaf-Op type differs.
///
/// `Discriminant<O>` is the discriminant of `Apply<O>`. Often it's just `O`
/// (`OpChildrenLanguage`), but languages with structural variants beyond a single
/// leaf-op slot can use a wrapper sum so the discriminant carries the variant tag.
///
/// `Weights<O>` chooses the cost model the egraph analysis runs under for this
/// family. Plain `OpChildren` uses `DefaultWeights` (size = `intrinsic_size`); the
/// lambda-calc families pick a `LambdaCalcWeights` profile.
pub trait LanguageFamily: Clone + 'static {
    /// Discriminant type for `Apply<O>`. Only needs `StitchDisc` (hash/eq/size/var
    /// detection) — `from_name` is not required since the family knows how to
    /// build var leaves directly via `make_var`.
    type Discriminant<O: StitchOp>: StitchDisc;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Discriminant<O>>;

    /// Cost model used by `StitchAnalysis` for this family's egraphs.
    type Weights<O: StitchOp>: Weights<Self::Apply<O>>;

    /// Construct an enode from a discriminant op and a list of children. For
    /// families with fixed-arity structural variants, this dispatches on the
    /// variant + arity.
    fn make<P: StitchOp>(op: Self::Discriminant<P>, kids: Vec<Id>) -> Self::Apply<P>;

    /// Functor map over the leaf-Op slot of the discriminant. Structural
    /// variants pass through unchanged; embedded leaves go through `f`.
    /// Lifting a program-side discriminant into the pattern-side one is just
    /// `map_discriminant(op, OpWithVar::Node)`.
    fn map_discriminant<A: StitchOp, B: StitchOp>(op: Self::Discriminant<A>, f: impl FnMut(A) -> B) -> Self::Discriminant<B>;

    /// Add a `name(children...)` application to the egraph and return its Id.
    /// For families with binary `App` this builds a curried application chain.
    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self::Apply<O>, Self::Weights<O>>) -> Id;

    /// Structural cost (sum of `Weights::size` over all enodes added by
    /// `add_stub_application`) of an `arity`-arg stub application — the
    /// head plus any spine nodes (e.g. curried `App`s) the family inserts.
    fn stub_application_size<O: StitchOp>(name: &str, arity: usize) -> u32;

    /// Build a pattern leaf containing the given pattern variable.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>>;
}

/// Marker for the `OpChildrenLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct OpChildren;

impl LanguageFamily for OpChildren {
    type Discriminant<O: StitchOp> = O;
    type Apply<O: StitchOp> = OpChildrenLanguage<O>;
    type Weights<O: StitchOp> = DefaultWeights;

    fn make<P: StitchOp>(op: P, kids: Vec<Id>) -> OpChildrenLanguage<P> {
        OpChildrenLanguage { op, children: kids }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: A, mut f: impl FnMut(A) -> B) -> B {
        f(op)
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<OpChildrenLanguage<O>, DefaultWeights>) -> Id {
        egraph.add(Self::make(O::from_name(name), children))
    }

    fn stub_application_size<O: StitchOp>(name: &str, _arity: usize) -> u32 {
        O::from_name(name).intrinsic_size()
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> OpChildrenLanguage<OpWithVar<O>> {
        Self::make(OpWithVar::Var(v), vec![])
    }
}

/// Generic LambdaCalc family parameterized by a weight profile. Three named
/// markers select the profiles we ship: `LambdaCalcAst` (babble-parity),
/// `LambdaCalcUnit` (curried wrappers free), and `LambdaCalcStitch` (stitch
/// literal/app weights).
#[derive(Clone, Copy, Debug)]
pub struct LambdaCalcWith<W: LambdaCalcWeights>(PhantomData<W>);

/// LambdaCalc with `App`/`Lam` cost = 1 (babble's `egg::AstSize` parity).
pub type LambdaCalcAst = LambdaCalcWith<AstWeights>;

/// LambdaCalc with `App`/`Lam` cost = 0 (curried wrappers contribute nothing).
pub type LambdaCalcUnit = LambdaCalcWith<UnitWeights>;

/// LambdaCalc that matches stitch.
pub type LambdaCalcStitch = LambdaCalcWith<StitchWeights>;

impl<W: LambdaCalcWeights> LanguageFamily for LambdaCalcWith<W> {
    type Discriminant<O: StitchOp> = LambdaCalcDisc<O>;
    type Apply<O: StitchOp> = LambdaCalcLanguage<O>;
    type Weights<O: StitchOp> = W;

    fn make<P: StitchOp>(op: LambdaCalcDisc<P>, kids: Vec<Id>) -> LambdaCalcLanguage<P> {
        match (op, kids.as_slice()) {
            (LambdaCalcDisc::Leaf(o), &[]) => LambdaCalcLanguage::Leaf(o),
            (LambdaCalcDisc::App, &[f, a]) => LambdaCalcLanguage::App([f, a]),
            (LambdaCalcDisc::Lam, &[b]) => LambdaCalcLanguage::Lam([b]),
            (LambdaCalcDisc::Programs, _) => LambdaCalcLanguage::Programs(kids),
            (op, _) => panic!("LambdaCalc::make: {op} got wrong arity ({} children)", kids.len()),
        }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: LambdaCalcDisc<A>, mut f: impl FnMut(A) -> B) -> LambdaCalcDisc<B> {
        match op {
            LambdaCalcDisc::Leaf(a) => LambdaCalcDisc::Leaf(f(a)),
            LambdaCalcDisc::App => LambdaCalcDisc::App,
            LambdaCalcDisc::Lam => LambdaCalcDisc::Lam,
            LambdaCalcDisc::Programs => LambdaCalcDisc::Programs,
        }
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>, W>) -> Id {
        let mut current = egraph.add(LambdaCalcLanguage::Leaf(O::from_name(name)));
        for child in children {
            current = egraph.add(LambdaCalcLanguage::App([current, child]));
        }
        current
    }

    fn stub_application_size<O: StitchOp>(name: &str, arity: usize) -> u32 {
        <W as Weights<LambdaCalcLanguage<O>>>::size(&LambdaCalcDisc::Leaf(O::from_name(name))) + arity as u32 * W::APP_COST
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> LambdaCalcLanguage<OpWithVar<O>> {
        Self::make(LambdaCalcDisc::Leaf(OpWithVar::Var(v)), vec![])
    }
}
