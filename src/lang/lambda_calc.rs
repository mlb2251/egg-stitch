use egg::{ENodeOrVar, FromOp, Id, Language, RecExpr};
use std::convert::Infallible;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::Hash;
use std::marker::PhantomData;

use super::{Op, OpChildrenLanguage, StitchDisc, StitchLanguage, StitchOp};

/// Per-enode cost configuration for a `LambdaCalc*` family.
///
/// Each profile produces a distinct `LambdaCalcDisc<O, W>` type, so the egraph's
/// hash-cons and cost analysis are independently parameterized per profile.
pub trait LambdaCalcWeights: 'static + Send + Sync + Clone + Eq + Ord + Hash + Debug {
    const LITERAL_COST: u32 = 1;
    const APP_COST: u32;
    const LAM_COST: u32;
}

/// Babble parity: every enode costs 1, matching `egg::AstSize`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct AstWeights;
impl LambdaCalcWeights for AstWeights {
    const APP_COST: u32 = 1;
    const LAM_COST: u32 = 1;
}

/// Zero-cost structural wrappers: an appified `(@ (@ (@ f a) b) c)` has the
/// same AST size as the flat `(f a b c)`. Useful when the search shouldn't be
/// biased against currying.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct UnitWeights;
impl LambdaCalcWeights for UnitWeights {
    const APP_COST: u32 = 0;
    const LAM_COST: u32 = 0;
}

/// Stitch-compatible weights: 1 for lam/app, 100 for literals.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct StitchWeights;
impl LambdaCalcWeights for StitchWeights {
    const APP_COST: u32 = 1;
    const LAM_COST: u32 = 1;
    const LITERAL_COST: u32 = 100;
}

/// A lambda-calculus shaped language: every node is either a `Leaf` symbol
/// (zero arity), a binary `App`, a unary `Lam`, or the corpus-root `Programs`.
///
/// Curried `App` chains are how multi-arity applications are represented.
/// `Programs` is kept as a flat multi-child variant rather than a curry chain
/// because it is the egraph root and is not a "real" application semantically.
///
/// Parameterized by leaf-Op `O` (so the same shape can be reinstantiated for
/// patterns with `O = OpWithVar<P>`) and a weight profile `W` (so different
/// `App`/`Lam` cost choices are different language types).
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcLanguage<O = Op, W: LambdaCalcWeights = AstWeights> {
    Leaf(O, PhantomData<W>),
    App([Id; 2]),
    Lam([Id; 1]),
    Programs(Vec<Id>),
}

/// Discriminant for `LambdaCalcLanguage<O, W>`. Carries the structural variant
/// tag alongside the leaf op when applicable, so the discriminant differs from `O`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcDisc<O = Op, W: LambdaCalcWeights = AstWeights> {
    Leaf(O, PhantomData<W>),
    App,
    Lam,
    Programs,
}

impl<O: Display, W: LambdaCalcWeights> Display for LambdaCalcDisc<O, W> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Leaf(o, _) => Display::fmt(o, f),
            Self::App => f.write_str("@"),
            Self::Lam => f.write_str("lam"),
            Self::Programs => f.write_str("programs"),
        }
    }
}

impl<O: StitchDisc, W: LambdaCalcWeights> StitchDisc for LambdaCalcDisc<O, W> {
    fn intrinsic_size(&self) -> u32 {
        match self {
            Self::App => W::APP_COST,
            Self::Lam => W::LAM_COST,
            // `Programs` is intentionally costed as a literal: it occupies one
            // slot in the corpus AST just like any leaf, and weight profiles
            // that scale leaf cost (e.g. `StitchWeights`) should scale it too.
            Self::Programs => W::LITERAL_COST,
            Self::Leaf(o, _) => W::LITERAL_COST * o.intrinsic_size(),
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Leaf(o, _) => o.as_var(),
            _ => None,
        }
    }
}

impl<O: StitchOp, W: LambdaCalcWeights> StitchOp for LambdaCalcDisc<O, W> {
    fn from_name(s: &str) -> Self {
        match s {
            "@" => Self::App,
            "lam" => Self::Lam,
            "programs" => Self::Programs,
            _ => Self::Leaf(O::from_name(s), PhantomData),
        }
    }
}

impl<O: StitchOp, W: LambdaCalcWeights> Language for LambdaCalcLanguage<O, W> {
    type Discriminant = LambdaCalcDisc<O, W>;

    fn discriminant(&self) -> Self::Discriminant {
        match self {
            Self::Leaf(o, _) => LambdaCalcDisc::Leaf(o.clone(), PhantomData),
            Self::App(_) => LambdaCalcDisc::App,
            Self::Lam(_) => LambdaCalcDisc::Lam,
            Self::Programs(_) => LambdaCalcDisc::Programs,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.discriminant() == other.discriminant() && self.children().len() == other.children().len()
    }

    fn children(&self) -> &[Id] {
        match self {
            Self::Leaf(_, _) => &[],
            Self::App(c) => c,
            Self::Lam(c) => c,
            Self::Programs(c) => c,
        }
    }

    fn children_mut(&mut self) -> &mut [Id] {
        match self {
            Self::Leaf(_, _) => &mut [],
            Self::App(c) => c,
            Self::Lam(c) => c,
            Self::Programs(c) => c,
        }
    }
}

impl<O: StitchOp, W: LambdaCalcWeights> Display for LambdaCalcLanguage<O, W> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.discriminant(), f)
    }
}

impl<O: StitchOp, W: LambdaCalcWeights> FromOp for LambdaCalcLanguage<O, W> {
    type Error = Infallible;

    /// Multi-arity applications are not representable as a single enode in this
    /// language; callers must appify before constructing or use `add_stub_application`.
    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        Ok(match (LambdaCalcDisc::<O, W>::from_name(op), children.as_slice()) {
            (LambdaCalcDisc::App, &[f, a]) => Self::App([f, a]),
            (LambdaCalcDisc::Lam, &[b]) => Self::Lam([b]),
            (LambdaCalcDisc::Programs, _) => Self::Programs(children),
            (LambdaCalcDisc::Leaf(o, _), &[]) => Self::Leaf(o, PhantomData),
            // Multi-arity leaves get curried automatically so RecExpr/Pattern parsers
            // (which call `from_op` once per node) yield the appified shape directly.
            (LambdaCalcDisc::Leaf(_, _), _) => panic!("multi-arity application of {op:?} can't be a single LambdaCalcLanguage node; appify first"),
            (LambdaCalcDisc::App, _) | (LambdaCalcDisc::Lam, _) => panic!("{op:?} expects fixed arity, got {} children", children.len()),
        })
    }
}

impl<O: StitchOp, W: LambdaCalcWeights> StitchLanguage for LambdaCalcLanguage<O, W> {
    fn is_programs_node(&self) -> bool {
        matches!(self, Self::Programs(_))
    }

    fn parse_program(s: &str) -> anyhow::Result<RecExpr<Self>> {
        let flat: RecExpr<OpChildrenLanguage<O>> = s.parse().map_err(|e| anyhow::anyhow!("parse {s:?}: {e:?}"))?;
        Ok(appify_recexpr(&flat))
    }

    fn parse_pattern_ast(s: &str) -> anyhow::Result<RecExpr<ENodeOrVar<Self>>> {
        let flat: egg::Pattern<OpChildrenLanguage<O>> = s.parse().map_err(|e| anyhow::anyhow!("parse pattern {s:?}: {e:?}"))?;
        Ok(appify_pattern_ast(&flat.ast))
    }

    fn display_recexpr(expr: &RecExpr<Self>) -> String {
        unappify_recexpr(expr).to_string()
    }
}

/// Rewrites `(f a b c)` → `(@ (@ (@ f a) b) c)`. The corpus root `(programs ...)`
/// is preserved as a single multi-child `Programs` node rather than curried.
fn appify_recexpr<O: StitchOp, W: LambdaCalcWeights>(src: &RecExpr<OpChildrenLanguage<O>>) -> RecExpr<LambdaCalcLanguage<O, W>> {
    let mut out = RecExpr::default();
    appify_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn appify_walk<O: StitchOp, W: LambdaCalcWeights>(out: &mut RecExpr<LambdaCalcLanguage<O, W>>, src: &RecExpr<OpChildrenLanguage<O>>, ptr: usize) -> Id {
    let node = &src.as_ref()[ptr];
    let kids: Vec<Id> = node.children.iter().map(|&c| appify_walk(out, src, c.into())).collect();
    add_appified::<_, O, W>(out, &node.op, kids, |out, n| out.add(n))
}

/// Appify a flat `(op kids...)` head into `LambdaCalcLanguage`, inserting curried App
/// chains for ordinary multi-arity ops.
fn add_appified<N, O, W>(out: &mut RecExpr<N>, op: &O, kids: Vec<Id>, mut wrap: impl FnMut(&mut RecExpr<N>, LambdaCalcLanguage<O, W>) -> Id) -> Id
where
    N: egg::Language,
    O: StitchOp,
    W: LambdaCalcWeights,
{
    match (LambdaCalcDisc::<O, W>::from_name(&op.to_string()), kids.len()) {
        (LambdaCalcDisc::App, 2) => wrap(out, LambdaCalcLanguage::App([kids[0], kids[1]])),
        (LambdaCalcDisc::Lam, 1) => wrap(out, LambdaCalcLanguage::Lam([kids[0]])),
        (LambdaCalcDisc::Programs, _) => wrap(out, LambdaCalcLanguage::Programs(kids)),
        (LambdaCalcDisc::Leaf(o, _), _) => {
            let mut current = wrap(out, LambdaCalcLanguage::Leaf(o, PhantomData));
            for c in kids {
                current = wrap(out, LambdaCalcLanguage::App([current, c]));
            }
            current
        }
        (head, n) => panic!("special op {head} got wrong arity ({n} children)"),
    }
}

/// Inverse of `appify_recexpr`: collapse `App` chains back to flat `(f a b c)` form.
fn unappify_recexpr<O: StitchOp, W: LambdaCalcWeights>(src: &RecExpr<LambdaCalcLanguage<O, W>>) -> RecExpr<OpChildrenLanguage<O>> {
    let mut out = RecExpr::default();
    unappify_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn unappify_walk<O: StitchOp, W: LambdaCalcWeights>(out: &mut RecExpr<OpChildrenLanguage<O>>, src: &RecExpr<LambdaCalcLanguage<O, W>>, mut ptr: usize) -> Id {
    let nodes = src.as_ref();
    let mut tail_rev = vec![];
    while let LambdaCalcLanguage::App([head, arg]) = &nodes[ptr] {
        tail_rev.push(unappify_walk(out, src, (*arg).into()));
        ptr = (*head).into();
    }
    let kids: Vec<Id> = tail_rev.into_iter().rev().collect();
    let head_node = match &nodes[ptr] {
        LambdaCalcLanguage::Leaf(o, _) => OpChildrenLanguage { op: o.clone(), children: kids },
        LambdaCalcLanguage::Programs(programs_kids) => {
            assert!(kids.is_empty(), "programs cannot be applied to extra args");
            let new_kids: Vec<Id> = programs_kids.iter().map(|&c| unappify_walk(out, src, c.into())).collect();
            OpChildrenLanguage { op: O::from_name("programs"), children: new_kids }
        }
        LambdaCalcLanguage::Lam([body]) => {
            assert!(kids.is_empty(), "lam in app head position not supported");
            let body_id = unappify_walk(out, src, (*body).into());
            OpChildrenLanguage { op: O::from_name("lam"), children: vec![body_id] }
        }
        LambdaCalcLanguage::App(_) => unreachable!("loop above consumes all App nodes"),
    };
    out.add(head_node)
}

/// Pattern-AST analogue of `appify_recexpr`. Pattern variables are carried through
/// unchanged (a `?x` leaf has no children, so currying never applies to it).
fn appify_pattern_ast<O: StitchOp, W: LambdaCalcWeights>(src: &RecExpr<ENodeOrVar<OpChildrenLanguage<O>>>) -> RecExpr<ENodeOrVar<LambdaCalcLanguage<O, W>>> {
    let mut out = RecExpr::default();
    appify_pattern_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn appify_pattern_walk<O: StitchOp, W: LambdaCalcWeights>(out: &mut RecExpr<ENodeOrVar<LambdaCalcLanguage<O, W>>>, src: &RecExpr<ENodeOrVar<OpChildrenLanguage<O>>>, ptr: usize) -> Id {
    match &src.as_ref()[ptr] {
        ENodeOrVar::Var(v) => out.add(ENodeOrVar::Var(*v)),
        ENodeOrVar::ENode(n) => {
            let kids: Vec<Id> = n.children.iter().map(|&c| appify_pattern_walk(out, src, c.into())).collect();
            add_appified::<_, O, W>(out, &n.op, kids, |out, node| out.add(ENodeOrVar::ENode(node)))
        }
    }
}
