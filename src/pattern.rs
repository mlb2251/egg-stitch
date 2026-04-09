use crate::lang::StitchLang;
use egg::{Language, ENodeOrVar, Id};
use crate::revexpr::RevExpr;

/// A partially-built pattern over `StitchLang`, tracking which nodes are open variables.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub pattern: RevExpr<ENodeOrVar<StitchLang>>,
    pub vars: Vec<Vec<Id>>, // each var is a vector of Ids that index into the locations in the pattern where that var is used
    pub max_var: u32, // not same as arity because can expand away a var
}

impl Pattern {
    /// Creates the initial #?0 pattern which is just a single var
    pub fn single_var() -> Self {
        // annoyingly parsing "#?0" doesn't create a ENodeOrVar::Var it creates an ENodeOrVar::ENode
        let e: RevExpr<ENodeOrVar<StitchLang>>  = RevExpr::new(vec![ENodeOrVar::Var(egg::Var::from(0))]);
        Pattern {
            pattern: e,
            vars: vec![vec![0.into()]],
            max_var: 0,
        }
    }

    /// Creates a new variable with a fresh name and adds it to the pattern
    pub fn new_var(&mut self) -> Id {
        self.max_var += 1;
        let arg_node = ENodeOrVar::Var(egg::Var::from(self.max_var));
        self.pattern.nodes.push(arg_node);
        let new_id = Id::from(self.pattern.nodes.len()-1);
        self.vars.push(vec![new_id]);
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
        assert!(matches!(self.pattern[var[0]], ENodeOrVar::Var(_)), "Attempting to expand a non-var");
        for var_id in var {
            // could optimze
            self.pattern[var_id] = ENodeOrVar::ENode(new_node.clone());
        }
    }

    /// Merges `second_var_idx` into `var_idx`, replacing all occurrences with the first var's node.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        for var_id in &self.vars[second_var_idx] {
            self.pattern[*var_id] = self.pattern[self.vars[var_idx][0]].clone();
        }
        let second_var_ids = self.vars[second_var_idx].clone();
        self.vars[var_idx].extend(second_var_ids);
        self.vars.remove(second_var_idx);
    }
}


impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pattern)
    }
}