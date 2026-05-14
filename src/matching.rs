use crate::lang::{StitchEgraph, StitchLanguage};
use egg::Id;

/// All the ways the current pattern can match at a specific e-class.
#[derive(Debug, Clone)]
pub struct MatchAtEClass {
    pub root_eclass: egg::Id,
    pub substs: Vec<Subst>,
}

/// One assignment of pattern variables to e-class ids.
#[derive(Debug, Clone)]
pub struct Subst {
    pub vars: Vec<Id>,
}

impl MatchAtEClass {
    /// Creates a match for e-class `c` with a single substitution mapping the root variable to `c`.
    pub fn identity_match(c: egg::Id) -> Self {
        Self { root_eclass: c, substs: vec![Subst { vars: vec![c] }] }
    }
}

/// Returns one identity match per e-class in the egraph, skipping the root
/// e-class. The root holds the synthetic `(programs ...)` node that wraps the
/// whole corpus; letting the search match there produces abstractions like
/// `(programs ?#0 ?#0)` that collapse the program list itself, which is never
/// what we want.
pub fn identity_matches<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: egg::Id) -> Vec<MatchAtEClass> {
    let root = egraph.find(root);
    // Restrict to classes reachable from `root`. After build_shifted_variants
    // the egraph carries synthetic re-indexings of real subterms — they're
    // tools for the search, not corpus subterms to match against — so they
    // must be kept out of the match-root set.
    let reachable = reachable_from(egraph, root);
    reachable.iter().filter(|&&id| id != root).map(|&id| MatchAtEClass::identity_match(id)).collect()
}

/// All e-class ids reachable from `start` via any enode child edge. Result ids
/// are canonical.
pub fn reachable_from<L: StitchLanguage>(egraph: &StitchEgraph<L>, start: egg::Id) -> rustc_hash::FxHashSet<egg::Id> {
    let mut seen: rustc_hash::FxHashSet<egg::Id> = rustc_hash::FxHashSet::default();
    let mut stack = vec![egraph.find(start)];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for enode in &egraph[id].nodes {
            for &child in enode.children() {
                stack.push(egraph.find(child));
            }
        }
    }
    seen
}
