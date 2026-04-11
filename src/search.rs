use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use egg::Language;
use rand::Rng;

#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
    /// Probability of attempting variable reuse during expansion.
    pub p_reuse: f64,
}

#[derive(Debug, Clone)]
pub struct SearchState {
    pub pattern: Pattern,
    // each match represents a different eclass at which `pattern` can be rooted
    pub matches: Vec<MatchAtEClass>,
}

impl SearchState {
    /// Randomly selects a match and variable, then expands or reuses the variable.
    pub fn expand_random(&mut self, shared: &SharedSearchData) {
        let match_idx = rand::rng().random_range(0..self.matches.len());
        let m = &self.matches[match_idx];
        let subst_idx = rand::rng().random_range(0..m.substs.len());
        let subst = &m.substs[subst_idx];

        let var_idx = rand::rng().random_range(0..self.pattern.vars.len());
        let target_id = subst.vars[var_idx];

        if rand::rng().random_bool(shared.p_reuse) {
            let reuse_candidates: Vec<usize> = subst.vars.iter().enumerate().filter(|(idx, id)| *idx != var_idx && **id == target_id).map(|(idx, _)| idx).collect();
            if !reuse_candidates.is_empty() {
                let candidate_idx = rand::rng().random_range(0..reuse_candidates.len());
                let candidate_var_idx = reuse_candidates[candidate_idx];
                self.reuse(var_idx, candidate_var_idx);
                return;
            }
        }

        let target_eclass = &shared.egraph[target_id];
        let node_idx = rand::rng().random_range(0..target_eclass.len());
        let target_node = &target_eclass.nodes[node_idx];

        self.expand(var_idx, target_node, shared);
    }

    /// Expands the pattern at `var_idx` with `target` and filters matches accordingly.
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        self.pattern.expand(var_idx, target);
        self.subset_matches(var_idx, target, shared);
    }

    /// Merges two pattern variables and filters matches to those where both point to the same e-class.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        self.pattern.reuse(var_idx, second_var_idx);
        self.subset_matches_reuse(var_idx, second_var_idx);
    }

    /// Updates all matches by transforming each substitution via the given closure,
    /// which may produce zero or more new substitutions per input. Removes matches
    /// with no remaining substitutions.
    fn update_matches(&mut self, mut f: impl FnMut(&Subst, &mut Vec<Subst>)) {
        for m in &mut self.matches {
            let mut new_substs: Vec<Subst> = vec![];
            for subst in &m.substs {
                f(subst, &mut new_substs);
            }
            m.substs = new_substs;
        }
        self.matches.retain(|m| !m.substs.is_empty());
    }

    /// Filters matches to those where `var_idx` can be expanded with `target`, updating substitutions.
    pub fn subset_matches(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        self.update_matches(|subst, out| {
            let var_id = subst.vars[var_idx];
            let var_eclass = &shared.egraph[var_id];
            for node in &var_eclass.nodes {
                if node.matches(target) {
                    let mut new_subst = subst.clone();
                    new_subst.vars.remove(var_idx);
                    for child_id in &node.children {
                        new_subst.vars.push(*child_id);
                    }
                    out.push(new_subst);
                }
            }
        });
    }

    /// Filters matches to those where `var_idx` and `second_var_idx` point to the same e-class.
    pub fn subset_matches_reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        self.update_matches(|subst, out| {
            if subst.vars[var_idx] == subst.vars[second_var_idx] {
                let mut new_subst = subst.clone();
                new_subst.vars.remove(second_var_idx);
                out.push(new_subst);
            }
        });
    }
}


impl std::fmt::Display for SearchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState {{ pattern: {}, matches: {} }}", self.pattern, self.matches.len())
    }
}

impl SearchState {
    pub fn new(shared: &SharedSearchData) -> Self {
        Self {
            pattern: Pattern::single_var(),
            matches: identity_matches(&shared.egraph)
        }
    }
}
