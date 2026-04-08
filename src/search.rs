use crate::{lang::StitchLang, smc::StitchEgraph};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language};
use rand::Rng;
use rustc_hash::FxHashMap;
use std::collections::HashMap;

#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
    pub follow: Option<RevExpr<ENodeOrVar<StitchLang>>>,
    /// How many times each e-class is used in the fully-expanded corpus tree.
    pub usage_counts: FxHashMap<Id, usize>,
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
    pub fn expand_random(&mut self, shared: &SharedSearchData, verbose: bool) {
        // select a match weighted by usage count
        let extractor = egg::Extractor::new(&shared.egraph, egg::AstSize);
        let weights: Vec<usize> = self.matches.iter()
            .map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1))
            .collect();
        let total: usize = weights.iter().sum();
        let mut r = rand::rng().random_range(0..total);
        let mut match_idx = 0;
        for (i, &w) in weights.iter().enumerate() {
            if r < w {
                match_idx = i;
                break;
            }
            r -= w;
        }
        let m = &self.matches[match_idx];
        if verbose {
            let (_cost, minimal_term) = extractor.find_best(m.root_eclass);
            println!("Expanding on match at eclass {} with pattern {}", minimal_term, self.pattern);
        }
        // randomly select a subst within the match to base the expansion on
        let subst_idx = rand::rng().random_range(0..m.substs.len());
        let subst = &m.substs[subst_idx];

        // randomly select a var within the subst to expand (length of vars in subst is same as num vars in pattern)
        let var_idx = rand::rng().random_range(0..self.pattern.vars.len());
        if verbose {
            println!("Expanding variable {:?} in pattern {}", self.pattern.vars[var_idx], self.pattern);
        }
        let target_id = subst.vars[var_idx];
        
        if verbose {
            println!("Target eclass is represented by minimal term {}", extractor.find_best(target_id).1);
        }

        // consider reuse – look for vars in the subst that point to the same eclass
        let p_reuse = 0.5;
        if rand::rng().random_bool(p_reuse) {
            // optmization: could get rid of this allocation.
            let reuse_candidates = subst.vars.iter().enumerate().filter(|(idx, id)|  *idx != var_idx && **id == target_id).collect::<Vec<_>>();
            if reuse_candidates.len() > 0 {
                let candidate_idx = rand::rng().random_range(0..reuse_candidates.len());
                let candidate_var_idx = reuse_candidates[candidate_idx].0;
                
                self.reuse(var_idx, candidate_var_idx, shared);
                return
            }
        }

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
    /// Check if this particle's pattern is a valid prefix of the follow target.
    /// A partial pattern is consistent if every non-variable node matches the
    /// corresponding node in the target (same op/arity), no variable in the
    /// pattern corresponds to a position where the target has a variable
    /// (which would mean we over-expanded past the target), and shared variables
    /// (from reuse) map to the same subtree in the follow target.
    pub fn matches_follow(&self, follow: &RevExpr<ENodeOrVar<StitchLang>>) -> bool {
        /// Checks structural equality of two subtrees in the follow RevExpr.
        fn subtrees_equal(
            follow: &RevExpr<ENodeOrVar<StitchLang>>,
            a: Id,
            b: Id,
        ) -> bool {
            if a == b { return true; }
            match (&follow[a], &follow[b]) {
                (ENodeOrVar::Var(va), ENodeOrVar::Var(vb)) => va == vb,
                (ENodeOrVar::ENode(na), ENodeOrVar::ENode(nb)) => {
                    na.matches(nb)
                        && na.children.iter().zip(nb.children.iter())
                            .all(|(&ca, &cb)| subtrees_equal(follow, ca, cb))
                }
                _ => false,
            }
        }

        fn check(
            pattern: &RevExpr<ENodeOrVar<StitchLang>>,
            pid: Id,
            follow: &RevExpr<ENodeOrVar<StitchLang>>,
            fid: Id,
            var_bindings: &mut HashMap<egg::Var, Id>,
        ) -> bool {
            match &pattern[pid] {
                ENodeOrVar::Var(v) => {
                    // Shared variables must map to structurally equal follow subtrees
                    match var_bindings.entry(*v) {
                        std::collections::hash_map::Entry::Vacant(e) => { e.insert(fid); true }
                        std::collections::hash_map::Entry::Occupied(e) => subtrees_equal(follow, *e.get(), fid),
                    }
                }
                ENodeOrVar::ENode(p_node) => match &follow[fid] {
                    ENodeOrVar::Var(_) => false,
                    ENodeOrVar::ENode(f_node) => {
                        p_node.matches(f_node)
                            && p_node.children.iter().zip(f_node.children.iter())
                                .all(|(&pc, &fc)| check(pattern, pc, follow, fc, var_bindings))
                    }
                },
            }
        }
        let mut var_bindings = HashMap::new();
        check(&self.pattern.pattern, Id::from(0), follow, Id::from(0), &mut var_bindings)
    }

    pub fn expand(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        self.pattern.expand(var_idx, target);
        self.subset_matches(var_idx, target, shared);
    }

    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize, shared: &SharedSearchData) {
        self.pattern.reuse(var_idx, second_var_idx);
        self.subset_matches_reuse(var_idx, second_var_idx, shared);
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
    pub fn subset_matches_reuse(&mut self, var_idx: usize, second_var_idx: usize, shared: &SharedSearchData) {
        for m in &mut self.matches {
            // just filter down substs to ones where the two vars are same eclass
            let mut new_substs: Vec<Subst> = m.substs.clone().into_iter().filter(|subst| subst.vars[var_idx] == subst.vars[second_var_idx]).collect();
            // then cut the second one out of the subst
            for subst in new_substs.iter_mut() {
                subst.vars.remove(second_var_idx);
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

/// Computes how many times each e-class appears in the fully-expanded corpus tree.
/// Top-down pass: root gets count 1, then propagate to children of the best (first) enode.
pub fn compute_usage_counts(egraph: &StitchEgraph, root: Id) -> FxHashMap<Id, usize> {
    let mut counts = FxHashMap::<Id, usize>::default();
    counts.insert(root, 1);
    // Iterate in reverse id order (parents before children, since children have smaller ids)
    let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
    for i in (0..=max_id).rev() {
        let id = Id::from(i);
        let count = match counts.get(&id) {
            Some(&c) => c,
            None => continue,
        };
        if let Some(enode) = egraph[id].nodes.first() {
            for &child in &enode.children {
                *counts.entry(child).or_insert(0) += count;
            }
        }
    }
    counts
}
