use crate::lang::StitchEgraph;
use egg::Id;
use rustc_hash::{FxHashMap, FxHashSet};

mod cost_only_extractor;
mod exact_cost;
mod lower_bound_cost;
mod rewrite_analysis;

pub use cost_only_extractor::CostOnlyExtractor;
pub use exact_cost::{compute_cost, compute_pattern_size};
pub(crate) use exact_cost::compute_size;
pub use lower_bound_cost::{LowerBoundAnalysis, compute_lower_bound};
pub use rewrite_analysis::{RewriteAnalysis, RewriteScratch};

/// Precomputed egraph topology for fast cost computation.
/// Built once from the egraph and reused across all `compute_cost` calls.
pub struct CostCache {
    /// Eclasses reachable from `root`, in postorder (children before parents).
    /// `solve` iterates this so child sizes settle before their parents reconsider.
    visit_order: Vec<Id>,
    /// Postorder index per eclass (children < parents). Indexed by `usize::from(Id)`.
    /// Currently unused by `solve`, but kept for callers/inspection.
    postorder: Vec<Option<u32>>,
    /// Child → parent eclass edges, built from all enodes.
    /// We maintain our own map because `egraph.parents()` can return stale non-canonical ids.
    /// Currently unused by `solve`, but kept for callers/inspection.
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
        let mut visit_order: Vec<Id> = Vec::new();
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
                    visit_order.push(id);
                    order += 1;
                }
            }
        }

        Self { visit_order, postorder, parents_of }
    }
}

/// Reusable allocations for repeated cost computations. Build once with `new(egraph)`
/// and pass `&mut` to `compute_cost`, `compute_size`, or `compute_lower_bound` to
/// avoid reallocating across calls.
pub struct CostScratch {
    pub runner: RunnerScratch,
    pub rewrite: RewriteScratch,
}

impl CostScratch {
    /// Builds the scratch space for a given egraph. The egraph's per-eclass AstSize
    /// is captured into `runner.original` here and reused across all subsequent calls.
    pub fn new(egraph: &StitchEgraph) -> Self {
        Self {
            runner: RunnerScratch::new(egraph),
            rewrite: RewriteScratch::default(),
        }
    }
}

/// Allocations owned by `StitchAnalysisRunner` itself (independent of the analysis).
/// Two parallel dense vectors indexed by `usize::from(Id)`: `original` holds the
/// un-rewritten AstSize per eclass (built once at construction), `overrides` is the
/// working size table that `solve` relaxes downward. Both are sized to `max_id + 1`.
pub struct RunnerScratch {
    original: Vec<i64>,
    overrides: Vec<i64>,
    /// Per-eclass dirty flag indexed by `usize::from(Id)`. `solve` only re-evaluates
    /// dirty eclasses; visiting clears the flag, and an improvement re-dirties the
    /// eclass's parents so they reconsider next time around.
    dirty: Vec<bool>,
}

impl RunnerScratch {
    /// Captures `original` from the egraph; `overrides` and `dirty` are left empty
    /// and filled by `reset` at the start of each solve.
    fn new(egraph: &StitchEgraph) -> Self {
        let max_id = egraph.classes().map(|c| usize::from(c.id)).max().unwrap_or(0);
        let mut original = vec![0i64; max_id + 1];
        for class in egraph.classes() {
            original[usize::from(class.id)] = class.data as i64;
        }
        Self { original, overrides: Vec::new(), dirty: Vec::new() }
    }
    /// Resets `overrides` to a copy of `original` and marks every eclass clean.
    /// Callers (or the analysis) seed dirty bits via `set` / `mark_dirty` before
    /// `solve` runs; nothing else needs revisiting. `original` is preserved.
    fn reset(&mut self) {
        self.overrides.clear();
        self.overrides.extend_from_slice(&self.original);
        self.dirty.clear();
        self.dirty.resize(self.original.len(), false);
    }
}

/// Pluggable per-eclass relaxation rule. `best` is an associated function (no `&self`)
/// so the solver can pass `&StitchAnalysisRunner<Self>` without conflicting borrows;
/// analysis-owned data is reached via `sizes.analysis`.
pub trait StitchAnalysis: Sized {
    /// Candidate size for `eclass` given currently known sizes.
    fn best(sizes: &StitchAnalysisRunner<Self>, eclass: Id) -> i64;
}

/// Dense per-eclass size table with a fallback to the unrewritten AstSize
/// (`egraph[id].data`). An entry is set only when the rewritten size beats the default.
pub struct StitchAnalysisRunner<'a, A: StitchAnalysis> {
    egraph: &'a StitchEgraph,
    cache: &'a CostCache,
    scratch: &'a mut RunnerScratch,
    pub analysis: A,
}
impl<'a, A: StitchAnalysis> StitchAnalysisRunner<'a, A> {
    /// Allocates the override table sized to the egraph's eclasses.
    fn new(egraph: &'a StitchEgraph, cache: &'a CostCache, scratch: &'a mut RunnerScratch, analysis: A) -> Self {
        scratch.reset();
        StitchAnalysisRunner { egraph, cache, scratch, analysis }
    }
    pub fn get(&self, id: Id) -> i64 {
        self.scratch.overrides[usize::from(id)]
    }
    /// Writes a new size for `id` and marks every parent dirty so they reconsider.
    /// `id` itself is left clean — re-evaluating won't beat the value we just wrote.
    fn set(&mut self, id: Id, v: i64) {
        self.scratch.overrides[usize::from(id)] = v;
        let parents = self.cache.parents_of.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
        for &p in parents {
            self.mark_dirty(p);
        }
    }
    /// Marks `id` dirty so `solve` will (re)evaluate it.
    fn mark_dirty(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = true;
    }
    /// Marks `id` clean (will be skipped by `solve` until something re-dirties it).
    fn mark_clean(&mut self, id: Id) {
        self.scratch.dirty[usize::from(id)] = false;
    }
    fn is_dirty(&self, id: Id) -> bool {
        self.scratch.dirty[usize::from(id)]
    }
    fn any_dirty(&self) -> bool {
        self.scratch.dirty.iter().any(|&d| d)
    }
    /// Sum of `get` over a list of eclass ids.
    pub fn sum(&self, ids: &[Id]) -> i64 {
        ids.iter().map(|&id| self.get(id)).sum()
    }
    pub fn original_size(&self, id: Id) -> i64 {
        self.scratch.original[usize::from(id)]
    }
    /// Minimum size over the enodes of `eclass`. Panics if the eclass has no enodes.
    pub fn min_enode_size(&self, eclass: Id) -> i64 {
        self.egraph[eclass].nodes.iter().map(|enode| 1 + self.sum(&enode.children)).min().unwrap()
    }
    /// Iterates eclasses reachable from the root in postorder (children first),
    /// re-evaluating only those marked dirty. Visiting clears the flag, and `set`
    /// re-marks parents on any improvement. Repeats until a full pass finds nothing
    /// better. Initial dirty bits are seeded by callers/analyses before `solve`.
    fn solve(&mut self) {
        while self.any_dirty() {
            for &id in &self.cache.visit_order {
                if !self.is_dirty(id) {
                    continue;
                }
                self.mark_clean(id);
                let new = A::best(self, id);
                if new < self.get(id) {
                    self.set(id, new);
                }
            }
        }
    }
}
