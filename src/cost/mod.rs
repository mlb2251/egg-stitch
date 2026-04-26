use crate::lang::StitchEgraph;
use egg::Id;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

mod cost_only_extractor;
mod exact_cost;
mod lower_bound_cost;
mod rewrite_analysis;

pub use cost_only_extractor::CostOnlyExtractor;
pub use exact_cost::{compute_cost, compute_pattern_size};
pub(crate) use exact_cost::compute_size;
pub use lower_bound_cost::{compute_lower_bound, LowerBoundAnalysis};
pub use rewrite_analysis::{build_eclass_to_substs, EclassToSubsts, RewriteAnalysis};

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

/// Pluggable per-eclass relaxation rule. The analysis decides which eclasses seed the
/// work queue and how to compute a candidate size for an eclass given the current
/// `StitchAnalysisRunner` state. Implemented as associated functions (no `&self`) so the solver can
/// pass `&StitchAnalysisRunner<Self>` without conflicting borrows; analysis-owned data is reached
/// via `sizes.analysis`, while shared match info lives on `sizes.eclass_to_substs`.
pub trait StitchAnalysis: Sized {
    /// Eclasses to push onto the work queue when the solver starts.
    fn init(sizes: &StitchAnalysisRunner<Self>) -> Vec<Id>;
    /// Candidate size for `eclass` given currently known sizes.
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64;
}

/// Sparse per-eclass size map with a fallback to the unrewritten AstSize (`egraph[id].data`).
/// Entries represent eclasses whose rewritten size is strictly smaller than the default.
pub struct StitchAnalysisRunner<'a, A: StitchAnalysis> {
    egraph: &'a StitchEgraph,
    cache: &'a CostCache,
    overrides: FxHashMap<Id, i64>,
    work_queue: BinaryHeap<Reverse<(u32, Id)>>,
    pub analysis: A,
}
impl<'a, A: StitchAnalysis> StitchAnalysisRunner<'a, A> {
    /// Builds an empty size table seeded with the analysis's chosen eclasses.
    fn new(egraph: &'a StitchEgraph, cache: &'a CostCache, analysis: A) -> Self {
        let mut sizes = StitchAnalysisRunner {
            egraph,
            cache,
            overrides: FxHashMap::default(),
            work_queue: BinaryHeap::new(),
            analysis,
        };
        for id in A::init(&sizes) {
            sizes.work_queue.push(Reverse((cache.postorder[usize::from(id)].unwrap(), id)));
        }
        sizes
    }
    pub fn get(&self, id: Id) -> i64 {
        self.overrides.get(&id).copied().unwrap_or(self.original_size(id))
    }
    fn set(&mut self, id: Id, v: i64) {
        self.overrides.insert(id, v);
    }
    fn contains(&self, id: Id) -> bool {
        self.overrides.contains_key(&id)
    }
    /// Sum of `get` over a list of eclass ids.
    pub fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    pub fn original_size(&self, id: Id) -> i64 {
        self.egraph[id].data as i64
    }
    /// Minimum size over the enodes of `eclass`. Panics if the eclass has no enodes.
    pub fn min_enode_size(&self, eclass: Id) -> i64 {
        self.egraph[eclass].nodes.iter().map(|enode| 1 + self.sum(&enode.children)).min().unwrap()
    }
    /// If `new` improves on the current size of `eclass`, record it and enqueue parents for re-relaxation.
    fn update(&mut self, eclass: Id, new: i64) {
        if new < self.get(eclass) {
            self.notify_parents(eclass);
            self.set(eclass, new);
        }
    }
    /// Runs the postorder relaxation until the work queue drains.
    fn solve(&mut self) {
        while let Some(Reverse((_, eclass))) = self.work_queue.pop() {
            if self.contains(eclass) {
                continue;
            }
            let best = A::best(self, eclass);
            self.update(eclass, best);
        }
    }
    /// Re-enqueues every parent of `eclass` so they reconsider the new child size.
    fn notify_parents(&mut self, eclass: Id) {
        if let Some(parents) = self.cache.parents_of.get(&eclass) {
            for &parent in parents {
                if let Some(po) = self.cache.postorder[usize::from(parent)] {
                    self.work_queue.push(Reverse((po, parent)));
                }
            }
        }
    }
}
