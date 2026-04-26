use egg::Id;

use super::{LambdaCalcLanguage, LambdaCalcOp, OpChildrenLanguage, OpWithVar, StitchEgraph, StitchLanguage, StitchOp};

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
/// leaf-op slot (like `LambdaCalcLanguage` with `App`/`Lam`/`Programs`) wrap `O`
/// in a sum so the discriminant carries the variant tag.
pub trait LanguageFamily: Clone + 'static {
    /// Discriminant type for `Apply<O>`.
    type Discriminant<O: StitchOp>: StitchOp;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Discriminant<O>>;

    /// Construct an enode from a discriminant op and a list of children. For
    /// families with fixed-arity structural variants, this dispatches on the
    /// variant + arity.
    fn make<P: StitchOp>(op: Self::Discriminant<P>, kids: Vec<Id>) -> Self::Apply<P>;

    /// Functor map over the leaf-Op slot of the discriminant. Structural
    /// variants pass through unchanged; embedded leaves go through `f`.
    /// Lifting a program-side discriminant into the pattern-side one is just
    /// `map_discriminant(op, OpWithVar::Node)`; no extra family method needed.
    fn map_discriminant<A: StitchOp, B: StitchOp>(op: Self::Discriminant<A>, f: impl FnMut(A) -> B) -> Self::Discriminant<B>;

    /// Add a `name(children...)` application to the egraph and return its Id.
    /// For families with binary `App` this builds a curried application chain.
    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;

    /// Build a pattern leaf containing the given pattern variable. Defaulted via
    /// `Op::from_name` since `OpWithVar::from_name` round-trips var syntax.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>> {
        Self::make(<Self::Discriminant<OpWithVar<O>> as StitchOp>::from_name(&v.to_string()), vec![])
    }
}

/// Marker for the `OpChildrenLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct OpChildren;

impl LanguageFamily for OpChildren {
    type Discriminant<O: StitchOp> = O;
    type Apply<O: StitchOp> = OpChildrenLanguage<O>;

    fn make<P: StitchOp>(op: P, kids: Vec<Id>) -> OpChildrenLanguage<P> {
        OpChildrenLanguage { op, children: kids }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: A, mut f: impl FnMut(A) -> B) -> B {
        f(op)
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<OpChildrenLanguage<O>>) -> Id {
        egraph.add(Self::make(O::from_name(name), children))
    }
}

/// Marker for the `LambdaCalcLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct LambdaCalc;

impl LanguageFamily for LambdaCalc {
    type Discriminant<O: StitchOp> = LambdaCalcOp<O>;
    type Apply<O: StitchOp> = LambdaCalcLanguage<O>;

    fn make<P: StitchOp>(op: LambdaCalcOp<P>, kids: Vec<Id>) -> LambdaCalcLanguage<P> {
        match (op, kids.as_slice()) {
            (LambdaCalcOp::Leaf(o), &[]) => LambdaCalcLanguage::Leaf(o),
            (LambdaCalcOp::App, &[f, a]) => LambdaCalcLanguage::App([f, a]),
            (LambdaCalcOp::Lam, &[b]) => LambdaCalcLanguage::Lam([b]),
            (LambdaCalcOp::Programs, _) => LambdaCalcLanguage::Programs(kids),
            (op, _) => panic!("LambdaCalc::make: {op} got wrong arity ({} children)", kids.len()),
        }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: LambdaCalcOp<A>, mut f: impl FnMut(A) -> B) -> LambdaCalcOp<B> {
        match op {
            LambdaCalcOp::Leaf(a) => LambdaCalcOp::Leaf(f(a)),
            LambdaCalcOp::App => LambdaCalcOp::App,
            LambdaCalcOp::Lam => LambdaCalcOp::Lam,
            LambdaCalcOp::Programs => LambdaCalcOp::Programs,
        }
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>>) -> Id {
        let mut current = egraph.add(LambdaCalcLanguage::Leaf(O::from_name(name)));
        for child in children {
            current = egraph.add(LambdaCalcLanguage::App([current, child]));
        }
        current
    }
}
