use crate::lang::{StitchEgraph, StitchLanguage};
use egg::Id;
use rustc_hash::FxHashSet;

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

/// Returns one identity match per pre-shifted-variant corpus e-class,
/// skipping the root. The root holds the synthetic `(programs ...)` node that
/// wraps the whole corpus; letting the search match there produces
/// abstractions like `(programs ?#0 ?#0)` that collapse the program list,
/// which is never what we want. `original_eclasses` (from
/// `build_shifted_variants`) excludes the synthetic re-indexing classes —
/// those are search tools, not corpus subterms to match against.
pub fn identity_matches<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: egg::Id, original_eclasses: &FxHashSet<Id>) -> Vec<MatchAtEClass> {
    let root = egraph.find(root);
    original_eclasses.iter().filter(|&&id| id != root).map(|&id| MatchAtEClass::identity_match(id)).collect()
}
