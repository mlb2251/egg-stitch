use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use egg::Language;
use rand::Rng;

#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
}

#[derive(Debug, Clone)]
pub struct SearchState {
    pub pattern: Pattern,
    // each match represents a different eclass at which `pattern` can be rooted
    pub matches: Vec<MatchAtEClass>
}

impl SearchState {
    pub fn expand_random(&mut self, shared: &SharedSearchData) {
        // randomly select a match to base the expansion on
        let match_idx = rand::rng().random_range(0..self.matches.len());
        let m = &self.matches[match_idx];
        // randomly select a subst within the match to base the expansion on
        let subst_idx = rand::rng().random_range(0..m.substs.len());
        let subst = &m.substs[subst_idx];

        // randomly select a var within the subst to expand (length of vars in subst is same as num vars in pattern)
        let var_idx = rand::rng().random_range(0..self.pattern.vars.len());
        let target_id = subst.vars[var_idx];
        let target_eclass = &shared.egraph[target_id];

        // randomly select an enode within the eclass to expand
        let node_idx = rand::rng().random_range(0..target_eclass.len());
        let target_node = &target_eclass.nodes[node_idx];

        self.expand(var_idx, target_node, shared);
    }
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        self.pattern.expand(var_idx, target);
        self.subset_matches(var_idx, target, shared);
    }
    pub fn subset_matches(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        for m in &mut self.matches {
            let mut new_substs: Vec<Subst> = vec![];
            for subst in &m.substs {
                let var_id = subst.vars[var_idx];
                let var_eclass = &shared.egraph[var_id];
                for node in &var_eclass.nodes {
                    if node.matches(target) { // this is egg::Language::matches
                        let mut new_subst = subst.clone();
                        new_subst.vars.remove(var_idx); // pop the expanded var
                        for child_id in &node.children {
                            new_subst.vars.push(*child_id);
                        }
                        new_substs.push(new_subst);
                    }
                }
            }
            m.substs = new_substs;
        }
        // filter out empty matches
        self.matches.retain(|m| !m.substs.is_empty());
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
