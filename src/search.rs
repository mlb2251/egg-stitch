use crate::{lang::StitchLang, smc::StitchEgraph};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use rand::Rng;

#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
    pub follow: Option<RevExpr<ENodeOrVar<StitchLang>>>,
}

#[derive(Debug, Clone)]
pub struct MatchAtEClass {
    pub root_eclass: egg::Id,
    // variables[i][j] represents the j'th variable in the i'th way to match the pattern
    pub substs: Vec<Subst>,
}

#[derive(Debug, Clone)]
pub struct Subst {
    pub vars: Vec<Id>,
}

#[derive(Debug, Clone)]
pub struct SearchState {
    pub pattern: Pattern,
    // each match represents a different eclass at which `pattern` can be rooted
    pub matches: Vec<MatchAtEClass>,
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

        self.expand(var_idx, &target_node, shared);
    }
    /// Check if this particle's pattern is a valid prefix of the follow target.
    /// A partial pattern is consistent if every non-variable node matches the
    /// corresponding node in the target (same op/arity), and no variable in the
    /// pattern corresponds to a position where the target has a variable
    /// (which would mean we over-expanded past the target).
    pub fn matches_follow(&self, follow: &RevExpr<ENodeOrVar<StitchLang>>) -> bool {
        fn check(
            pattern: &RevExpr<ENodeOrVar<StitchLang>>,
            pid: Id,
            follow: &RevExpr<ENodeOrVar<StitchLang>>,
            fid: Id,
        ) -> bool {
            match &pattern[pid] {
                // A hole in the pattern matches anything in the target
                ENodeOrVar::Var(_) => true,
                ENodeOrVar::ENode(p_node) => match &follow[fid] {
                    // Pattern has structure where target has a var — over-expanded
                    ENodeOrVar::Var(_) => false,
                    ENodeOrVar::ENode(f_node) => {
                        p_node.matches(f_node)
                            && p_node.children.iter().zip(f_node.children.iter())
                                .all(|(&pc, &fc)| check(pattern, pc, follow, fc))
                    }
                },
            }
        }
        check(&self.pattern.pattern, Id::from(0), follow, Id::from(0))
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


impl MatchAtEClass {
    pub fn identity_match(c: egg::Id) -> Self {
        Self {
            root_eclass: c,
            substs: vec![Subst { vars: vec![c] }],
        }
    }
}

fn identity_matches(egraph: &StitchEgraph) -> Vec<MatchAtEClass> {
    egraph.classes().map(|c| MatchAtEClass::identity_match(c.id)).collect()
}

impl SearchState {
    pub fn new(shared: &SharedSearchData) -> Self {
        Self {
            pattern: Pattern::single_var(),
            matches: identity_matches(&shared.egraph),
        }
    }
}
