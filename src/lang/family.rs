use egg::Id;

use super::{LambdaCalcDisc, LambdaCalcLanguage, OpChildrenLanguage, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp};

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
    type Discriminant<O: StitchOp>: StitchDisc;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Discriminant<O>>;

    /// Construct an enode from a discriminant op and a list of children.
    fn make<O: StitchOp>(op: Self::Discriminant<O>, kids: Vec<Id>) -> Self::Apply<O>;

    /// Functor map over the leaf-Op slot of the discriminant.
    fn map_discriminant<A: StitchOp, B: StitchOp>(op: Self::Discriminant<A>, f: impl FnMut(A) -> B) -> Self::Discriminant<B>;

    /// Add e.g., (fn_0 X Y Z) to the graph, and return its id.
    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;

    /// Build a pattern leaf containing the given pattern variable.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>>;
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

    fn make_var<O: StitchOp>(v: egg::Var) -> OpChildrenLanguage<OpWithVar<O>> {
        Self::make(OpWithVar::Var(v), vec![])
    }
}

/// Marker for the `LambdaCalcLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct LambdaCalc;

impl LanguageFamily for LambdaCalc {
    type Discriminant<O: StitchOp> = LambdaCalcDisc<O>;
    type Apply<O: StitchOp> = LambdaCalcLanguage<O>;

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

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>>) -> Id {
        let mut current = egraph.add(LambdaCalcLanguage::Leaf(O::from_name(name)));
        for child in children {
            current = egraph.add(LambdaCalcLanguage::App([current, child]));
        }
        current
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> LambdaCalcLanguage<OpWithVar<O>> {
        Self::make(LambdaCalcDisc::Leaf(OpWithVar::Var(v)), vec![])
    }
}
