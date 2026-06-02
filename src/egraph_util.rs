//! E-graph traversal and query helpers used by the search, kept apart from the
//! search logic itself. Each takes a [`StitchEgraph`] and computes a fact about
//! it — a size-minimal extraction or per-class usage counts — without touching
//! the search state.

use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp};
use crate::matching::Subst;
use egg::{Id, Language};
use rustc_hash::{FxHashMap, FxHashSet};

/// Returns the set of e-classes that lie on a cycle — a class reachable from
/// itself by following enode children. Identity-shrinking DSRs create these (a
/// 1-cycle from `x => (T x (M 1 0 0 0))`, a 2-cycle from `(f (g ?x)) => ?x`) and
/// are the source of unbounded no-op wrapper towers. A class is on a cycle iff it
/// is in a strongly-connected component of size ≥ 2, or is a singleton with a
/// self-edge (an enode whose own child is its class). The set is empty iff the
/// e-graph is acyclic. See [`crate::search::SharedSearchData`].
///
/// Iterative Tarjan SCC over canonical class ids; adjacency is the union of each
/// class's enodes' (canonicalized) children, deduped.
pub fn egraph_cycle_classes<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> FxHashSet<Id> {
    let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut self_edge: FxHashSet<Id> = FxHashSet::default();
    for class in egraph.classes() {
        let id = egraph.find(class.id);
        let succ = adj.entry(id).or_default();
        for node in &class.nodes {
            for &c in node.children() {
                let c = egraph.find(c);
                if c == id {
                    self_edge.insert(id);
                }
                succ.push(c);
            }
        }
        succ.sort_unstable();
        succ.dedup();
    }

    // Iterative Tarjan. `index`/`lowlink` keyed by class; `stack`+`on_stack` are
    // the SCC stack; `call` is the explicit DFS stack of (node, next-child-idx).
    let mut index: FxHashMap<Id, u32> = FxHashMap::default();
    let mut lowlink: FxHashMap<Id, u32> = FxHashMap::default();
    let mut on_stack: FxHashSet<Id> = FxHashSet::default();
    let mut stack: Vec<Id> = Vec::new();
    let mut counter: u32 = 0;
    let mut result: FxHashSet<Id> = FxHashSet::default();

    let starts: Vec<Id> = adj.keys().copied().collect();
    for start in starts {
        if index.contains_key(&start) {
            continue;
        }
        index.insert(start, counter);
        lowlink.insert(start, counter);
        counter += 1;
        stack.push(start);
        on_stack.insert(start);
        let mut call: Vec<(Id, usize)> = vec![(start, 0)];
        while let Some(&(v, ci)) = call.last() {
            let succs = &adj[&v];
            if ci < succs.len() {
                call.last_mut().unwrap().1 += 1;
                let w = succs[ci];
                match index.get(&w).copied() {
                    None => {
                        index.insert(w, counter);
                        lowlink.insert(w, counter);
                        counter += 1;
                        stack.push(w);
                        on_stack.insert(w);
                        call.push((w, 0));
                    }
                    Some(iw) if on_stack.contains(&w) => {
                        let e = lowlink.get_mut(&v).unwrap();
                        *e = (*e).min(iw);
                    }
                    Some(_) => {}
                }
            } else {
                // v is fully explored; if it's an SCC root, pop the component.
                if lowlink[&v] == index[&v] {
                    let mut comp: Vec<Id> = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack.remove(&w);
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    // Size ≥ 2 ⇒ every member is on a cycle; a singleton is cyclic
                    // only if it has a self-edge.
                    if comp.len() >= 2 {
                        result.extend(comp);
                    } else if self_edge.contains(&comp[0]) {
                        result.insert(comp[0]);
                    }
                }
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    let lv = lowlink[&v];
                    let e = lowlink.get_mut(&parent).unwrap();
                    *e = (*e).min(lv);
                }
            }
        }
    }
    result
}

/// Fills `ec[i]` with the e-class each pattern node maps to under `subst`:
/// `Var(k)` leaves resolve to `subst.vars[k]`; interior nodes resolve via
/// `egraph.lookup` of the op applied to its children's e-classes (`None` if the
/// composite enode isn't hash-consed, which propagates upward). Children sit at
/// higher indices than parents in a RevExpr, so the single high→low pass fills
/// children before parents. `ec` must be at least `nodes.len()` long.
///
/// `pos_to_var[i]` is the metavariable index of node `i` if it is a `Var` leaf,
/// or `usize::MAX` otherwise.
pub fn compute_node_eclasses<F: LanguageFamily, O: StitchOp>(nodes: &[F::Apply<OpWithVar<O>>], pos_to_var: &[usize], subst: &Subst, egraph: &StitchEgraph<F::Apply<O>>, ec: &mut [Option<Id>]) {
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
