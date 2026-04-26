use egg::Id;

use super::{OpChildrenLanguage, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp};

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
pub trait LanguageFamily: Clone + 'static {
    /// Discriminant type for `Apply<O>`. Only needs `StitchDisc` (hash/eq/size/var
    /// detection) — `from_name` is not required since the family knows how to
    /// build var leaves directly via `make_var`.
    type Discriminant<O: StitchOp>: StitchDisc;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Discriminant<O>>;

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
