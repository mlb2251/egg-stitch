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
///
/// Scope-indexed meta-vars: `var_depth[k]` is the number of `lam` ancestors enclosing
/// every occurrence of `?#k` in the pattern tree. All occurrences of the same `?#k`
/// must sit at the same depth (enforced by `reuse`, which rejects cross-depth unification).
#[derive(Debug, Clone)]
pub struct Pattern {
    pub pattern: RevExpr<ENodeOrVar<StitchLang>>,
    pub vars: Vec<Vec<Id>>, // vars[k] = all RecExpr ids holding Var(k)
    pub var_depth: Vec<u32>, // var_depth[k] = number of lam ancestors of ?#k
}

impl Pattern {
    /// Creates the initial `?#0` pattern: a single variable at depth 0.
    pub fn single_var() -> Self {
        let e: RevExpr<ENodeOrVar<StitchLang>> = RevExpr::new(vec![ENodeOrVar::Var(egg::Var::from(0))]);
        Pattern { pattern: e, vars: vec![vec![0.into()]], var_depth: vec![0] }
    }

    /// Expands the variable at `var_idx` with `target`. New children are inserted
    /// at list positions `var_idx..var_idx+k`; any vars that previously followed
    /// `var_idx` shift right and get their in-tree `Var(n)` leaves rewritten to
    /// match their new position, so the canonical-form invariant is preserved.
    pub fn expand(&mut self, var_idx: usize, target: &StitchLang) {
        let var_positions = self.vars.remove(var_idx);
        let parent_depth = self.var_depth.remove(var_idx);
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

        // Children of a `lam` sit under one additional binder; all other children
        // inherit the parent meta-var's depth. (`lam` has arity 1 so only one child
        // gets the bump — but the rule is written per-child to generalize cleanly.)
        let is_lam = matches!(target.op, crate::lang::Op::Lam);

        // Build the new enode with freshly-named Var children at positions var_idx..var_idx+k.
        let mut new_node = target.clone();
        for j in 0..num_children {
            let new_var = ENodeOrVar::Var(egg::Var::from((var_idx + j) as u32));
            self.pattern.nodes.push(new_var);
            let new_id = Id::from(self.pattern.nodes.len() - 1);
            new_node.children[j] = new_id;
            self.vars.insert(var_idx + j, vec![new_id]);
            self.var_depth.insert(var_idx + j, if is_lam { parent_depth + 1 } else { parent_depth });
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

        // Scope invariant: only meta-vars at the same binder depth may be unified.
        // Mixing depths would require shifting one side at substitution time, which
        // fights the canonicalization and is out of scope for now.
        assert_eq!(self.var_depth[keep_idx], self.var_depth[drop_idx], "reuse across differing binder depths is not allowed (depths {} vs {})", self.var_depth[keep_idx], self.var_depth[drop_idx]);

        let keep_name = ENodeOrVar::Var(egg::Var::from(keep_idx as u32));
        for var_id in &self.vars[drop_idx] {
            self.pattern[*var_id] = keep_name.clone();
        }
        let drop_ids = self.vars[drop_idx].clone();
        self.vars[keep_idx].extend(drop_ids);
        self.vars.remove(drop_idx);
        self.var_depth.remove(drop_idx);

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
