use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::{MatchAtEClass, Subst, identity_matches};
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use egg::{ENodeOrVar, Id, Language, Symbol};
use rand::Rng;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;

/// A deterministic move taken at a search node: either expanding a pattern variable
/// with a specific enode shape, or unifying two existing variables.
#[derive(Debug, Clone)]
pub enum Action {
    Expand { var_idx: usize, op: Symbol, arity: usize },
    Reuse { keep: usize, drop: usize },
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Expand { var_idx, op, arity } => write!(f, "expand #{} := {}/{}", var_idx, op, arity),
            Action::Reuse { keep, drop } => write!(f, "reuse #{} = #{}", keep, drop),
        }
    }
}

/// Shared read-only context passed to all search operations.
#[derive(Debug)]
pub struct SharedSearchData {
    pub egraph: StitchEgraph,
    /// Follow pattern: particles whose pattern isn't a valid prefix get zero
    /// weight at the resample step.
    pub follow: Option<RevExpr<ENodeOrVar<StitchLang>>>,
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
    pub fn expand_random(&mut self, shared: &SharedSearchData, verbose: bool) {
        let match_idx = if shared.weight_by_usage {
            let mut weights: Vec<f64> = self.matches.iter().map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1) as f64).collect();
            let weights_acc = crate::smc::normalize_and_accumulate(&mut weights);
            crate::smc::weighted_choice(&weights_acc)
        } else {
            rand::rng().random_range(0..self.matches.len())
        };
        let m = &self.matches[match_idx];
        let extractor = if verbose { Some(egg::Extractor::new(&shared.egraph, egg::AstSize)) } else { None };
        if let Some(ref ext) = extractor {
            let (_cost, minimal_term) = ext.find_best(m.root_eclass);
            println!("Expanding on match at eclass {} with pattern {}", minimal_term, self.pattern);
        }
        let subst_idx = rand::rng().random_range(0..m.substs.len());
        let subst = &m.substs[subst_idx];

        let var_idx = rand::rng().random_range(0..self.pattern.vars.len());
        if verbose {
            println!("Expanding variable {:?} in pattern {}", self.pattern.vars[var_idx], self.pattern);
        }
        let target_id = subst.vars[var_idx];

        if let Some(ref ext) = extractor {
            println!("Target eclass is represented by minimal term {}", ext.find_best(target_id).1);
        }

        if rand::rng().random_bool(shared.p_reuse) {
            let reuse_candidates = subst.vars.iter().enumerate().filter(|(idx, id)| *idx != var_idx && **id == target_id).collect::<Vec<_>>();
            if !reuse_candidates.is_empty() {
                let candidate_idx = rand::rng().random_range(0..reuse_candidates.len());
                let candidate_var_idx = reuse_candidates[candidate_idx].0;
                self.reuse(var_idx, candidate_var_idx);
                return;
            }
        }

        let target_eclass = &shared.egraph[target_id];
        let node_idx = rand::rng().random_range(0..target_eclass.len());
        let target_node = &target_eclass.nodes[node_idx];

        self.expand(var_idx, target_node, shared);
    }

    /// Check if this particle's pattern is a valid prefix of the follow target.
    pub fn matches_follow(&self, follow: &RevExpr<ENodeOrVar<StitchLang>>) -> bool {
        let mut var_bindings = HashMap::new();
        crate::follow::check_follow(&self.pattern.pattern, Id::from(0), follow, Id::from(0), &mut var_bindings)
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

    /// Creates the initial search state: a single-variable pattern matching every e-class.
    pub fn new(shared: &SharedSearchData) -> Self {
        Self {
            pattern: Pattern::single_var(),
            matches: identity_matches(&shared.egraph),
        }
    }

    /// Enumerates every successor state reachable in one `expand` or `reuse` step.
    ///
    /// Expansion candidates: for each variable, collect every distinct `(op, arity)`
    /// pair appearing as an enode in any bound e-class across all matches, then produce
    /// one child per shape. Reuse candidates: for every pair `(i, j)` with `i < j`,
    /// emit a child if some match has `subst.vars[i] == subst.vars[j]`. Children whose
    /// match set becomes empty after filtering are dropped.
    pub fn enumerate_successors(&self, shared: &SharedSearchData) -> Vec<(Action, SearchState)> {
        let mut out = Vec::new();

        for var_idx in 0..self.pattern.vars.len() {
            let mut seen: FxHashSet<(Symbol, usize)> = FxHashSet::default();
            let mut shapes: Vec<StitchLang> = Vec::new();
            for m in &self.matches {
                for subst in &m.substs {
                    let eclass = &shared.egraph[subst.vars[var_idx]];
                    for node in &eclass.nodes {
                        let key = (node.op, node.children.len());
                        if seen.insert(key) {
                            shapes.push(node.clone());
                        }
                    }
                }
            }
            for shape in shapes {
                let mut child = self.clone();
                child.expand(var_idx, &shape, shared);
                if !child.matches.is_empty() {
                    out.push((Action::Expand { var_idx, op: shape.op, arity: shape.children.len() }, child));
                }
            }
        }

        let n = self.pattern.vars.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let unifiable = self.matches.iter().any(|m| m.substs.iter().any(|s| s.vars[i] == s.vars[j]));
                if unifiable {
                    let mut child = self.clone();
                    child.reuse(i, j);
                    if !child.matches.is_empty() {
                        out.push((Action::Reuse { keep: i, drop: j }, child));
                    }
                }
            }
        }

        out
    }
}

/// Parses the shared-context fields out of CLI args, computes usage counts, and
/// returns the initial corpus size alongside the populated `SharedSearchData`.
pub fn setup_search(egraph: StitchEgraph, root: Id, args: &crate::Args) -> (SharedSearchData, usize) {
    let follow_expr: Option<RevExpr<ENodeOrVar<StitchLang>>> = args.follow.as_deref().map(|s| s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e)));
    let usage_counts = compute_usage_counts(&egraph, root);
    let shared = SharedSearchData {
        egraph,
        follow: follow_expr,
        weight_by_usage: args.weight_by_usage,
        usage_counts,
        p_reuse: args.p_reuse,
        check_slow: args.check_slow,
    };
    let initial = SearchState::new(&shared);
    let original_size = crate::cost::compute_size(&shared.egraph, root, &initial, shared.check_slow);
    (shared, original_size)
}

impl std::fmt::Display for SearchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState {{ pattern: {}, matches: {} }}", self.pattern, self.matches.len())
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
