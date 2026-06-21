//! Numeric rewrites declared via a `constant_folding: !<kind>` directive in a
//! rewrites file (parsed in [`crate::io::parse`]). The folding kinds collapse a
//! set of operators over numeric-literal leaves; they differ in which literals
//! they treat as foldable.
//!
//! The set of operators is configurable: a kind may be followed by a [`params`]
//! s-expression, e.g. `constant_folding: !integersarefloats (params (ops (+ * /
//! sin cos pi)))`. With no params it defaults to the binary arithmetic operators
//! [`DEFAULT_OPS`] (`+ - * /`). Recognised operators (see [`FoldOp`]): the binary
//! arithmetic `+ - * /`, the unary float functions `sin cos tan sqrt`, and the
//! nullary constants `pi e tau` (folding `pi` to a literal lets a later
//! `sin`/`cos` fold read it, replacing a hand-written `pi => 3.14…` rule).
//!
//! [`params`]: FoldingParams
//!
//! The kinds:
//! - `!integers` — folds integer-only operations (`(+ 1 2) => 3`); operations
//!   with a float operand are left untouched.
//! - `!floats` — folds operations with at least one float operand
//!   (`(+ 1.5 2.5) => 4.0`, `(+ 1 2.5) => 3.5`); integer-only operations are
//!   left untouched.
//! - `!integersarefloats` — like `!floats` but every numeric leaf is read as a
//!   float, so integer-only operations fold too (`(+ 1 2) => 3.0`); it also adds
//!   an `n => n.0` rewrite so each integer literal gains its float form
//!   (`1 => 1.0`).
//! - `!numbers` — `!integers` and `!floats` combined: the original behaviour,
//!   folding each operation in whichever domain its operands live.
//! - `!successors` — the inverse direction: expands an integer literal `n` into
//!   `(+ 1 (n-1))`, exposing the `(+ ?n 1)` form that successor-style rules
//!   consume. Bounded so it terminates (see [`successor_expansion_rewrite`]).

use crate::lang::{Op, OpChildrenLanguage, StitchLanguage};
use anyhow::anyhow;
use egg::{Analysis, Applier, EGraph, Id, Pattern, PatternAst, Rewrite, Subst, Symbol, Var};

/// The operators a folding directive collapses when it names no explicit list.
const DEFAULT_OPS: [&str; 4] = ["+", "-", "*", "/"];

/// Every operator constant folding can evaluate, grouped by arity so each `eval`
/// match stays exhaustive. Parsing, arity, and evaluation all live on this type.
#[derive(Clone, Copy)]
enum FoldOp {
    Binary(BinOp),
    Unary(UnOp),
    Const(ConstOp),
}

#[derive(Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy)]
enum UnOp {
    Sin,
    Cos,
    Tan,
    Sqrt,
}

#[derive(Clone, Copy)]
enum ConstOp {
    Pi,
    E,
    Tau,
}

impl FoldOp {
    /// Parses a surface symbol (`+`, `sin`, `pi`, …) into its operator.
    fn from_symbol(s: &str) -> Option<FoldOp> {
        Some(match s {
            "+" => FoldOp::Binary(BinOp::Add),
            "-" => FoldOp::Binary(BinOp::Sub),
            "*" => FoldOp::Binary(BinOp::Mul),
            "/" => FoldOp::Binary(BinOp::Div),
            "sin" => FoldOp::Unary(UnOp::Sin),
            "cos" => FoldOp::Unary(UnOp::Cos),
            "tan" => FoldOp::Unary(UnOp::Tan),
            "sqrt" => FoldOp::Unary(UnOp::Sqrt),
            "pi" => FoldOp::Const(ConstOp::Pi),
            "e" => FoldOp::Const(ConstOp::E),
            "tau" => FoldOp::Const(ConstOp::Tau),
            _ => return None,
        })
    }

    /// Number of operands; fixes the searcher's shape.
    fn arity(self) -> usize {
        match self {
            FoldOp::Binary(_) => 2,
            FoldOp::Unary(_) => 1,
            FoldOp::Const(_) => 0,
        }
    }

    /// Evaluates over `nums` (length `self.arity()`), returning the canonical leaf
    /// string or `None` when the operands are out of `mode`'s domain or the result
    /// is undefined / non-finite.
    fn eval(self, mode: FoldMode, nums: &[Num]) -> Option<String> {
        match self {
            FoldOp::Binary(op) => op.eval(mode, nums[0], nums[1]),
            FoldOp::Unary(op) => op.eval(mode, nums[0]),
            FoldOp::Const(op) => op.eval(mode),
        }
    }
}

impl BinOp {
    /// Binary arithmetic fold, governed by `mode`: which `(x, y)` pairs fold and
    /// whether they fold in the integer or the float domain.
    fn eval(self, mode: FoldMode, x: Num, y: Num) -> Option<String> {
        match mode {
            FoldMode::Integers => match (x, y) {
                (Num::Int(a), Num::Int(b)) => self.eval_int(a, b),
                _ => None,
            },
            FoldMode::Floats => match (x, y) {
                (Num::Int(_), Num::Int(_)) => None,
                _ => self.eval_float(x.as_f64(), y.as_f64()),
            },
            FoldMode::IntegersAreFloats => self.eval_float(x.as_f64(), y.as_f64()),
        }
    }

    /// Exact `i64` fold: `/` only when the divisor is non-zero and divides evenly;
    /// `+ - *` use checked arithmetic so overflow declines to fold.
    fn eval_int(self, a: i64, b: i64) -> Option<String> {
        let r = match self {
            BinOp::Add => a.checked_add(b)?,
            BinOp::Sub => a.checked_sub(b)?,
            BinOp::Mul => a.checked_mul(b)?,
            BinOp::Div => {
                if b == 0 || a % b != 0 {
                    return None;
                }
                a / b
            }
        };
        Some(r.to_string())
    }

    /// `f64` fold; declines on a non-finite result. Formatted with `{:?}` so the
    /// leaf always carries a decimal point (`4.0`, not `4`), keeping it a float.
    fn eval_float(self, a: f64, b: f64) -> Option<String> {
        let r = match self {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
        };
        r.is_finite().then(|| format!("{r:?}"))
    }
}

impl UnOp {
    /// Unary float-function fold. Skipped in `Integers` mode, and in `Floats` mode
    /// on an integer operand — mirroring the binary "needs a float" gate. Declines
    /// on a non-finite result.
    fn eval(self, mode: FoldMode, x: Num) -> Option<String> {
        match mode {
            FoldMode::Integers => return None,
            FoldMode::Floats if matches!(x, Num::Int(_)) => return None,
            _ => {}
        }
        let v = x.as_f64();
        let r = match self {
            UnOp::Sin => v.sin(),
            UnOp::Cos => v.cos(),
            UnOp::Tan => v.tan(),
            UnOp::Sqrt => v.sqrt(),
        };
        r.is_finite().then(|| format!("{r:?}"))
    }
}

impl ConstOp {
    /// Nullary numeric constant → its `f64` literal. Skipped in `Integers` mode,
    /// where introducing a float would escape the domain.
    fn eval(self, mode: FoldMode) -> Option<String> {
        if matches!(mode, FoldMode::Integers) {
            return None;
        }
        let r = match self {
            ConstOp::Pi => std::f64::consts::PI,
            ConstOp::E => std::f64::consts::E,
            ConstOp::Tau => std::f64::consts::TAU,
        };
        r.is_finite().then(|| format!("{r:?}"))
    }
}

/// Which numeric literals a folding kind treats as foldable. See the module docs.
#[derive(Clone, Copy, Debug)]
pub enum FoldMode {
    /// Fold only integer-on-integer operations; leave anything with a float operand.
    Integers,
    /// Fold operations with at least one float operand; leave integer-only ones.
    Floats,
    /// Read every numeric leaf as a float so integer-only operations fold too, and
    /// rewrite each integer literal `n` to its float form `n.0`.
    IntegersAreFloats,
}

/// A numeric literal: an integer when it round-trips through `i64`, otherwise a float.
#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    /// Parses a leaf's display string as a number, preferring `i64`.
    fn parse(s: &str) -> Option<Num> {
        if let Ok(i) = s.parse::<i64>() { Some(Num::Int(i)) } else { s.parse::<f64>().ok().map(Num::Float) }
    }

    fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

/// Parameters for a `constant_folding` directive, written as an s-expression and
/// read with [`OpChildrenLanguage`] so it shares the corpus surface syntax, e.g.
/// `(params (ops (+ * / -)))`. Each child of `params` is a `(<key> <value>…)`
/// node; new keys can be added without changing the directive grammar.
#[derive(Default)]
pub struct FoldingParams {
    /// Operators to fold (the `ops` key); empty means "use [`DEFAULT_OPS`]".
    pub ops: Vec<String>,
}

impl FoldingParams {
    /// Parses a `(params …)` block. A blank string is the default (no keys set).
    pub fn parse(s: &str) -> anyhow::Result<FoldingParams> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(FoldingParams::default());
        }
        let expr = OpChildrenLanguage::<Op>::parse_program(s)?;
        let nodes = expr.as_ref();
        let root = nodes.last().expect("a parsed RecExpr has at least one node");
        if root.op.to_string() != "params" {
            return Err(anyhow!("constant_folding: parameters must be a (params …) block"));
        }
        let mut params = FoldingParams::default();
        for &key in &root.children {
            let node = &nodes[usize::from(key)];
            match node.op.to_string().as_str() {
                "ops" => params.ops = read_ops(nodes, &node.children)?,
                other => return Err(anyhow!("constant_folding: unknown param key {other:?} (supported: ops)")),
            }
        }
        Ok(params)
    }
}

/// Reads the operators in an `(ops (<op> <op>…))` value: the head operator plus
/// its direct children, e.g. `(ops (+ * / -))` → `[+, *, /, -]`. The `ops` key
/// must wrap a single group node, and every operator in it must be a leaf
/// (0-arity) symbol — anything else is a malformed list.
fn read_ops(nodes: &[OpChildrenLanguage<Op>], children: &[Id]) -> anyhow::Result<Vec<String>> {
    let [group] = children else {
        return Err(anyhow!("constant_folding: ops takes a single (op …) group"));
    };
    let group = &nodes[usize::from(*group)];
    let mut ops = vec![group.op.to_string()];
    for &child in &group.children {
        let leaf = &nodes[usize::from(child)];
        if !leaf.children.is_empty() {
            return Err(anyhow!("constant_folding: operator {:?} must be a leaf symbol", leaf.op.to_string()));
        }
        ops.push(leaf.op.to_string());
    }
    Ok(ops)
}

/// Builds the folding rewrites for `mode` over `ops` (an empty slice defaults to
/// [`DEFAULT_OPS`]): one searcher per operator — `(op ?a ?b)` binary, `(op ?a)`
/// unary, the bare symbol for a nullary constant — paired with an [`OpFold`]
/// applier, plus, for [`FoldMode::IntegersAreFloats`], an `?n => n.0` rewrite
/// ([`IntAsFloat`]) that exposes each integer literal's float form. Errors on an
/// unrecognised operator.
pub fn folding_rewrites<L, A>(mode: FoldMode, ops: &[String]) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let vars: [Var; 2] = ["?a".parse().unwrap(), "?b".parse().unwrap()];
    let ops: Vec<&str> = if ops.is_empty() { DEFAULT_OPS.to_vec() } else { ops.iter().map(String::as_str).collect() };
    let mut rewrites: Vec<Rewrite<L, A>> = Vec::new();
    for sym in ops {
        let op = FoldOp::from_symbol(sym).ok_or_else(|| anyhow!("constant_folding: unknown operator {sym:?}"))?;
        // Integer-only folding can't evaluate the float functions/constants.
        if matches!(mode, FoldMode::Integers) && op.arity() != 2 {
            continue;
        }
        let pat = match op.arity() {
            0 => sym.to_string(),
            1 => format!("({sym} ?a)"),
            _ => format!("({sym} ?a ?b)"),
        };
        let searcher: Pattern<L> = L::parse_pattern_ast(&pat)?.into();
        let applier = OpFold { op, args: vars[..op.arity()].to_vec(), mode };
        rewrites.push(Rewrite::new(format!("fold-{sym}-{mode:?}"), searcher, applier).map_err(|e| anyhow!("{e}"))?);
    }
    if matches!(mode, FoldMode::IntegersAreFloats) {
        let searcher: Pattern<L> = L::parse_pattern_ast("?n")?.into();
        rewrites.push(Rewrite::new("int-as-float", searcher, IntAsFloat).map_err(|e| anyhow!("{e}"))?);
    }
    Ok(rewrites)
}

/// Applier that folds an [`FoldOp`] application once every operand e-class has a
/// numeric-literal representative: it evaluates the operator (see [`FoldOp::eval`])
/// and unions the resulting literal leaf into the matched e-class.
#[derive(Clone)]
struct OpFold {
    op: FoldOp,
    /// The searcher's pattern variables, one per operand (empty for a constant).
    args: Vec<Var>,
    mode: FoldMode,
}

/// Applier behind `!integersarefloats`: adds the float form `n.0` to any e-class
/// holding an integer literal `n` and unions it in, so integer literals
/// participate in float folding (and `1` unifies with `1.0`). Formatted with
/// `{:?}` to match [`BinOp::eval_float`].
#[derive(Clone)]
struct IntAsFloat;

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for IntAsFloat {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, _subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(Num::Int(n)) = eclass_number(egraph, eclass) else { return vec![] };
        let id = egraph.add(L::from_op(&format!("{:?}", n as f64), vec![]).expect("float leaf is a valid 0-arity op"));
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for OpFold {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let mut nums = Vec::with_capacity(self.args.len());
        for v in &self.args {
            let Some(n) = eclass_number(egraph, subst[*v]) else { return vec![] };
            nums.push(n);
        }
        let Some(folded) = self.op.eval(self.mode, &nums) else { return vec![] };
        let id = egraph.add(L::from_op(&folded, vec![]).expect("numeric leaf is a valid 0-arity op"));
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

/// Finds a numeric-literal representative of an e-class: a childless e-node
/// whose display parses as a number.
fn eclass_number<L: StitchLanguage, A: Analysis<L>>(egraph: &EGraph<L, A>, id: Id) -> Option<Num> {
    egraph[id].nodes.iter().find_map(|n| if n.children().is_empty() { Num::parse(&n.to_string()) } else { None })
}

/// Builds the `!successors` expansion rewrite: any integer literal `n > floor`
/// gains the equivalent form `(+ 1 (n-1))`, exposing the `(+ ?n 1)` shape that
/// successor-style rules (e.g. `repeat_unroll`) consume — the expansion direction
/// that [`number_folding_rewrites`] (which only collapses) cannot provide.
///
/// Bounded by `floor`: each step lowers the value and stops once `n <= floor`,
/// so it terminates rather than descending forever (the failure mode of a naive
/// `?n => (+ 1 (- ?n 1))` static rule). Non-integer and small literals are left
/// untouched.
pub fn successor_expansion_rewrite<L, A>(floor: i64) -> anyhow::Result<Rewrite<L, A>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let searcher: Pattern<L> = L::parse_pattern_ast("?n")?.into();
    Rewrite::new("expand-successor", searcher, SuccessorExpand { floor }).map_err(|e| anyhow!("{e}"))
}

/// Applier behind `!successors`: adds `(+ 1 (n-1))` to a literal e-class (when
/// `n > floor`) and unions it in. Fires on integer literals and on floats with
/// an integer value (e.g. `4.0`), producing the successor form in the same
/// numeric domain — `(+ 1 3)` vs `(+ 1.0 3.0)`. The term is built via
/// [`StitchLanguage::parse_program`] so it takes the correct shape in both the
/// flat (`OpChildren`) and curried (`LambdaCalc`) families.
#[derive(Clone)]
struct SuccessorExpand {
    floor: i64,
}

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for SuccessorExpand {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, _subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let (n, is_float) = match eclass_number(egraph, eclass) {
            Some(Num::Int(n)) => (n, false),
            // An integer-valued float (`2.0`, `4.0`) unrolls too, staying float.
            Some(Num::Float(f)) if f.is_finite() && f.fract() == 0.0 => (f as i64, true),
            _ => return vec![],
        };
        if n <= self.floor {
            return vec![];
        }
        let expr_str = if is_float { format!("(+ {:?} {:?})", 1.0_f64, (n - 1) as f64) } else { format!("(+ 1 {})", n - 1) };
        let expr = L::parse_program(&expr_str).expect("successor expression parses");
        let id = egraph.add_expr(&expr);
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{Op, OpChildrenLanguage, StitchAnalysis, Weights};

    type L = OpChildrenLanguage<Op>;

    /// Builds an egraph from `expr`, saturates it with `rules`, and returns the
    /// min-cost extraction of the root.
    fn fold_with(expr: &str, rules: Vec<Rewrite<L, StitchAnalysis>>) -> String {
        let mut egraph: EGraph<L, StitchAnalysis> = EGraph::new(StitchAnalysis::new(Weights::default()));
        let root = egraph.add_expr(&expr.parse().unwrap());
        egraph.rebuild();
        let runner: egg::Runner<L, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(Weights::default())).with_egraph(egraph).run(&rules);
        let extractor = egg::Extractor::new(&runner.egraph, crate::cost::WeightedSize { weights: Weights::default() });
        extractor.find_best(runner.egraph.find(root)).1.to_string()
    }

    /// `fold_with` under a single [`FoldMode`] over `ops` (`&[]` → [`DEFAULT_OPS`]).
    fn fold_mode_ops(expr: &str, mode: FoldMode, ops: &[String]) -> String {
        fold_with(expr, folding_rewrites::<L, StitchAnalysis>(mode, ops).unwrap())
    }

    /// `fold_with` under a single [`FoldMode`] with the default operators.
    fn fold_mode(expr: &str, mode: FoldMode) -> String {
        fold_mode_ops(expr, mode, &[])
    }

    /// `fold_with` under the combined `!numbers` rewrites (`Integers` + `Floats`).
    fn fold(expr: &str) -> String {
        let mut rules = folding_rewrites::<L, StitchAnalysis>(FoldMode::Integers, &[]).unwrap();
        rules.extend(folding_rewrites::<L, StitchAnalysis>(FoldMode::Floats, &[]).unwrap());
        fold_with(expr, rules)
    }

    /// True iff `a` and `b` land in the same e-class after saturating with `mode`.
    fn same_class_mode(a: &str, b: &str, mode: FoldMode) -> bool {
        let mut egraph: EGraph<L, StitchAnalysis> = EGraph::new(StitchAnalysis::new(Weights::default()));
        let (ia, ib) = (egraph.add_expr(&a.parse().unwrap()), egraph.add_expr(&b.parse().unwrap()));
        egraph.rebuild();
        let rules = folding_rewrites::<L, StitchAnalysis>(mode, &[]).unwrap();
        let runner: egg::Runner<L, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(Weights::default())).with_egraph(egraph).run(&rules);
        runner.egraph.find(ia) == runner.egraph.find(ib)
    }

    #[test]
    fn folds_integer_arithmetic() {
        assert_eq!(fold("(+ 1 2)"), "3");
        assert_eq!(fold("(- 5 8)"), "-3");
        assert_eq!(fold("(* 4 6)"), "24");
        assert_eq!(fold("(/ 12 3)"), "4");
    }

    #[test]
    fn folds_nested_arithmetic() {
        assert_eq!(fold("(* (+ 1 2) (- 10 4))"), "18");
    }

    #[test]
    fn folds_float_arithmetic() {
        // Float results keep their decimal point: 4.0, not 4.
        assert_eq!(fold("(+ 1.5 2.5)"), "4.0");
        assert_eq!(fold("(/ 7.0 2.0)"), "3.5");
    }

    #[test]
    fn skips_inexact_and_undefined() {
        // Inexact integer division and division by zero are left unfolded.
        assert_eq!(fold("(/ 7 2)"), "(/ 7 2)");
        assert_eq!(fold("(/ 7 0)"), "(/ 7 0)");
    }

    #[test]
    fn leaves_symbolic_terms_alone() {
        assert_eq!(fold("(+ x 2)"), "(+ x 2)");
    }

    #[test]
    fn integers_mode_folds_ints_leaves_floats() {
        assert_eq!(fold_mode("(+ 1 2)", FoldMode::Integers), "3");
        // Any float operand is out of scope; the operation is left intact.
        assert_eq!(fold_mode("(+ 1.5 2.5)", FoldMode::Integers), "(+ 1.5 2.5)");
        assert_eq!(fold_mode("(+ 1 2.5)", FoldMode::Integers), "(+ 1 2.5)");
        // No `n => n.0`, so `1` and `1.0` stay distinct.
        assert!(!same_class_mode("1", "1.0", FoldMode::Integers));
    }

    #[test]
    fn floats_mode_folds_floats_leaves_ints() {
        // Float and mixed operations fold to a float-formatted literal...
        assert_eq!(fold_mode("(+ 1.5 2.5)", FoldMode::Floats), "4.0");
        assert_eq!(fold_mode("(+ 1 2.5)", FoldMode::Floats), "3.5");
        // ...but pure-integer operations are left intact.
        assert_eq!(fold_mode("(+ 1 2)", FoldMode::Floats), "(+ 1 2)");
    }

    #[test]
    fn integers_are_floats_folds_everything_in_float() {
        // Integer-only operations fold, computed in float: 3.0, not 3.
        assert_eq!(fold_mode("(+ 1 2)", FoldMode::IntegersAreFloats), "3.0");
        // Each integer literal gains its float form, so `1` unifies with `1.0`.
        assert!(same_class_mode("1", "1.0", FoldMode::IntegersAreFloats));
    }

    /// Builds an owned operator list from string slices, for the folding tests.
    fn ops(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn folds_constants_and_unary_functions() {
        assert_eq!(fold_mode_ops("pi", FoldMode::IntegersAreFloats, &ops(&["pi"])), "3.141592653589793");
        assert_eq!(fold_mode_ops("(sqrt 4)", FoldMode::IntegersAreFloats, &ops(&["sqrt"])), "2.0");
        // `pi` folds to a literal, then the `-` and `cos` fold over it in turn —
        // the chain a hand-written exact-trig rule replaces, here for any angle.
        assert_eq!(fold_mode_ops("(cos (- pi pi))", FoldMode::IntegersAreFloats, &ops(&["cos", "-", "pi"])), "1.0");
    }

    #[test]
    fn integer_mode_skips_float_operators() {
        // `sqrt` can't yield an integer, so integer folding skips it (left intact)...
        assert_eq!(fold_mode_ops("(sqrt 4)", FoldMode::Integers, &ops(&["sqrt"])), "(sqrt 4)");
        // ...while the binary operators in the same list still fold.
        assert_eq!(fold_mode_ops("(+ 1 2)", FoldMode::Integers, &ops(&["+", "pi"])), "3");
    }

    #[test]
    fn unknown_operator_is_an_error() {
        assert!(folding_rewrites::<L, StitchAnalysis>(FoldMode::IntegersAreFloats, &ops(&["wat"])).is_err());
    }

    #[test]
    fn parses_params_ops() {
        // The `ops` group is the head operator plus its direct children.
        assert_eq!(FoldingParams::parse("(params (ops (+ * /)))").unwrap().ops, ops(&["+", "*", "/"]));
        assert_eq!(FoldingParams::parse("(params (ops (cos sin pi)))").unwrap().ops, ops(&["cos", "sin", "pi"]));
        // Blank → no operators (the caller falls back to the default set).
        assert!(FoldingParams::parse("  ").unwrap().ops.is_empty());
    }

    #[test]
    fn rejects_malformed_params() {
        assert!(FoldingParams::parse("(ops (+ *))").is_err()); // missing the `params` wrapper
        assert!(FoldingParams::parse("(params (nope (+)))").is_err()); // unknown key
        assert!(FoldingParams::parse("(params (ops (+ (* 1 2))))").is_err()); // non-leaf operator
    }

    /// Runs the `!successors` expander (floor `floor`) on a single literal to
    /// saturation and returns the resulting e-graph plus the literal's id.
    fn expand(literal: &str, floor: i64) -> (EGraph<L, StitchAnalysis>, Id) {
        let mut egraph: EGraph<L, StitchAnalysis> = EGraph::new(StitchAnalysis::new(Weights::default()));
        let id = egraph.add_expr(&literal.parse().unwrap());
        egraph.rebuild();
        let rules = vec![successor_expansion_rewrite::<L, StitchAnalysis>(floor).unwrap()];
        let runner: egg::Runner<L, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(Weights::default())).with_egraph(egraph).run(&rules);
        let id = runner.egraph.find(id);
        (runner.egraph, id)
    }

    /// Asserts `expr` parses into the same e-class as `id` in `egraph`.
    fn equiv(egraph: &mut EGraph<L, StitchAnalysis>, id: Id, expr: &str) -> bool {
        let other = egraph.add_expr(&expr.parse().unwrap());
        egraph.rebuild();
        egraph.find(id) == egraph.find(other)
    }

    #[test]
    fn expands_literal_down_to_floor() {
        // 6 unrolls the whole successor ladder; the `(+ ?n 1)` form repeat_unroll
        // needs is reachable via `add_comm` from each `(+ 1 k)`.
        let (mut egraph, six) = expand("6", 1);
        assert!(equiv(&mut egraph, six, "(+ 1 5)"));
        let two = egraph.add_expr(&"2".parse().unwrap());
        egraph.rebuild();
        assert!(equiv(&mut egraph, two, "(+ 1 1)"));
    }

    #[test]
    fn successor_expansion_respects_floor() {
        // floor = 1, so `1` is never rewritten into `(+ 1 0)` (terminates here).
        let (mut egraph, one) = expand("1", 1);
        assert!(!equiv(&mut egraph, one, "(+ 1 0)"));
    }

    #[test]
    fn successor_expansion_fires_on_integer_valued_floats() {
        // `4.0` unrolls like `4`, staying in floats: (+ 1.0 3.0).
        let (mut egraph, four) = expand("4.0", 1);
        assert!(equiv(&mut egraph, four, "(+ 1.0 3.0)"));
        // A non-integer float is left alone.
        let (mut egraph, half) = expand("2.5", 1);
        assert!(!equiv(&mut egraph, half, "(+ 1.0 1.5)"));
    }
}
