//! Numeric rewrites declared via a `constant_folding: !<kind>` directive in a
//! rewrites file (parsed in [`crate::io::parse`]). The folding kinds collapse
//! the binary arithmetic operators `+ - * /` over numeric-literal leaves; they
//! differ in which literals they treat as foldable:
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

use crate::lang::StitchLanguage;
use anyhow::anyhow;
use egg::{Analysis, Applier, EGraph, Id, Pattern, PatternAst, Rewrite, Subst, Symbol, Var};

/// The binary arithmetic operators the folding kinds collapse.
const ARITH_OPS: [&str; 4] = ["+", "-", "*", "/"];

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

/// Builds the folding rewrites for `mode`: one `(op ?a ?b)` searcher per
/// [`ARITH_OPS`] operator paired with a [`NumberFold`] applier, plus — for
/// [`FoldMode::IntegersAreFloats`] — an `?n => n.0` rewrite ([`IntAsFloat`])
/// that exposes each integer literal's float form.
pub fn folding_rewrites<L, A>(mode: FoldMode) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let (a, b): (Var, Var) = ("?a".parse().unwrap(), "?b".parse().unwrap());
    let mut rewrites: Vec<Rewrite<L, A>> = ARITH_OPS
        .iter()
        .map(|&op| {
            let searcher: Pattern<L> = L::parse_pattern_ast(&format!("({op} ?a ?b)"))?.into();
            Rewrite::new(format!("fold-{op}-{mode:?}"), searcher, NumberFold { op, a, b, mode }).map_err(|e| anyhow!("{e}"))
        })
        .collect::<anyhow::Result<_>>()?;
    if matches!(mode, FoldMode::IntegersAreFloats) {
        let searcher: Pattern<L> = L::parse_pattern_ast("?n")?.into();
        rewrites.push(Rewrite::new("int-as-float", searcher, IntAsFloat).map_err(|e| anyhow!("{e}"))?);
    }
    Ok(rewrites)
}

/// Applier that folds `(op a b)` when both operands have numeric-literal
/// representatives: it adds the resulting literal leaf and unions it into the
/// matched e-class. Which `(x, y)` pairs fold is governed by [`FoldMode`].
#[derive(Clone)]
struct NumberFold {
    op: &'static str,
    a: Var,
    b: Var,
    mode: FoldMode,
}

impl NumberFold {
    /// Applies `self.op` per [`self.mode`](FoldMode), returning the canonical
    /// leaf string or `None` when the pair is out of the mode's domain or the
    /// result is undefined (division by zero, inexact integer division, or not
    /// finite).
    fn eval(&self, x: Num, y: Num) -> Option<String> {
        match self.mode {
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
    fn eval_int(&self, a: i64, b: i64) -> Option<String> {
        let r = match self.op {
            "+" => a.checked_add(b)?,
            "-" => a.checked_sub(b)?,
            "*" => a.checked_mul(b)?,
            "/" => {
                if b == 0 || a % b != 0 {
                    return None;
                }
                a / b
            }
            _ => return None,
        };
        Some(r.to_string())
    }

    /// `f64` fold; declines on a non-finite result. Formatted with `{:?}` so the
    /// leaf always carries a decimal point (`4.0`, not `4`), keeping it a float.
    fn eval_float(&self, a: f64, b: f64) -> Option<String> {
        let r = match self.op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            _ => return None,
        };
        r.is_finite().then(|| format!("{r:?}"))
    }
}

/// Applier behind `!integersarefloats`: adds the float form `n.0` to any e-class
/// holding an integer literal `n` and unions it in, so integer literals
/// participate in float folding (and `1` unifies with `1.0`). Formatted with
/// `{:?}` to match [`NumberFold::eval_float`].
#[derive(Clone)]
struct IntAsFloat;

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for IntAsFloat {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, _subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(Num::Int(n)) = eclass_number(egraph, eclass) else { return vec![] };
        let id = egraph.add(L::from_op(&format!("{:?}", n as f64), vec![]).expect("float leaf is a valid 0-arity op"));
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

/// Canonical float string: round to 6 decimal places (snapping float noise such
/// as `cos(pi/2) = 6e-17` to 0 and collapsing values that agree to 6 decimals
/// onto one literal) and format as a plain decimal so it unifies with the
/// corpus's literal leaves.
fn round6(x: f64) -> String {
    let r = (x * 1e6).round() / 1e6;
    let r = if r == 0.0 { 0.0 } else { r }; // normalise -0.0
    format!("{r:?}")
}

/// Applier behind `!round6`: adds the 6-decimal-rounded form of any numeric
/// literal e-class and unions it in, so noise-equal / near-equal numbers (e.g. a
/// fold's `1e-16` and `0`, or `0.7853981…` and `0.785398`) share one literal.
#[derive(Clone)]
struct RoundLiteral;

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for RoundLiteral {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, _subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(num) = eclass_number(egraph, eclass) else { return vec![] };
        let id = egraph.add(L::from_op(&round6(num.as_f64()), vec![]).expect("float leaf is a valid 0-arity op"));
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

/// Builds the `!round6` rewrite (see [`RoundLiteral`]).
pub fn round6_rewrite<L, A>() -> anyhow::Result<Rewrite<L, A>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let searcher: Pattern<L> = L::parse_pattern_ast("?n")?.into();
    Rewrite::new("round6", searcher, RoundLiteral).map_err(|e| anyhow!("{e}"))
}

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for NumberFold {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(x) = eclass_number(egraph, subst[self.a]) else { return vec![] };
        let Some(y) = eclass_number(egraph, subst[self.b]) else { return vec![] };
        let Some(folded) = self.eval(x, y) else { return vec![] };
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

/// Find an `(M s r x y)` e-node in `id` and return its four field e-classes.
fn matrix_fields<L: StitchLanguage, A: Analysis<L>>(egraph: &EGraph<L, A>, id: Id) -> Option<[Id; 4]> {
    egraph[id].nodes.iter().find_map(|n| {
        let c = n.children();
        (n.to_string() == "M" && c.len() == 4).then(|| [c[0], c[1], c[2], c[3]])
    })
}

/// Read a scalar matrix-field e-class as an f64 *only if it is already a constant
/// leaf* — a numeric literal or `pi`. No recursion into `+ - * /`/trig: those are
/// left to `constant_folding` (which, with a `pi => 3.14…` rule, reduces angle
/// expressions to literals first). This keeps the matmul fold O(1) per field and
/// makes it fire only once both inputs are fully-constant matrices.
fn eval_scalar<L: StitchLanguage, A: Analysis<L>>(egraph: &EGraph<L, A>, id: Id) -> Option<f64> {
    egraph[id].nodes.iter().find_map(|n| {
        if !n.children().is_empty() {
            return None;
        }
        let s = n.to_string();
        if s == "pi" { Some(std::f64::consts::PI) } else { s.parse::<f64>().ok() }
    })
}

/// Applier behind `!matmul`: when `(matmul A B)` has both children resolvable to
/// concrete affine matrices `(M s r x y)` (fields evaluable to f64), computes the
/// composite — `A` applied first then `B`, per the drawings `Transform` semantics
/// (`p ↦ R(r)·(s·p) + (x,y)`) — and unions the single literal result matrix into
/// the e-class. Emits *only* the reduced `(M …)` (no intermediate `+`/`*` nodes),
/// so composing transforms never materialises coordinate-arithmetic trees.
#[derive(Clone)]
struct MatmulFold {
    a: Var,
    b: Var,
}

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for MatmulFold {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(ma) = matrix_fields(egraph, subst[self.a]) else { return vec![] };
        let Some(mb) = matrix_fields(egraph, subst[self.b]) else { return vec![] };
        let v = [ma[0], ma[1], ma[2], ma[3], mb[0], mb[1], mb[2], mb[3]].map(|id| eval_scalar(egraph, id));
        let [Some(s1), Some(r1), Some(x1), Some(y1), Some(s2), Some(r2), Some(x2), Some(y2)] = v else { return vec![] };
        // A first, then B: scale multiplies, rotation adds, and A's translation is
        // carried through B's scale+rotation before B's translation is added.
        let (s, r) = (s1 * s2, r1 + r2);
        let (cos, sin) = (r2.cos(), r2.sin());
        let x = x2 + s2 * (cos * x1 - sin * y1);
        let y = y2 + s2 * (sin * x1 + cos * y1);
        if ![s, r, x, y].iter().all(|f| f.is_finite()) {
            return vec![];
        }
        let mut leaf = |v: f64| egraph.add(L::from_op(&format!("{v:?}"), vec![]).expect("float leaf is a valid 0-arity op"));
        let (sl, rl, xl, yl) = (leaf(s), leaf(r), leaf(x), leaf(y));
        let Ok(m_node) = L::from_op("M", vec![sl, rl, xl, yl]) else { return vec![] };
        let m = egraph.add(m_node);
        if egraph.union(eclass, m) { vec![m] } else { vec![] }
    }
}

/// Builds the `!matmul` fold rewrite (see [`MatmulFold`]).
pub fn matmul_fold_rewrite<L, A>() -> anyhow::Result<Rewrite<L, A>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let (a, b): (Var, Var) = ("?a".parse().unwrap(), "?b".parse().unwrap());
    let searcher: Pattern<L> = L::parse_pattern_ast("(matmul ?a ?b)")?.into();
    Rewrite::new("matmul-fold", searcher, MatmulFold { a, b }).map_err(|e| anyhow!("{e}"))
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

    /// `fold_with` under a single [`FoldMode`].
    fn fold_mode(expr: &str, mode: FoldMode) -> String {
        fold_with(expr, folding_rewrites::<L, StitchAnalysis>(mode).unwrap())
    }

    /// `fold_with` under the combined `!numbers` rewrites (`Integers` + `Floats`).
    fn fold(expr: &str) -> String {
        let mut rules = folding_rewrites::<L, StitchAnalysis>(FoldMode::Integers).unwrap();
        rules.extend(folding_rewrites::<L, StitchAnalysis>(FoldMode::Floats).unwrap());
        fold_with(expr, rules)
    }

    /// True iff `a` and `b` land in the same e-class after saturating with `mode`.
    fn same_class_mode(a: &str, b: &str, mode: FoldMode) -> bool {
        let mut egraph: EGraph<L, StitchAnalysis> = EGraph::new(StitchAnalysis::new(Weights::default()));
        let (ia, ib) = (egraph.add_expr(&a.parse().unwrap()), egraph.add_expr(&b.parse().unwrap()));
        egraph.rebuild();
        let rules = folding_rewrites::<L, StitchAnalysis>(mode).unwrap();
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
