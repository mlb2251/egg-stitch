use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};

/// A partially-built pattern over `StitchLang`, tracking which nodes are open variables.
///
/// Canonical-form invariant: for every `k`, every `Id` in `vars[k]` holds
/// `ENodeOrVar::Var(egg::Var::from(k as u32))` in the tree — i.e. the tree's var names
/// match their DFS first-appearance order exactly. `expand` and `reuse` preserve this
/// by actively rewriting affected `Var(n)` leaves, so `pattern.to_string()` is itself
/// canonical: two alpha-equivalent patterns render identically.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub pattern: RevExpr<ENodeOrVar<StitchLang>>,
    pub vars: Vec<Vec<Id>>, // vars[k] = all RecExpr ids holding Var(k)
}

impl Pattern {
    /// Creates the initial `?#0` pattern: a single variable.
    pub fn single_var() -> Self {
        let e: RevExpr<ENodeOrVar<StitchLang>> = RevExpr::new(vec![ENodeOrVar::Var(egg::Var::from(0))]);
        Pattern { pattern: e, vars: vec![vec![0.into()]] }
    }

    /// Expands the variable at `var_idx` with `target`. New children are inserted
    /// at list positions `var_idx..var_idx+k`; any vars that previously followed
    /// `var_idx` shift right and get their in-tree `Var(n)` leaves rewritten to
    /// match their new position, so the canonical-form invariant is preserved.
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang) {
        let var_positions = self.vars.remove(var_idx);
        assert!(matches!(self.pattern[var_positions[0]], ENodeOrVar::Var(_)), "Attempting to expand a non-var");
        let num_children = target.len();

        // Shift names of trailing vars: a var currently at post-removal index p
        // will end up at post-insertion index p + num_children, so rename its leaves.
        // (Skip the no-op case num_children == 1 where indices don't move.)
        if num_children != 1 {
            for p in var_idx..self.vars.len() {
                let shifted = ENodeOrVar::Var(egg::Var::from((p + num_children) as u32));
                for &id in &self.vars[p] {
                    self.pattern[id] = shifted.clone();
                }
            }
        }

        // Build the new enode with freshly-named Var children at positions var_idx..var_idx+k.
        let mut new_node = target.clone();
        for j in 0..num_children {
            let new_var = ENodeOrVar::Var(egg::Var::from((var_idx + j) as u32));
            self.pattern.nodes.push(new_var);
            let new_id = Id::from(self.pattern.nodes.len() - 1);
            new_node.children[j] = new_id;
            self.vars.insert(var_idx + j, vec![new_id]);
        }

        // Replace each position of the expanded var with the new enode. If the var
        // had multiple positions (from a prior reuse), all parents share the same
        // children via the RecExpr DAG.
        for var_id in var_positions {
            self.pattern[var_id] = ENodeOrVar::ENode(new_node.clone());
        }
    }

    /// Unifies two variables. The lower-indexed one is kept; the higher one is
    /// removed and its positions are rewritten to the kept var's name. Trailing
    /// vars shift left by one and have their leaves renamed accordingly. Args may
    /// be passed in either order.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        assert_ne!(var_idx, second_var_idx, "reuse requires two distinct vars");
        let (keep_idx, drop_idx) = if var_idx < second_var_idx { (var_idx, second_var_idx) } else { (second_var_idx, var_idx) };

        let keep_name = ENodeOrVar::Var(egg::Var::from(keep_idx as u32));
        for var_id in &self.vars[drop_idx] {
            self.pattern[*var_id] = keep_name.clone();
        }
        let drop_ids = self.vars[drop_idx].clone();
        self.vars[keep_idx].extend(drop_ids);
        self.vars.remove(drop_idx);

        // Shift names of trailing vars down by one.
        for p in drop_idx..self.vars.len() {
            let shifted = ENodeOrVar::Var(egg::Var::from(p as u32));
            for &id in &self.vars[p] {
                self.pattern[id] = shifted.clone();
            }
        }
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

#[cfg(test)]
mod tests {
    use crate::lang::Op;

    use super::*;
    use egg::Symbol;

    /// Build a StitchLang enode with `arity` placeholder children. `expand` overwrites
    /// the children, so the dummy Ids here are never read.
    fn op(name: &str, arity: usize) -> StitchLang {
        StitchLang {
            op: Op::Sym(Symbol::from(name)),
            children: vec![Id::from(0); arity],
        }
    }

    /// Asserts the canonical-form invariant: every id in `vars[k]` holds `Var(k)`,
    /// and nothing in `vars` is non-Var.
    fn assert_vars_canonical(p: &Pattern) {
        for (k, ids) in p.vars.iter().enumerate() {
            let expected = egg::Var::from(k as u32);
            for id in ids {
                match &p.pattern[*id] {
                    ENodeOrVar::Var(v) => assert_eq!(*v, expected, "vars[{}] = {:?}: expected {:?}, got {:?}", k, ids, expected, v),
                    other => panic!("vars[{}] contains non-Var: {:?}", k, other),
                }
            }
        }
    }

    #[test]
    fn single_var_is_canonical() {
        let p = Pattern::single_var();
        assert_eq!(p.vars.len(), 1);
        assert_eq!(p.to_string(), "?#0");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_fresh_var_binary() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        assert_eq!(p.vars.len(), 2);
        assert_eq!(p.to_string(), "(+ ?#0 ?#1)");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_nested_left_first() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(0, &op("-", 2)); // (+ (- ?#0 ?#1) ?#2)
        assert_eq!(p.to_string(), "(+ (- ?#0 ?#1) ?#2)");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_right_keeps_earlier_vars_first() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#2))");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_ternary() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("f", 3));
        assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#2)");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_adjacent() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.reuse(0, 1); // (+ ?#0 ?#0)
        assert_eq!(p.to_string(), "(+ ?#0 ?#0)");
        assert_eq!(p.vars.len(), 1);
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_normalizes_reversed_args() {
        let mut p1 = Pattern::single_var();
        p1.expand(0, &op("+", 2));
        p1.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        p1.reuse(0, 2);

        let mut p2 = Pattern::single_var();
        p2.expand(0, &op("+", 2));
        p2.expand(1, &op("*", 2));
        p2.reuse(2, 0); // reversed

        assert_eq!(p1.to_string(), "(+ ?#0 (* ?#1 ?#0))");
        assert_eq!(p1.to_string(), p2.to_string());
        assert_eq!(p1.vars.len(), p2.vars.len());
        assert_vars_canonical(&p1);
        assert_vars_canonical(&p2);

        // Downstream expansion should agree: "var 0" must mean the same thing in both.
        p1.expand(0, &op("h", 1));
        p2.expand(0, &op("h", 1));
        assert_eq!(p1.to_string(), p2.to_string());
        assert_vars_canonical(&p1);
        assert_vars_canonical(&p2);
    }

    #[test]
    fn reuse_with_intervening_var() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("f", 3)); // (f ?#0 ?#1 ?#2)
        p.reuse(0, 2); // (f ?#0 ?#1 ?#0)
        assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#0)");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_reused_var_preserves_dag_sharing() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.reuse(0, 1); // (+ ?#0 ?#0)
        assert_eq!(p.vars.len(), 1);
        p.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))
        assert_eq!(p.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);

        // The two new vars must each have a single RecExpr slot (DAG sharing),
        // not one per tree occurrence.
        assert_eq!(p.vars[0].len(), 1);
        assert_eq!(p.vars[1].len(), 1);
    }

    #[test]
    fn expand_then_reuse_across_structure() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        p.reuse(1, 2); // (+ ?#0 (* ?#1 ?#1))
        assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
    }

    #[test]
    fn to_string_distinguishes_non_equivalent_shapes() {
        let mut a = Pattern::single_var();
        a.expand(0, &op("+", 2));
        a.reuse(0, 1); // (+ ?#0 ?#0)
        a.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))

        let mut b = Pattern::single_var();
        b.expand(0, &op("+", 2));
        b.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) ?#2)
        b.expand(2, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#2 ?#3))

        assert_ne!(a.to_string(), b.to_string());
        assert_eq!(a.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(b.to_string(), "(+ (* ?#0 ?#1) (* ?#2 ?#3))");
        assert_vars_canonical(&a);
        assert_vars_canonical(&b);
    }
}
