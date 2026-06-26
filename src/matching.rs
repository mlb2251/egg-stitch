use crate::factor::Factor;
use crate::lang::{StitchEgraph, StitchLanguage};
use egg::Id;

/// All the ways the current pattern can match at a specific e-class, stored as a
/// cartesian factoring of the substitution set (see [`Factor`]).
/// Invariant: `root_eclass` and all ids in every factor row are canonical
/// (the egraph isn't unioned during search). The factors' `slots` partition
/// `0..arity` where `arity` is the pattern's variable count; every factor covers
/// at least one slot (zero-slot identity factors are dropped at construction), so
/// a fully-ground pattern has an *empty* factor list — its single empty
/// substitution is the empty product.
#[derive(Debug, Clone)]
pub struct MatchAtEClass {
    pub root_eclass: Id,
    pub factors: Vec<Factor>,
}

impl MatchAtEClass {
    /// Creates a match for e-class `c` with a single-variable pattern: one
    /// factor over slot 0 with the lone row `[c]`.
    pub fn identity_match(c: Id) -> Self {
        Self {
            root_eclass: c,
            factors: vec![Factor { slots: vec![0], rows: vec![vec![c]] }],
        }
    }

    /// Number of full substitutions = product of the factors' row counts. Each
    /// factor's rows are non-empty, and the empty product is 1, so this is `≥ 1`
    /// (a ground match with no factors counts as one empty substitution).
    pub fn num_substs(&self) -> usize {
        self.factors.iter().map(|f| f.rows.len()).product()
    }

    /// `(factor_index, position_within_factor)` for `slot`. Panics if no factor
    /// covers it (a broken partition invariant).
    pub fn locate_slot(&self, slot: usize) -> (usize, usize) {
        for (fi, f) in self.factors.iter().enumerate() {
            if let Some(pos) = f.pos_of(slot) {
                return (fi, pos);
            }
        }
        panic!("slot {slot} not covered by any factor");
    }
}

/// Returns one identity match per e-class in the egraph, skipping the root
/// e-class. The root holds the synthetic `(programs ...)` node that wraps the
/// whole corpus; letting the search match there produces abstractions like
/// `(programs ?#0 ?#0)` that collapse the program list itself, which is never
/// what we want.
pub fn identity_matches<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: Id) -> Vec<MatchAtEClass> {
    let root = egraph.find(root);
    egraph.classes().filter(|c| c.id != root).map(|c| MatchAtEClass::identity_match(c.id)).collect()
}
