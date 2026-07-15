use crate::factor::Factor;
use crate::lang::{StitchEgraph, StitchLanguage};
use egg::Id;

/// All the ways the current pattern can match at a specific e-class, stored as a
/// cartesian factoring of the substitution set (see [`Factor`]).
/// Invariant: `root_eclass` and all ids in every factor row are canonical
/// (the egraph isn't unioned during search). The factors' `slots` partition
/// `0..arity` where `arity` is the pattern's variable count.
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

    /// Number of full substitutions = product of the factors' row counts.
    /// Factors are never empty, so this is `≥ 1`.
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

    /// This match with the variable slots in `drop` (ascending) projected out: each
    /// factor loses those columns (and its rows are re-deduplicated), factors left
    /// with no columns are removed, and the surviving slots are renumbered to the
    /// contiguous `0..(arity - drop.len())` range. Used to canonicalise away
    /// useless (constant) variables before computing a footprint signature.
    pub fn project_out(&self, drop: &[usize]) -> MatchAtEClass {
        // A surviving slot `s` shifts down by the number of dropped slots below it.
        let remap = |s: usize| s - drop.partition_point(|&d| d < s);
        let mut factors = Vec::with_capacity(self.factors.len());
        for f in &self.factors {
            let keep: Vec<usize> = (0..f.slots.len()).filter(|&pos| !drop.contains(&f.slots[pos])).collect();
            if keep.is_empty() {
                continue;
            }
            let slots: Vec<usize> = keep.iter().map(|&pos| remap(f.slots[pos])).collect();
            let rows: Vec<Vec<Id>> = f.rows.iter().map(|r| keep.iter().map(|&pos| r[pos]).collect()).collect();
            if let Some(nf) = Factor::new(slots, rows) {
                factors.push(nf);
            }
        }
        MatchAtEClass { root_eclass: self.root_eclass, factors }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(x: usize) -> Id {
        Id::from(x)
    }

    #[test]
    fn project_out_drops_column_and_renumbers() {
        // arity 3: factor {0,1}, factor {2}. Drop variable 1.
        let m = MatchAtEClass {
            root_eclass: id(100),
            factors: vec![Factor::new(vec![0, 1], vec![vec![id(10), id(20)], vec![id(11), id(20)]]).unwrap(), Factor::new(vec![2], vec![vec![id(30)], vec![id(31)]]).unwrap()],
        };
        let p = m.project_out(&[1]);
        assert_eq!(p.root_eclass, id(100));
        // `{0,1}` loses column 1 → `{0}` with rows `[10],[11]`; `{2}` renumbers to `{1}`.
        assert_eq!(p.factors.iter().map(|f| f.slots.clone()).collect::<Vec<_>>(), vec![vec![0], vec![1]]);
        assert_eq!(p.factors[0].rows, vec![vec![id(10)], vec![id(11)]]);
        assert_eq!(p.factors[1].rows, vec![vec![id(30)], vec![id(31)]]);
    }

    #[test]
    fn project_out_removes_now_empty_factor() {
        // arity 2: the dropped variable is alone in its factor.
        let m = MatchAtEClass {
            root_eclass: id(1),
            factors: vec![Factor::new(vec![0], vec![vec![id(5)]]).unwrap(), Factor::new(vec![1], vec![vec![id(6)], vec![id(7)]]).unwrap()],
        };
        let p = m.project_out(&[0]);
        assert_eq!(p.factors.len(), 1);
        assert_eq!(p.factors[0].slots, vec![0]);
        assert_eq!(p.factors[0].rows, vec![vec![id(6)], vec![id(7)]]);
    }
}
