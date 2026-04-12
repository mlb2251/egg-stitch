use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use egg::{Id, Language};
use rand::Rng;
use rustc_hash::FxHashMap;

#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
    /// Probability of attempting variable reuse during expansion.
    pub p_reuse: f64,
    /// Enable slow rewrite check (assert fast == slow computation).
    pub check_slow: bool,
    /// Whether to weight match selection by usage count during expansion.
    pub weight_by_usage: bool,
    /// How many times each e-class is used in the fully-expanded corpus tree.
    pub usage_counts: FxHashMap<Id, usize>,
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
        let match_idx = if shared.weight_by_usage {
            let mut weights: Vec<f64> = self.matches.iter().map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1) as f64).collect();
            crate::smc::normalize_and_accumulate(&mut weights);
            crate::smc::weighted_choice(&weights)
        } else {
            rand::rng().random_range(0..self.matches.len())
        };
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
    /// Mirrors `Pattern::expand`: drops the old var from `subst.vars` and inserts the new
    /// child eclass ids at positions `var_idx..var_idx+k`, keeping substs aligned with
    /// the pattern's DFS-ordered vars list.
    pub fn subset_matches(&mut self, var_idx: usize, target: &StitchLang, shared: &SharedSearchData) {
        self.update_matches(|subst, out| {
            let var_id = subst.vars[var_idx];
            let var_eclass = &shared.egraph[var_id];
            for node in &var_eclass.nodes {
                if node.matches(target) {
                    let mut new_subst = subst.clone();
                    new_subst.vars.remove(var_idx);
                    for (j, child_id) in node.children.iter().enumerate() {
                        new_subst.vars.insert(var_idx + j, *child_id);
                    }
                    out.push(new_subst);
                }
            }
        });
    }

    /// Filters matches to those where `var_idx` and `second_var_idx` point to the same e-class.
    /// Mirrors `Pattern::reuse`: keeps the lower-indexed var and removes the higher one,
    /// so substs stay aligned with the pattern regardless of caller argument order.
    pub fn subset_matches_reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        let drop_idx = var_idx.max(second_var_idx);
        self.update_matches(|subst, out| {
            if subst.vars[var_idx] == subst.vars[second_var_idx] {
                let mut new_subst = subst.clone();
                new_subst.vars.remove(drop_idx);
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

/// Computes how many times each e-class appears in the fully-expanded corpus tree.
/// Top-down pass: root gets count 1, then propagate to children of the best (first) enode.
pub fn compute_usage_counts(egraph: &StitchEgraph, root: Id) -> FxHashMap<Id, usize> {
    let mut counts = FxHashMap::<Id, usize>::default();
    counts.insert(root, 1);
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
