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
/// `Op<O>` is the discriminant of `Apply<O>`. Often it's just `O`
/// (`OpChildrenLanguage`), but languages with structural variants beyond a single
/// leaf-op slot (like `LambdaCalcLanguage` with `App`/`Lam`/`Programs`) wrap `O`
/// in a sum so the discriminant carries the variant tag.
pub trait LanguageFamily: Clone + 'static {
    /// Discriminant type for `Apply<O>`.
    type Op<O: StitchOp>: StitchOp;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Op<O>>;

    /// Build a pattern leaf containing the given pattern variable.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>>;

    /// Lift a program enode into the matching pattern enode shape, with the
    /// supplied new children. The pattern enode mirrors the program enode's
    /// structural variant; any leaf op gets wrapped in `OpWithVar::Node`.
    fn lift_to_pattern<O: StitchOp>(target: &Self::Apply<O>, new_children: Vec<Id>) -> Self::Apply<OpWithVar<O>>;

    /// Add a `name(children...)` application to the egraph and return its Id.
    /// For families with binary `App` this builds a curried application chain.
    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;
}

/// Marker for the `OpChildrenLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct OpChildren;

impl LanguageFamily for OpChildren {
    type Op<O: StitchOp> = O;
    type Apply<O: StitchOp> = OpChildrenLanguage<O>;

    fn make_var<O: StitchOp>(v: egg::Var) -> OpChildrenLanguage<OpWithVar<O>> {
        OpChildrenLanguage { op: OpWithVar::Var(v), children: vec![] }
    }

    fn lift_to_pattern<O: StitchOp>(target: &OpChildrenLanguage<O>, new_children: Vec<Id>) -> OpChildrenLanguage<OpWithVar<O>> {
        OpChildrenLanguage {
            op: OpWithVar::Node(target.op.clone()),
            children: new_children,
        }
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<OpChildrenLanguage<O>>) -> Id {
        egraph.add(OpChildrenLanguage { op: O::from_name(name), children })
    }
}

/// Marker for the `LambdaCalcLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct LambdaCalc;

impl LanguageFamily for LambdaCalc {
    type Op<O: StitchOp> = LambdaCalcOp<O>;
    type Apply<O: StitchOp> = LambdaCalcLanguage<O>;

    fn make_var<O: StitchOp>(v: egg::Var) -> LambdaCalcLanguage<OpWithVar<O>> {
        LambdaCalcLanguage::Leaf(OpWithVar::Var(v))
    }

    fn lift_to_pattern<O: StitchOp>(target: &LambdaCalcLanguage<O>, new_children: Vec<Id>) -> LambdaCalcLanguage<OpWithVar<O>> {
        match target {
            LambdaCalcLanguage::Leaf(o) => {
                debug_assert!(new_children.is_empty(), "leaf has no children");
                LambdaCalcLanguage::Leaf(OpWithVar::Node(o.clone()))
            }
            LambdaCalcLanguage::App(_) => {
                debug_assert_eq!(new_children.len(), 2, "App is binary");
                LambdaCalcLanguage::App([new_children[0], new_children[1]])
            }
            LambdaCalcLanguage::Lam(_) => {
                debug_assert_eq!(new_children.len(), 1, "Lam is unary");
                LambdaCalcLanguage::Lam([new_children[0]])
            }
            LambdaCalcLanguage::Programs(_) => LambdaCalcLanguage::Programs(new_children),
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
