use egg::Id;

use super::{OpChildrenLanguage, StitchLanguage, StitchOp};

/// A type-level type constructor `L<_>` for a language family.
///
/// Rust has no first-class higher-kinded types, so we simulate "the same
/// language `L<_>` instantiated with a different leaf-Op type" with this trait
/// plus a GAT: `F::Apply<O>` is the spelling of `L<O>`. Programs are built over
/// `F::Apply<O>`; the corresponding pattern AST is built over
/// `F::Apply<OpWithVar<O>>`. Both share the same `F` (the language constructor),
/// and only the leaf-Op type differs.
pub trait LanguageFamily: Clone + 'static {
    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = O>;

    /// Construct an enode with the given op and children. Saves callers from
    /// hard-coding the storage struct's field layout.
    fn make<O: StitchOp>(op: O, kids: Vec<Id>) -> Self::Apply<O>;
}

/// Marker for the `OpChildrenLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct OpChildren;

impl LanguageFamily for OpChildren {
    type Apply<O: StitchOp> = OpChildrenLanguage<O>;

    fn make<O: StitchOp>(op: O, kids: Vec<Id>) -> OpChildrenLanguage<O> {
        OpChildrenLanguage { op, children: kids }
    }
}
