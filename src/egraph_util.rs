//! E-graph traversal and query helpers used by the search, kept apart from the
//! search logic itself. Each takes a [`StitchEgraph`] and computes a fact about
//! it — a size-minimal extraction or per-class usage counts — without touching
//! the search state.

use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp};
use crate::matching::Subst;
use egg::{Id, Language};
use rustc_hash::{FxHashMap, FxHashSet};

/// True iff the e-graph contains a cycle — a class reachable from itself by
/// following enode children. Identity-shrinking DSRs create these (a 1-cycle from
/// `x => (T x (M 1 0 0 0))`, a 2-cycle from `(f (g ?x)) => ?x`) and are the source
/// of unbounded no-op wrapper towers. See [`crate::search::SharedSearchData::has_cycle`].
///
/// Iterative DFS with white/gray/black coloring over canonical class ids; a back
/// edge (an edge into a gray ancestor still on the stack) means a cycle. Adjacency
/// is the union of each class's enodes' (canonicalized) children.
pub fn egraph_has_cycle<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> bool {
    let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for class in egraph.classes() {
        let succ = adj.entry(egraph.find(class.id)).or_default();
        for node in &class.nodes {
            succ.extend(node.children().iter().map(|&c| egraph.find(c)));
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        Gray,
        Black,
    }
    let mut color: FxHashMap<Id, Color> = FxHashMap::default();
    let starts: Vec<Id> = adj.keys().copied().collect();
    for start in starts {
        if color.contains_key(&start) {
            continue;
        }
        color.insert(start, Color::Gray);
        let mut stack: Vec<(Id, usize)> = vec![(start, 0)];
        while let Some(&(id, idx)) = stack.last() {
            let succs = &adj[&id];
            if idx < succs.len() {
                stack.last_mut().unwrap().1 += 1;
                let next = succs[idx];
                match color.get(&next).copied() {
                    Some(Color::Gray) => return true, // back edge ⇒ cycle
                    Some(Color::Black) => {}
                    None => {
                        color.insert(next, Color::Gray);
                        stack.push((next, 0));
                    }
                }
            } else {
                color.insert(id, Color::Black);
                stack.pop();
            }
        }
    }
    false
}

/// Fills `ec[i]` with `ec_σ(i)` — the e-class the subpattern at node `i` denotes
/// under the substitution `subst = σ`, in the paper's notation:
///
///   ec_σ(?v)         = σ(v)                              (a `Var(k)` leaf)
///   ec_σ(op(i₁..iₖ)) = lookup_G(op(ec_σ(i₁)..ec_σ(iₖ)))  (interior node)
///
/// where `lookup_G` returns `⊥` (`None`) when the composite enode isn't
/// hash-consed; `⊥` then propagates upward. When defined, `ec_σ(i)` is the e-class
/// of the instantiated subpattern, and at the root it is the match root `r`.
/// Children sit at higher indices than parents in a RevExpr, so the single
/// high→low pass fills children before parents. `ec` must be ≥ `nodes.len()` long.
///
/// `pos_to_var[i]` is the metavariable index of node `i` if it is a `Var` leaf,
/// or `usize::MAX` otherwise.
pub fn compute_eclasses_for_pattern_nodes<F: LanguageFamily, O: StitchOp>(nodes: &[F::Apply<OpWithVar<O>>], pos_to_var: &[usize], subst: &Subst, egraph: &StitchEgraph<F::Apply<O>>, ec: &mut [Option<Id>]) {
    for i in (0..nodes.len()).rev() {
        ec[i] = if pos_to_var[i] != usize::MAX {
            Some(egraph.find(subst.vars[pos_to_var[i]]))
        } else {
            let disc = F::map_discriminant(nodes[i].discriminant(), |ov| match ov {
                OpWithVar::Node(o) => o,
                OpWithVar::Var(_) => unreachable!("var leaf handled via pos_to_var"),
            });
            nodes[i].children().iter().map(|&c| ec[usize::from(c)]).collect::<Option<Vec<Id>>>().and_then(|kids| egraph.lookup(F::make(disc, kids)))
        };
    }
}

/// Walks `eclass` picking the size-minimal enode at each step (same cost
/// rule as `extract_root_size`: intrinsic node weight + sum of child eclass
/// sizes), appending each enode in postorder to `out` with its children
/// remapped to the appended positions. `memo` shares any eclass visited
/// twice in the walk, so the result is DAG-shared rather than tree-expanded.
/// Returns the root's index in `out`. Discriminants are lifted into the
/// pattern leaf-op via `OpWithVar::Node` so the result splices into a
/// `Pattern<F, O>::pattern` directly.
pub fn build_size_minimal_extraction<F: LanguageFamily, O: StitchOp>(egraph: &StitchEgraph<F::Apply<O>>, eclass: Id, out: &mut Vec<F::Apply<OpWithVar<O>>>, memo: &mut FxHashMap<Id, Id>) -> Id {
    let canonical = egraph.find(eclass);
    if let Some(&id) = memo.get(&canonical) {
        return id;
    }
    let weights = egraph.analysis.weights;
    let rep = egraph[canonical]
        .nodes
        .iter()
        .min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&c| egraph[c].data.size as u64).sum::<u64>())
        .expect("non-empty eclass")
        .clone();
    let children: Vec<Id> = rep.children().iter().map(|&c| build_size_minimal_extraction::<F, O>(egraph, c, out, memo)).collect();
    let node = F::make(F::map_discriminant(rep.discriminant(), OpWithVar::Node), children);
    out.push(node);
    let id = Id::from(out.len() - 1);
    memo.insert(canonical, id);
    id
}

/// Computes how many times each e-class appears in the fully-expanded corpus tree.
/// Top-down pass: root gets count 1, then propagate to children of the size-minimal
/// enode (same rule as [`build_size_minimal_extraction`] and `WeightedSize`).
///
/// Heuristic: this only accounts for the pre-rewrite extraction. `RewriteAnalysis`
/// may route through a non-minimal enode when a rewrite shrinks the result, so
/// counts can under-attribute multiplicity at e-classes only reached that way.
///
/// Canonical eclass ids are not necessarily in topological order after unions
/// (a parent's canonical id can be lower than a child's), so we explicitly
/// derive a parents-before-children order via iterative DFS post-order from
/// the root and propagate along it.
pub fn compute_usage_counts<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> FxHashMap<Id, usize> {
    let root = egraph.find(root);
    let weights = egraph.analysis.weights;
    let min_enode = |id: Id| -> Option<&L> { egraph[id].nodes.iter().min_by_key(|n| n.discriminant().intrinsic_size(&weights) as u64 + n.children().iter().map(|&c| egraph[c].data.size as u64).sum::<u64>()) };
    let mut order: Vec<Id> = Vec::new();
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut stack: Vec<(Id, bool)> = vec![(root, false)];
    while let Some((id, post)) = stack.pop() {
        if post {
            order.push(id);
            continue;
        }
        if !seen.insert(id) {
            continue;
        }
        stack.push((id, true));
        if let Some(enode) = min_enode(id) {
            for &child in enode.children() {
                let child = egraph.find(child);
                if !seen.contains(&child) {
                    stack.push((child, false));
                }
            }
        }
    }
    order.reverse();
    let mut counts = FxHashMap::<Id, usize>::default();
    counts.insert(root, 1);
    for id in order {
        let count = counts.get(&id).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        if let Some(enode) = min_enode(id) {
            for &child in enode.children() {
                *counts.entry(egraph.find(child)).or_insert(0) += count;
            }
        }
    }
    counts
}
