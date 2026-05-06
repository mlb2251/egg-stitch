use egg::{ENodeOrVar, FromOp, Id, Language, RecExpr};
use std::convert::Infallible;
use std::fmt::{self, Debug, Display, Formatter};

use super::{Op, OpChildrenLanguage, StitchDisc, StitchLanguage, StitchOp, Weights};

/// A lambda-calculus shaped language: every node is either a `Leaf` symbol
/// (zero arity), a binary `App`, a unary `Lam`, or the corpus-root `Programs`.
///
/// Curried `App` chains are how multi-arity applications are represented.
/// `Programs` is kept as a flat multi-child variant rather than a curry chain
/// because it is the egraph root and is not a "real" application semantically.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcLanguage<O = Op> {
    Leaf(O),
    App([Id; 2]),
    Lam([Id; 1]),
    Programs(Vec<Id>),
}

/// Discriminant for `LambdaCalcLanguage<O>`. Carries the structural variant
/// tag alongside the leaf op when applicable, so the discriminant differs from `O`.
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum LambdaCalcDisc<O = Op> {
    Leaf(O),
    App,
    Lam,
    Programs,
}

impl<O: Display> Display for LambdaCalcDisc<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Leaf(o) => Display::fmt(o, f),
            Self::App => f.write_str("@"),
            Self::Lam => f.write_str("lam"),
            Self::Programs => f.write_str("programs"),
        }
    }
}

impl<O: StitchDisc> StitchDisc for LambdaCalcDisc<O> {
    fn intrinsic_size(&self, weights: &Weights) -> u32 {
        match self {
            Self::App => weights.app_cost,
            Self::Lam => weights.lam_cost,
            // `Programs` and leaves are both costed as literals: the corpus
            // root occupies one slot like any leaf, so weight profiles that
            // scale leaf cost (e.g. stitch) scale it too.
            Self::Programs => weights.sym_var_cost,
            Self::Leaf(o) => o.intrinsic_size(weights),
        }
    }

    fn as_var(&self) -> Option<egg::Var> {
        match self {
            Self::Leaf(o) => o.as_var(),
            _ => None,
        }
    }

    fn de_bruijn_index(&self) -> Option<u32> {
        match self {
            Self::Leaf(o) => o.de_bruijn_index(),
            _ => None,
        }
    }

    /// `Lam` binds its single body child; nothing else introduces a binder.
    fn binds_child(&self, j: usize) -> bool {
        matches!(self, Self::Lam) && j == 0
    }
}

impl<O: StitchOp> StitchOp for LambdaCalcDisc<O> {
    fn from_name(s: &str) -> Self {
        match s {
            "@" => Self::App,
            "lam" => Self::Lam,
            "programs" => Self::Programs,
            _ => Self::Leaf(O::from_name(s)),
        }
    }

    fn make_db_var(n: u32) -> Option<Self> {
        O::make_db_var(n).map(Self::Leaf)
    }
}

impl<O: StitchOp> Language for LambdaCalcLanguage<O> {
    type Discriminant = LambdaCalcDisc<O>;

    fn discriminant(&self) -> Self::Discriminant {
        match self {
            Self::Leaf(o) => LambdaCalcDisc::Leaf(o.clone()),
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
        Ok(match (LambdaCalcDisc::<O>::from_name(op), children.as_slice()) {
            (LambdaCalcDisc::App, &[f, a]) => Self::App([f, a]),
            (LambdaCalcDisc::Lam, &[b]) => Self::Lam([b]),
            (LambdaCalcDisc::Programs, _) => Self::Programs(children),
            (LambdaCalcDisc::Leaf(o), &[]) => Self::Leaf(o),
            // Multi-arity leaves get curried automatically so RecExpr/Pattern parsers
            // (which call `from_op` once per node) yield the appified shape directly.
            (LambdaCalcDisc::Leaf(_), _) => panic!("multi-arity application of {op:?} can't be a single LambdaCalcLanguage node; appify first"),
            (LambdaCalcDisc::App, _) | (LambdaCalcDisc::Lam, _) => panic!("{op:?} expects fixed arity, got {} children", children.len()),
        })
    }
}

impl<O: StitchOp> StitchLanguage for LambdaCalcLanguage<O> {
    fn is_programs_node(&self) -> bool {
        matches!(self, Self::Programs(_))
    }

    /// Custom sexp parser: walks the s-expression directly and emits curried
    /// `App` chains for any list-headed application form (e.g. `((a x) y)`).
    /// egg's `RecExpr::from_str` rejects head-as-list because `from_op` takes
    /// a string head — but in the lambda calculus a list head is a perfectly
    /// valid application, so we handle it ourselves.
    fn parse_program(s: &str) -> anyhow::Result<RecExpr<Self>> {
        let sexp = symbolic_expressions::parser::parse_str(s).map_err(|e| anyhow::anyhow!("parse {s:?}: {e}"))?;
        let mut out: RecExpr<Self> = RecExpr::default();
        sexp_to_lambda_calc::<O>(&sexp, &mut out)?;
        Ok(out)
    }

    fn parse_pattern_ast(s: &str) -> anyhow::Result<RecExpr<ENodeOrVar<Self>>> {
        let flat: egg::Pattern<OpChildrenLanguage<O>> = s.parse().map_err(|e| anyhow::anyhow!("parse pattern {s:?}: {e:?}"))?;
        Ok(appify_pattern_ast(&flat.ast))
    }

    fn display_recexpr(expr: &RecExpr<Self>) -> String {
        unappify_recexpr(expr).to_string()
    }
}

/// Walk a `Sexp` and emit it as a `LambdaCalcLanguage<O>` `RecExpr` using the
/// usual currying conventions:
/// - `(lam body)` → `Lam([body])`.
/// - `(@ f a)` → `App([f, a])`.
/// - `(programs …)` → `Programs(children)` (preserves the multi-child root).
/// - `(head a b c)` for any other shape (including list-headed `((f x) y) z`)
///   → curried `App` chain `App(App(App(head, a), b), c)`.
/// - Bare atoms parse as `Leaf(O::from_name(atom))`.
fn sexp_to_lambda_calc<O: StitchOp>(sexp: &symbolic_expressions::Sexp, out: &mut RecExpr<LambdaCalcLanguage<O>>) -> anyhow::Result<Id> {
    use symbolic_expressions::Sexp;
    match sexp {
        Sexp::Empty => anyhow::bail!("empty s-expression"),
        Sexp::String(atom) => Ok(out.add(LambdaCalcLanguage::Leaf(O::from_name(atom)))),
        Sexp::List(items) => {
            if items.is_empty() {
                anyhow::bail!("empty list");
            }
            // If the head is one of the structural keywords, dispatch on it.
            // Otherwise (atom or list head) curry-apply the remaining items.
            if let Sexp::String(head) = &items[0] {
                match head.as_str() {
                    "lam" => {
                        anyhow::ensure!(items.len() == 2, "lam expects 1 arg, got {}", items.len() - 1);
                        let body = sexp_to_lambda_calc::<O>(&items[1], out)?;
                        return Ok(out.add(LambdaCalcLanguage::Lam([body])));
                    }
                    "@" => {
                        anyhow::ensure!(items.len() == 3, "@ expects 2 args, got {}", items.len() - 1);
                        let f = sexp_to_lambda_calc::<O>(&items[1], out)?;
                        let a = sexp_to_lambda_calc::<O>(&items[2], out)?;
                        return Ok(out.add(LambdaCalcLanguage::App([f, a])));
                    }
                    "programs" => {
                        let kids: Result<Vec<Id>, _> = items[1..].iter().map(|c| sexp_to_lambda_calc::<O>(c, out)).collect();
                        return Ok(out.add(LambdaCalcLanguage::Programs(kids?)));
                    }
                    _ => {}
                }
            }
            // General application: curry-chain over all items left-to-right.
            let mut current = sexp_to_lambda_calc::<O>(&items[0], out)?;
            for arg in &items[1..] {
                let arg_id = sexp_to_lambda_calc::<O>(arg, out)?;
                current = out.add(LambdaCalcLanguage::App([current, arg_id]));
            }
            Ok(current)
        }
    }
}

/// Appify a flat `(op kids...)` head into `LambdaCalcLanguage`, inserting curried App
/// chains for ordinary multi-arity ops.
fn add_appified<N, O>(out: &mut RecExpr<N>, op: &O, kids: Vec<Id>, mut wrap: impl FnMut(&mut RecExpr<N>, LambdaCalcLanguage<O>) -> Id) -> Id
where
    N: egg::Language,
    O: StitchOp,
{
    match (LambdaCalcDisc::<O>::from_name(&op.to_string()), kids.len()) {
        (LambdaCalcDisc::App, 2) => wrap(out, LambdaCalcLanguage::App([kids[0], kids[1]])),
        (LambdaCalcDisc::Lam, 1) => wrap(out, LambdaCalcLanguage::Lam([kids[0]])),
        (LambdaCalcDisc::Programs, _) => wrap(out, LambdaCalcLanguage::Programs(kids)),
        (LambdaCalcDisc::Leaf(o), _) => {
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
fn unappify_recexpr<O: StitchOp>(src: &RecExpr<LambdaCalcLanguage<O>>) -> RecExpr<OpChildrenLanguage<O>> {
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
