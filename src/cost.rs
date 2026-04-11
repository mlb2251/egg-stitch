use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Id, Language};
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern);
    cost + pattern_size
}

/// Returns the AST size of the pattern (counting each node and edge once).
pub fn compute_pattern_size(pattern: &Pattern) -> usize {
    1 + pattern.pattern.nodes.iter().map(|node| node.children().len()).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
/// When `check_slow` is set, cross-checks the result against a slow egg-based
/// reference implementation and panics on mismatch.
pub fn compute_size(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState, check_slow: bool) -> usize {
    let mut size_under_rewrite = FxHashMap::<Id, i64>::default();
    let mut work_queue = BinaryHeap::new();
    let mut eclass_to_matches = FxHashMap::<Id, &Vec<Subst>>::default();

    let get_size = |eclass: Id, s_u_r: &FxHashMap<Id, i64>| -> i64 { s_u_r.get(&eclass).cloned().unwrap_or(egraph[eclass].data as i64) };

    for m in &search_state.matches {
        work_queue.push(Reverse(m.root_eclass));
        eclass_to_matches.insert(m.root_eclass, &m.substs);
    }
    while let Some(Reverse(eclass)) = work_queue.pop() {
        // we assume that small numbers are children of large numbers, so when we pop we have already computed children
        if size_under_rewrite.contains_key(&eclass) {
            continue;
        }
        let size_current = get_size(eclass, &size_under_rewrite);
        let mut best = size_current;
        // trying a rewrite; (fn_i arg0 ...)
        if let Some(substs) = eclass_to_matches.get(&eclass) {
            for subst in *substs {
                let mut size_new: i64 = 1;
                for &var in &subst.vars {
                    size_new += get_size(var, &size_under_rewrite);
                }
                if size_new < best {
                    best = size_new;
                }
            }
        }
        // not doing a rewrite (just try all the enocdes)
        if let Some(enode) = egraph[eclass].nodes.first() {
            let mut size_no_rewrite: i64 = 1;
            for &child in &enode.children {
                size_no_rewrite += get_size(child, &size_under_rewrite);
            }
            if size_no_rewrite < best {
                best = size_no_rewrite;
            }
        }
        if best < size_current {
            for parent in egraph[eclass].parents() {
                work_queue.push(Reverse(parent));
            }
            size_under_rewrite.insert(eclass, best);
        }
    }
    let final_size = size_under_rewrite.get(&root).cloned().unwrap_or(egraph[root].data as i64);
    if check_slow {
        let slow_size = rewrite_slow(egraph, root, search_state) as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

/// Slow reference implementation of `compute_size`: clones the egraph, unions each
/// match root with an `inv_0(args...)` node, rebuilds, and reads the resulting size
/// out of the analysis data.
fn rewrite_slow(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState) -> usize {
    let mut egraph = egraph.clone();
    for m in &search_state.matches {
        for subst in &m.substs {
            let node = StitchLang { op: "inv_0".into(), children: subst.vars.clone() };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph[root].data as usize
}
