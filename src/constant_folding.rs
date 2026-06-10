//! Numeric rewrites declared via a `constant_folding: !<kind>` directive in a
//! rewrites file (parsed in [`crate::io::parse`]). Supported kinds:
//! - `!numbers` — folds the binary arithmetic operators `+ - * /` over
//!   numeric-literal leaves (collapses `(+ 1 2)` into `3`).
//! - `!successors` — the inverse direction: expands an integer literal `n` into
//!   `(+ 1 (n-1))`, exposing the `(+ ?n 1)` form that successor-style rules
//!   consume. Bounded so it terminates (see [`successor_expansion_rewrite`]).

use crate::lang::StitchLanguage;
use anyhow::anyhow;
use egg::{Analysis, Applier, EGraph, Id, Pattern, PatternAst, Rewrite, Subst, Symbol, Var};

/// The arithmetic operators folded by `!numbers`.
const ARITH_OPS: [&str; 4] = ["+", "-", "*", "/"];

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

/// Builds the `!numbers` folding rewrites: one per [`ARITH_OPS`] operator,
/// each a `(op ?a ?b)` searcher paired with a [`NumberFold`] applier.
pub fn number_folding_rewrites<L, A>() -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let (a, b): (Var, Var) = ("?a".parse().unwrap(), "?b".parse().unwrap());
    ARITH_OPS
        .iter()
        .map(|&op| {
            let searcher: Pattern<L> = L::parse_pattern_ast(&format!("({op} ?a ?b)"))?.into();
            Rewrite::new(format!("fold-{op}"), searcher, NumberFold { op, a, b }).map_err(|e| anyhow!("{e}"))
        })
        .collect()
}

/// Applier that folds `(op a b)` when both operands have numeric-literal
/// representatives: it adds the resulting literal leaf and unions it into the
/// matched e-class.
#[derive(Clone)]
struct NumberFold {
    op: &'static str,
    a: Var,
    b: Var,
}

impl NumberFold {
    /// Applies `self.op` to two numbers. Stays in `i64` when both operands are
    /// integers (folding `/` only when exact); otherwise computes in `f64`.
    /// Returns the canonical leaf string, or `None` when the result is undefined
    /// (division by zero), inexact integer division, or not finite.
    fn eval(&self, x: Num, y: Num) -> Option<String> {
        if let (Num::Int(a), Num::Int(b)) = (x, y) {
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
            return Some(r.to_string());
        }
        let (a, b) = (x.as_f64(), y.as_f64());
        let r = match self.op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            _ => return None,
        };
        r.is_finite().then(|| r.to_string())
    }
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

/// Applier behind `!successors`: adds `(+ 1 (n-1))` to an integer-literal e-class
/// (when `n > floor`) and unions it in. The `(+ 1 …)` term is built via
/// [`StitchLanguage::parse_program`] so it takes the correct shape in both the
/// flat (`OpChildren`) and curried (`LambdaCalc`) families.
#[derive(Clone)]
struct SuccessorExpand {
    floor: i64,
}

impl<L: StitchLanguage, A: Analysis<L>> Applier<L, A> for SuccessorExpand {
    fn apply_one(&self, egraph: &mut EGraph<L, A>, eclass: Id, _subst: &Subst, _ast: Option<&PatternAst<L>>, _name: Symbol) -> Vec<Id> {
        let Some(Num::Int(n)) = eclass_number(egraph, eclass) else { return vec![] };
        if n <= self.floor {
            return vec![];
        }
        let expr = L::parse_program(&format!("(+ 1 {})", n - 1)).expect("successor expression parses");
        let id = egraph.add_expr(&expr);
        if egraph.union(eclass, id) { vec![id] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{Op, OpChildrenLanguage, StitchAnalysis, Weights};

    type L = OpChildrenLanguage<Op>;

    /// Builds an egraph from `expr`, runs the `!numbers` folding rewrites to
    /// saturation, and returns the min-cost extraction of the root.
    fn fold(expr: &str) -> String {
        let mut egraph: EGraph<L, StitchAnalysis> = EGraph::new(StitchAnalysis::new(Weights::default()));
        let root = egraph.add_expr(&expr.parse().unwrap());
        egraph.rebuild();
        let rules = number_folding_rewrites::<L, StitchAnalysis>().unwrap();
        let runner: egg::Runner<L, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(Weights::default())).with_egraph(egraph).run(&rules);
        let extractor = egg::Extractor::new(&runner.egraph, crate::cost::WeightedSize { weights: Weights::default() });
        extractor.find_best(runner.egraph.find(root)).1.to_string()
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
        assert_eq!(fold("(+ 1.5 2.5)"), "4");
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
}
