use crate::lang::StitchEgraph;
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

/// Returns one identity match per e-class in the egraph.
pub fn identity_matches(egraph: &StitchEgraph) -> Vec<MatchAtEClass> {
    egraph.classes().map(|c| MatchAtEClass::identity_match(c.id)).collect()
}
