use egg::{ENodeOrVar, FromOp, Id, Language, RecExpr};
use std::convert::Infallible;
use std::fmt::{self, Display, Formatter};

use super::{Op, OpChildrenLanguage, StitchLanguage, StitchOp};

/// A lambda-calculus shaped language: every node is either a `Leaf` symbol
/// (zero arity), a binary `App`, a unary `Lam`, or the corpus-root `Programs`.
///
/// Curried `App` chains are how multi-arity applications are represented; this
/// shape makes a `(f a b c)` corpus term align with `(f a b)` automatically,
/// since the leftmost prefix `(f a b)` is its own e-class.
///
/// `Programs` is kept as a flat multi-child variant rather than a curry chain
/// because it is the egraph root and is not a "real" application semantically.
///
/// Parameterized by leaf-Op `O` so the same shape can be reinstantiated for
/// patterns (with `O = OpWithVar<P>`).
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcLanguage<O = Op> {
    Leaf(O),
    App([Id; 2]),
    Lam([Id; 1]),
    Programs(Vec<Id>),
}

/// Discriminant for `LambdaCalcLanguage<O>`. Carries the structural variant tag
/// alongside the leaf op when applicable, so the discriminant differs from `O`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcOp<O = Op> {
    Leaf(O),
    App,
    Lam,
    Programs,
}

impl<O: Display> Display for LambdaCalcOp<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Leaf(o) => Display::fmt(o, f),
            Self::App => f.write_str("@"),
            Self::Lam => f.write_str("lam"),
            Self::Programs => f.write_str("programs"),
        }
    }
}

impl<O: StitchOp> StitchOp for LambdaCalcOp<O> {
    fn from_name(s: &str) -> Self {
        match s {
            "@" => Self::App,
            "lam" => Self::Lam,
            "programs" => Self::Programs,
            _ => Self::Leaf(O::from_name(s)),
        }
    }
    /// `App` and `Lam` are zero-cost structural wrappers so an appified
    /// `(@ (@ (@ f a) b) c)` has the same AST size as the flat `(f a b c)`.
    fn intrinsic_size(&self) -> u32 {
        match self {
            Self::App | Self::Lam => 0,
            Self::Programs => 1,
            Self::Leaf(o) => o.intrinsic_size(),
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Leaf(o) => o.as_var(),
            _ => None,
        }
    }
}

impl<O: StitchOp> Language for LambdaCalcLanguage<O> {
    type Discriminant = LambdaCalcOp<O>;

    fn discriminant(&self) -> Self::Discriminant {
        match self {
            Self::Leaf(o) => LambdaCalcOp::Leaf(o.clone()),
            Self::App(_) => LambdaCalcOp::App,
            Self::Lam(_) => LambdaCalcOp::Lam,
            Self::Programs(_) => LambdaCalcOp::Programs,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.discriminant() == other.discriminant() && self.children().len() == other.children().len()
    }

    fn children(&self) -> &[Id] {
        match self {
            Self::Leaf(_) => &[],
            Self::App(c) => c,
            Self::Lam(c) => c,
            Self::Programs(c) => c,
        }
    }

    fn children_mut(&mut self) -> &mut [Id] {
        match self {
            Self::Leaf(_) => &mut [],
            Self::App(c) => c,
            Self::Lam(c) => c,
            Self::Programs(c) => c,
        }
    }
}

impl<O: StitchOp> Display for LambdaCalcLanguage<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.discriminant(), f)
    }
}

impl<O: StitchOp> FromOp for LambdaCalcLanguage<O> {
    type Error = Infallible;

    /// Multi-arity applications are not representable as a single enode in this
    /// language; callers must appify before constructing or use `add_stub_application`.
    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        Ok(match (LambdaCalcOp::<O>::from_name(op), children.as_slice()) {
            (LambdaCalcOp::App, &[f, a]) => Self::App([f, a]),
            (LambdaCalcOp::Lam, &[b]) => Self::Lam([b]),
            (LambdaCalcOp::Programs, _) => Self::Programs(children),
            (LambdaCalcOp::Leaf(o), &[]) => Self::Leaf(o),
            // Multi-arity leaves get curried automatically so RecExpr/Pattern parsers
            // (which call `from_op` once per node) yield the appified shape directly.
            (LambdaCalcOp::Leaf(_), _) => panic!("multi-arity application of {op:?} can't be a single LambdaCalcLanguage node; appify first"),
            (LambdaCalcOp::App, _) | (LambdaCalcOp::Lam, _) => panic!("{op:?} expects fixed arity, got {} children", children.len()),
        })
    }
}

impl<O: StitchOp> StitchLanguage for LambdaCalcLanguage<O> {
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
pub fn appify_recexpr<O: StitchOp>(src: &RecExpr<OpChildrenLanguage<O>>) -> RecExpr<LambdaCalcLanguage<O>> {
    let mut out = RecExpr::default();
    appify_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn appify_walk<O: StitchOp>(out: &mut RecExpr<LambdaCalcLanguage<O>>, src: &RecExpr<OpChildrenLanguage<O>>, ptr: usize) -> Id {
    let node = &src.as_ref()[ptr];
    let kids: Vec<Id> = node.children.iter().map(|&c| appify_walk(out, src, c.into())).collect();
    add_appified::<_, O>(out, &node.op, kids, |out, n| out.add(n))
}

/// Appify a flat `(op kids...)` head into `LambdaCalcLanguage`, inserting curried App
/// chains for ordinary multi-arity ops. Inputs already in appified form (using `@`/`lam`)
/// are preserved structurally so e.g. rewrite files written in appified form re-parse
/// without double-currying. `wrap` lifts a `LambdaCalcLanguage` into the target node type
/// and adds it to the output, enabling reuse between `RecExpr<L>` and pattern-AST builds.
fn add_appified<N, O>(out: &mut RecExpr<N>, op: &O, kids: Vec<Id>, mut wrap: impl FnMut(&mut RecExpr<N>, LambdaCalcLanguage<O>) -> Id) -> Id
where
    N: egg::Language,
    O: StitchOp,
{
    match (LambdaCalcOp::<O>::from_name(&op.to_string()), kids.len()) {
        (LambdaCalcOp::App, 2) => wrap(out, LambdaCalcLanguage::App([kids[0], kids[1]])),
        (LambdaCalcOp::Lam, 1) => wrap(out, LambdaCalcLanguage::Lam([kids[0]])),
        (LambdaCalcOp::Programs, _) => wrap(out, LambdaCalcLanguage::Programs(kids)),
        (LambdaCalcOp::Leaf(o), _) => {
            let mut current = wrap(out, LambdaCalcLanguage::Leaf(o));
            for c in kids {
                current = wrap(out, LambdaCalcLanguage::App([current, c]));
            }
            current
        }
        (head, n) => panic!("special op {head} got wrong arity ({n} children)"),
    }
}

/// Inverse of `appify_recexpr`: collapse `App` chains back to flat `(f a b c)` form.
pub fn unappify_recexpr<O: StitchOp>(src: &RecExpr<LambdaCalcLanguage<O>>) -> RecExpr<OpChildrenLanguage<O>> {
    let mut out = RecExpr::default();
    unappify_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn unappify_walk<O: StitchOp>(out: &mut RecExpr<OpChildrenLanguage<O>>, src: &RecExpr<LambdaCalcLanguage<O>>, mut ptr: usize) -> Id {
    let nodes = src.as_ref();
    let mut tail_rev = vec![];
    while let LambdaCalcLanguage::App([head, arg]) = &nodes[ptr] {
        tail_rev.push(unappify_walk(out, src, (*arg).into()));
        ptr = (*head).into();
    }
    let kids: Vec<Id> = tail_rev.into_iter().rev().collect();
    let head_node = match &nodes[ptr] {
        LambdaCalcLanguage::Leaf(o) => OpChildrenLanguage { op: o.clone(), children: kids },
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
fn appify_pattern_ast<O: StitchOp>(src: &RecExpr<ENodeOrVar<OpChildrenLanguage<O>>>) -> RecExpr<ENodeOrVar<LambdaCalcLanguage<O>>> {
    let mut out = RecExpr::default();
    appify_pattern_walk(&mut out, src, src.as_ref().len() - 1);
    out
}

fn appify_pattern_walk<O: StitchOp>(out: &mut RecExpr<ENodeOrVar<LambdaCalcLanguage<O>>>, src: &RecExpr<ENodeOrVar<OpChildrenLanguage<O>>>, ptr: usize) -> Id {
    match &src.as_ref()[ptr] {
        ENodeOrVar::Var(v) => out.add(ENodeOrVar::Var(*v)),
        ENodeOrVar::ENode(n) => {
            let kids: Vec<Id> = n.children.iter().map(|&c| appify_pattern_walk(out, src, c.into())).collect();
            add_appified::<_, O>(out, &n.op, kids, |out, node| out.add(ENodeOrVar::ENode(node)))
        }
    }
}
