use egg::Id;

use super::{LambdaCalcDisc, LambdaCalcLanguage, OpChildrenLanguage, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};

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
/// Cost weights are runtime values (`Weights`) carried on `StitchAnalysis`, so
/// they no longer parameterize this trait.
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

    /// Structural cost (sum of node costs over all enodes added by
    /// `add_stub_application`) of an `arity`-arg stub application — the
    /// head plus any spine nodes (e.g. curried `App`s) the family inserts.
    fn stub_application_size<O: StitchOp>(name: &str, arity: usize, weights: &Weights) -> u32;

    /// Build a pattern leaf containing the given pattern variable.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>>;

    /// Wrap an eclass in `n` lambda binders, returning the new eclass id.
    fn wrap_lams<O: StitchOp>(child: Id, n: u32, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;

    /// Total node-count cost of `n` stacked lambda binders under `weights`.
    fn lams_cost(n: u32, weights: &Weights) -> u32;

    /// In a pattern-side `RecExpr`, wrap `head` in `n` curried applications to
    /// fresh DB-var leaves `$0, $1, …, $(n-1)` (innermost first). Returns the
    /// id of the outermost App. Used by `Pattern::display_with_ho` to render
    /// HO body uses as `(@ … (@ ?#k $0) … $(n-1))`.
    fn wrap_pattern_with_db_apps<O: StitchOp>(recexpr: &mut egg::RecExpr<Self::Apply<OpWithVar<O>>>, head: Id, n: u32) -> Id;
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

    fn stub_application_size<O: StitchOp>(name: &str, _arity: usize, weights: &Weights) -> u32 {
        O::from_name(name).intrinsic_size(weights)
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> OpChildrenLanguage<OpWithVar<O>> {
        Self::make(OpWithVar::Var(v), vec![])
    }

    fn wrap_lams<O: StitchOp>(_child: Id, _n: u32, _egraph: &mut StitchEgraph<OpChildrenLanguage<O>>) -> Id {
        panic!("OpChildren has no lambda binders; higher-order capture is unreachable here");
    }

    fn lams_cost(_n: u32, _weights: &Weights) -> u32 {
        panic!("OpChildren has no lambda binders; higher-order capture is unreachable here");
    }

    fn wrap_pattern_with_db_apps<O: StitchOp>(_recexpr: &mut egg::RecExpr<OpChildrenLanguage<OpWithVar<O>>>, _head: Id, _n: u32) -> Id {
        panic!("OpChildren has no apps/binders; higher-order display is unreachable here");
    }
}

/// LambdaCalc family. Cost behavior is selected at runtime via the `Weights`
/// stored on `StitchAnalysis` (defaults to all-ones for babble parity; tune
/// per-kind via the `--sym-cost`/`--app-cost`/`--lam-cost` CLI flags).
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

    fn stub_application_size<O: StitchOp>(name: &str, arity: usize, weights: &Weights) -> u32 {
        LambdaCalcDisc::Leaf(O::from_name(name)).intrinsic_size(weights) + arity as u32 * weights.app_cost
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> LambdaCalcLanguage<OpWithVar<O>> {
        Self::make(LambdaCalcDisc::Leaf(OpWithVar::Var(v)), vec![])
    }

    fn wrap_lams<O: StitchOp>(child: Id, n: u32, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>>) -> Id {
        let mut current = child;
        for _ in 0..n {
            current = egraph.add(LambdaCalcLanguage::Lam([current]));
        }
        current
    }

    fn lams_cost(n: u32, weights: &Weights) -> u32 {
        n * weights.lam_cost
    }

    fn wrap_pattern_with_db_apps<O: StitchOp>(recexpr: &mut egg::RecExpr<LambdaCalcLanguage<OpWithVar<O>>>, head: Id, n: u32) -> Id {
        let mut current = head;
        for i in 0..n {
            let var_op = OpWithVar::Node(O::make_db_var(i).expect("higher-order display needs a DB-var-bearing leaf op"));
            let var_id = recexpr.add(LambdaCalcLanguage::Leaf(var_op));
            current = recexpr.add(LambdaCalcLanguage::App([current, var_id]));
        }
        current
    }
}
