use crate::lang::{StitchEgraph, StitchLang};
use crate::matching::Subst;
use crate::pattern::Pattern;
use crate::search::SearchState;
use egg::{Id, Language};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    postorder: Vec<Option<u32>>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    parents_of: FxHashMap<Id, Vec<Id>>,
}

impl CostCache {
    /// Builds the cache from the egraph rooted at `root`.
    pub fn new(egraph: &StitchEgraph, root: Id) -> Self {
        let mut parents_of = FxHashMap::<Id, Vec<Id>>::default();
        for class in egraph.classes() {
            for enode in &class.nodes {
                for &child in &enode.children {
                    parents_of.entry(child).or_default().push(class.id);
                }
            }
        }

        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut postorder = vec![None; max_id + 1];
        let mut order: u32 = 0;
        let mut stack: Vec<Result<Id, Id>> = vec![Err(root)]; // Err=enter, Ok=exit
        let mut on_stack = FxHashSet::<Id>::default();
        while let Some(state) = stack.pop() {
            match state {
                Err(id) => {
                    if postorder[usize::from(id)].is_some() || !on_stack.insert(id) {
                        continue;
                    }
                    stack.push(Ok(id));
                    for enode in &egraph[id].nodes {
                        for &child in &enode.children {
                            stack.push(Err(child));
                        }
                    }
                }
                Ok(id) => {
                    on_stack.remove(&id);
                    postorder[usize::from(id)] = Some(order);
                    order += 1;
                }
            }
        }

        Self { postorder, parents_of }
    }
}

/// Returns the total cost: compressed corpus size plus the pattern's own size.
pub fn compute_cost(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let cost = compute_size(egraph, root, cache, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern);
    cost + pattern_size
}

/// Returns the AST size of the pattern (counting each node and edge once).
pub fn compute_pattern_size(pattern: &Pattern) -> usize {
    1 + pattern.pattern.nodes.iter().map(|node| node.children().len()).sum::<usize>()
}

/// Computes the minimum corpus size achievable by applying the pattern as a rewrite.
///
/// Uses a work-queue ordered by postorder (children before parents) so each
/// eclass is visited at most once.
/// Sparse per-eclass size map with a fallback to the unrewritten AstSize (`egraph[id].data`).
/// Entries represent eclasses whose rewritten size is strictly smaller than the default.
struct Sizes<'a> {
    egraph: &'a StitchEgraph,
    overrides: FxHashMap<Id, i64>,
}
impl Sizes<'_> {
    fn get(&self, id: Id) -> i64 {
        self.overrides.get(&id).copied().unwrap_or(self.egraph[id].data as i64)
    }
    fn set(&mut self, id: Id, v: i64) {
        self.overrides.insert(id, v);
    }
    fn contains(&self, id: Id) -> bool {
        self.overrides.contains_key(&id)
    }
}

pub(crate) fn compute_size(egraph: &StitchEgraph, root: egg::Id, cache: &CostCache, search_state: &SearchState, check_slow: bool) -> usize {
    let mut eclass_to_matches = FxHashMap::<Id, &Vec<Subst>>::default();
    let mut sizes = Sizes { egraph, overrides: FxHashMap::default() };
    let mut work_queue = BinaryHeap::new();
    for m in &search_state.matches {
        eclass_to_matches.insert(m.root_eclass, &m.substs);
        work_queue.push(Reverse((cache.postorder[usize::from(m.root_eclass)].unwrap(), m.root_eclass)));
    }
    while let Some(Reverse((_, eclass))) = work_queue.pop() {
        if sizes.contains(eclass) {
            continue;
        }

        // size without rewriting self NOR any descendants
        let size_current = egraph[eclass].data as i64;
        let mut best = size_current;

        // For every way we match at this eclass (if any), try all ways of rewriting it
        // (relies on postorder guaranteeing descendants (arguments) have sizes.get done)
        if let Some(substs) = eclass_to_matches.get(&eclass) {
            for subst in *substs {
                let size_new: i64 = 1 + subst.vars.iter().map(|&v| sizes.get(v)).sum::<i64>();
                best = best.min(size_new);
            }
        }

        // Try not rewriting self but YES allowing rewrites of descendants
        // (relies on postorder guaranteeing children have sizes.get done)
        for enode in &egraph[eclass].nodes {
            let size_no_rewrite: i64 = 1 + enode.children.iter().map(|&c| sizes.get(c)).sum::<i64>();
            best = best.min(size_no_rewrite);
        }

        // If we found a smaller size than the "no rewriting and no descendant rewriting" size, push
        // our parents to the queue to make sure they get updated
        if best < size_current {
            if let Some(parents) = cache.parents_of.get(&eclass) {
                for &parent in parents {
                    if let Some(po) = cache.postorder[usize::from(parent)] {
                        work_queue.push(Reverse((po, parent)));
                    }
                }
            }
            sizes.set(eclass, best);
        }
    }
    let final_size = sizes.get(root);
    if check_slow {
        let slow_size = build_rewritten_egraph(egraph, search_state)[root].data as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

/// Clones the egraph and unions each match root with an `inv_0(args...)` node, then rebuilds.
/// Used for validating `compute_size` and for extracting rewritten programs.
pub(crate) fn build_rewritten_egraph(egraph: &StitchEgraph, search_state: &SearchState) -> StitchEgraph {
    let mut egraph = egraph.clone();
    for m in &search_state.matches {
        for subst in &m.substs {
            let node = StitchLang { op: "inv_0".into(), children: subst.vars.clone() };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    egraph
}

/// Extracts each program from the rewritten egraph, using `inv_0` where it reduces size.
pub fn extract_rewritten_programs(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState) -> Vec<String> {
    let rewritten = build_rewritten_egraph(egraph, search_state);
    let extractor = egg::Extractor::new(&rewritten, egg::AstSize);
    rewritten[root].nodes[0].children.iter().map(|&child| extractor.find_best(child).1.to_string()).collect()
}
