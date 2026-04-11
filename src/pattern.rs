use crate::lang::StitchLang;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub pattern: RevExpr<ENodeOrVar<StitchLang>>,
    pub vars: Vec<Id>,
    pub max_var: u32, // not same as arity because can expand away a var
}

impl Pattern {
    /// Creates the initial #?0 pattern which is just a single var
    pub fn single_var() -> Self {
        // annoyingly parsing "#?0" doesn't create a ENodeOrVar::Var it creates an ENodeOrVar::ENode
        let e: RevExpr<ENodeOrVar<StitchLang>> = RevExpr::new(vec![ENodeOrVar::Var(egg::Var::from(0))]);
        Pattern { pattern: e, vars: vec![0.into()], max_var: 0 }
    }

    /// Creates a new variable with a fresh name and adds it to the pattern
    pub fn new_var(&mut self) -> Id {
        self.max_var += 1;
        let arg_node = ENodeOrVar::Var(egg::Var::from(self.max_var));
        self.pattern.nodes.push(arg_node);
        let new_id = Id::from(self.pattern.nodes.len() - 1);
        self.vars.push(new_id);
        new_id
    }

    /// Expands the pattern at the given Id with the given node
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang) {
        let var = self.vars.remove(var_idx);
        let mut new_node = target.clone();
        let num_vars = new_node.len();
        for j in 0..num_vars {
            new_node.children[j] = self.new_var();
        }
        assert!(matches!(self.pattern[var], ENodeOrVar::Var(_)), "Attempting to expand a non-var");
        self.pattern[var] = ENodeOrVar::ENode(new_node);
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pattern)
    }
}