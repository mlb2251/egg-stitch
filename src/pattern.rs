use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};

/// A partially-built pattern with a canonical-form invariant: `vars[k]` always
/// holds `Var(k)` in the tree, so `to_string()` is canonical and alpha-equivalent
/// patterns render identically.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub pattern: RevExpr<ENodeOrVar<StitchLang>>,
    pub vars: Vec<Vec<Id>>, // vars[k] = all tree ids holding Var(k)
}

impl Pattern {
    pub fn single_var() -> Self {
        let e: RevExpr<ENodeOrVar<StitchLang>> = RevExpr::new(vec![ENodeOrVar::Var(egg::Var::from(0))]);
        Pattern { pattern: e, vars: vec![vec![0.into()]] }
    }

    /// Replaces the variable at `var_idx` with `target`, inserting fresh child
    /// vars at `var_idx..var_idx+k` and renaming trailing vars to preserve
    /// canonical form.
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang) {
        let var_positions = self.vars.remove(var_idx);
        assert!(matches!(self.pattern[var_positions[0]], ENodeOrVar::Var(_)), "Attempting to expand a non-var");
        let num_children = target.len();

        // self.vars indices shift on remove/insert but tree Var(k) names don't.
        // Net shift for trailing vars is num_children - 1; only == 1 cancels exactly.
        if num_children != 1 {
            for p in var_idx..self.vars.len() {
                let shifted = ENodeOrVar::Var(egg::Var::from((p + num_children) as u32));
                for &id in &self.vars[p] {
                    self.pattern[id] = shifted.clone();
                }
            }
        }

        let mut new_node = target.clone();
        for j in 0..num_children {
            let new_var = ENodeOrVar::Var(egg::Var::from((var_idx + j) as u32));
            self.pattern.nodes.push(new_var);
            let new_id = Id::from(self.pattern.nodes.len() - 1);
            new_node.children[j] = new_id;
            self.vars.insert(var_idx + j, vec![new_id]);
        }

        for var_id in var_positions {
            self.pattern[var_id] = ENodeOrVar::ENode(new_node.clone());
        }
    }

    /// Unifies two variables (in either order). Keeps the lower-indexed one,
    /// removes the higher, and shifts trailing var names down by one.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        assert_ne!(var_idx, second_var_idx);
        let (keep_idx, drop_idx) = (var_idx.min(second_var_idx), var_idx.max(second_var_idx));

        let keep_name = ENodeOrVar::Var(egg::Var::from(keep_idx as u32));
        for var_id in &self.vars[drop_idx] {
            self.pattern[*var_id] = keep_name.clone();
        }
        let drop_ids = self.vars[drop_idx].clone();
        self.vars[keep_idx].extend(drop_ids);
        self.vars.remove(drop_idx);

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
    use super::*;
    use egg::Symbol;

    fn op(name: &str, arity: usize) -> StitchLang {
        StitchLang { op: Symbol::from(name), children: vec![Id::from(0); arity] }
    }

    fn assert_vars_canonical(p: &Pattern) {
        for (k, ids) in p.vars.iter().enumerate() {
            let expected = egg::Var::from(k as u32);
            for id in ids {
                match &p.pattern[*id] {
                    ENodeOrVar::Var(v) => assert_eq!(*v, expected, "vars[{}]: expected {:?}, got {:?}", k, expected, v),
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
    fn expand_binary() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        assert_eq!(p.to_string(), "(+ ?#0 ?#1)");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_left_then_right() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        p.expand(0, &op("-", 2));
        assert_eq!(p.to_string(), "(+ (- ?#0 ?#1) ?#2)");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_right_keeps_earlier_vars_first() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        p.expand(1, &op("*", 2));
        assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#2))");
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_adjacent() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        p.reuse(0, 1);
        assert_eq!(p.to_string(), "(+ ?#0 ?#0)");
        assert_eq!(p.vars.len(), 1);
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_normalizes_reversed_args() {
        let mut p1 = Pattern::single_var();
        p1.expand(0, &op("+", 2));
        p1.expand(1, &op("*", 2));
        p1.reuse(0, 2);

        let mut p2 = Pattern::single_var();
        p2.expand(0, &op("+", 2));
        p2.expand(1, &op("*", 2));
        p2.reuse(2, 0);

        assert_eq!(p1.to_string(), "(+ ?#0 (* ?#1 ?#0))");
        assert_eq!(p1.to_string(), p2.to_string());
        assert_vars_canonical(&p1);
        assert_vars_canonical(&p2);
    }

    #[test]
    fn reuse_with_intervening_var() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("f", 3));
        p.reuse(0, 2);
        assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#0)");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_reused_var_preserves_dag_sharing() {
        let mut p = Pattern::single_var();
        p.expand(0, &op("+", 2));
        p.reuse(0, 1);
        p.expand(0, &op("*", 2));
        assert_eq!(p.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
        assert_eq!(p.vars[0].len(), 1);
        assert_eq!(p.vars[1].len(), 1);
    }

    #[test]
    fn to_string_distinguishes_non_equivalent_shapes() {
        let mut a = Pattern::single_var();
        a.expand(0, &op("+", 2));
        a.reuse(0, 1);
        a.expand(0, &op("*", 2));

        let mut b = Pattern::single_var();
        b.expand(0, &op("+", 2));
        b.expand(0, &op("*", 2));
        b.expand(2, &op("*", 2));

        assert_eq!(a.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(b.to_string(), "(+ (* ?#0 ?#1) (* ?#2 ?#3))");
        assert_ne!(a.to_string(), b.to_string());
        assert_vars_canonical(&a);
        assert_vars_canonical(&b);
    }
}
