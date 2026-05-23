use crate::lang::{StitchDisc, StitchEgraph, StitchLanguage};
use egg::Id;
use rustc_hash::{FxHashMap, FxHashSet};

/// True iff every free-variable index of `id` lies outside the half-open
/// gap `[lo, hi)`. Used to decide whether an η-wrap can reconcile a capture
/// across a binder-depth gap: indices below `lo` are shared enclosing
/// context, indices `≥ hi` are free at every site, indices in the gap are
/// pattern-internal at the deep site but context at the shallow one.
fn fv_outside_gap<L: StitchLanguage>(egraph: &StitchEgraph<L>, id: Id, lo: u32, hi: u32) -> bool {
    egraph[id].data.fv.iter().all(|&i| i < lo as i32 || i >= hi as i32)
}

/// Shift-aware equality of two captured e-class ids at depths `da` and `db`.
/// Returns true when both captures represent the same underlying value at
/// different binder contexts.
pub fn shift_equal<L: StitchLanguage>(a: Id, b: Id, da: u32, db: u32, egraph: &StitchEgraph<L>) -> bool {
    let a = egraph.find(a);
    let b = egraph.find(b);
    let (lo, hi) = (da.min(db), da.max(db));
    if a == b {
        return fv_outside_gap(egraph, a, lo, hi);
    }
    if da == db {
        return false;
    }
    let (deeper, shallower) = if da > db { (a, b) } else { (b, a) };
    let mut ctx = ShiftEqCtx {
        egraph,
        s: hi - lo,
        memo: FxHashMap::default(),
        stack: FxHashSet::default(),
    };
    let mut tainted = false;
    ctx.shift_eq_struct(deeper, shallower, 0, &mut tainted)
}

/// Recursion state for shift-equality: the e-graph, the shift `s`, the
/// final-results memo, and the on-stack set used for cycle detection.
struct ShiftEqCtx<'a, L: StitchLanguage> {
    egraph: &'a StitchEgraph<L>,
    s: u32,
    memo: FxHashMap<(Id, Id, u32), bool>,
    stack: FxHashSet<(Id, Id, u32)>,
}

impl<'a, L: StitchLanguage> ShiftEqCtx<'a, L> {
    /// True iff there exist enodes `na ∈ deeper` and `nb ∈ shallower` such
    /// that `na` is the shift-up-by-`s` form of `nb`: same discriminant and
    /// arity, child eclasses recursively shift-equal at the appropriate child
    /// depths, and any free DB-var leaf in `nb` (index `≥ init_depth`) is
    /// replaced by an index `s` larger in `na`. Bound indices (`< init_depth`)
    /// must match exactly.
    ///
    /// Cyclic e-classes use coinductive reasoning: a recursive call back into
    /// a key already on the call stack returns `true` (taking the cycle as a
    /// hypothesis) and sets `*caller_used_cycle = true`. A `true` result
    /// whose derivation depended on the hypothesis is *not* finalized in
    /// `memo` — it would be unsound if a sibling computation later falsifies
    /// the hypothesis. `false` results are always safe to cache (a `false`
    /// derivation can only be strengthened by replacing a hypothesis with
    /// its actual value).
    fn shift_eq_struct(&mut self, deeper: Id, shallower: Id, init_depth: u32, caller_used_cycle: &mut bool) -> bool {
        let deeper = self.egraph.find(deeper);
        let shallower = self.egraph.find(shallower);
        if deeper == shallower {
            // Same e-class viewed at different recursion depths: identical
            // physics to the top-level shared-capture case, just relative to
            // the current recursion frame.
            return fv_outside_gap(self.egraph, deeper, init_depth, init_depth + self.s);
        }
        let key = (deeper, shallower, init_depth);
        if let Some(&r) = self.memo.get(&key) {
            return r;
        }
        if self.stack.contains(&key) {
            // Coinductive hypothesis — the caller's derivation is now tainted.
            *caller_used_cycle = true;
            return true;
        }
        self.stack.insert(key);
        let mut local_used_cycle = false;
        let result = (0..self.egraph[deeper].nodes.len()).any(|i| {
            (0..self.egraph[shallower].nodes.len()).any(|j| {
                // Clone the enodes so we can recurse with `&mut self` without
                // holding immutable borrows of the e-graph node lists.
                let na = self.egraph[deeper].nodes[i].clone();
                let nb = self.egraph[shallower].nodes[j].clone();
                self.enode_shift_eq(&na, &nb, init_depth, &mut local_used_cycle)
            })
        });
        self.stack.remove(&key);
        if !result {
            self.memo.insert(key, false);
        } else if !local_used_cycle {
            self.memo.insert(key, true);
        } else {
            // True derivation depended on an unverified cycle hypothesis —
            // leave unmemoized and propagate the taint so the caller declines
            // to cache its own `true` result if it relies on us.
            *caller_used_cycle = true;
        }
        result
    }

    /// One-enode-pair step of `shift_eq_struct`: matches DB-var leaves modulo
    /// the `s`-shift on free indices, otherwise requires identical
    /// discriminant and structurally shift-equal children (with per-child
    /// binder bumps).
    fn enode_shift_eq(&mut self, na: &L, nb: &L, init_depth: u32, used_cycle: &mut bool) -> bool {
        let da = na.discriminant();
        let db = nb.discriminant();
        match (da.de_bruijn_index(), db.de_bruijn_index()) {
            (Some(i), Some(j)) => {
                let expected = if j < init_depth as i32 { j } else { j + self.s as i32 };
                return i == expected;
            }
            (None, None) => {}
            _ => return false,
        }
        if da != db || na.children().len() != nb.children().len() {
            return false;
        }
        na.children().iter().zip(nb.children().iter()).enumerate().all(|(k, (&ca, &cb))| {
            let new_depth = init_depth + if da.binds_child(k) { 1 } else { 0 };
            self.shift_eq_struct(ca, cb, new_depth, used_cycle)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::shift_equal;
    use crate::lang::{LambdaCalcLanguage, Op, StitchEgraph, StitchOp};

    /// Build the cyclic reproducer e-graph and return `(R_d, R_s)`. `a_first`
    /// controls the canonical-id ordering of the `A` and `C` e-classes (egg
    /// sorts each e-class's enode list lexicographically at rebuild, so child
    /// ids drive iteration order). Both orderings should satisfy the same
    /// soundness property; we test both.
    fn build_cyclic_egraph(a_first: bool) -> (StitchEgraph<LambdaCalcLanguage<Op>>, egg::Id, egg::Id) {
        let mut eg: StitchEgraph<LambdaCalcLanguage<Op>> = egg::EGraph::default();
        let leaf = |eg: &mut StitchEgraph<LambdaCalcLanguage<Op>>, s: &str| eg.add(LambdaCalcLanguage::Leaf(Op::from_name(s)));

        let z = leaf(&mut eg, "z");
        let x = leaf(&mut eg, "x");
        let y = leaf(&mut eg, "y");
        let p_d = leaf(&mut eg, "pd");
        let p_s = leaf(&mut eg, "ps");

        // Stubs to break the A↔C cycle at construction time. Adding A's stubs
        // first gives them lower canonical ids than C's stubs (and vice versa).
        let (a_d_stub, a_s_stub, c_d_stub, c_s_stub) = if a_first {
            let ad = leaf(&mut eg, "a_d_stub");
            let asx = leaf(&mut eg, "a_s_stub");
            let cd = leaf(&mut eg, "c_d_stub");
            let cs = leaf(&mut eg, "c_s_stub");
            (ad, asx, cd, cs)
        } else {
            let cd = leaf(&mut eg, "c_d_stub");
            let cs = leaf(&mut eg, "c_s_stub");
            let ad = leaf(&mut eg, "a_d_stub");
            let asx = leaf(&mut eg, "a_s_stub");
            (ad, asx, cd, cs)
        };

        let a_d_app = eg.add(LambdaCalcLanguage::App([c_d_stub, p_d]));
        let a_s_app = eg.add(LambdaCalcLanguage::App([c_s_stub, p_s]));
        let c_d_app = eg.add(LambdaCalcLanguage::App([a_d_stub, z]));
        let c_s_app = eg.add(LambdaCalcLanguage::App([a_s_stub, z]));
        eg.union(a_d_stub, a_d_app);
        eg.union(a_s_stub, a_s_app);
        eg.union(c_d_stub, c_d_app);
        eg.union(c_s_stub, c_s_app);
        eg.rebuild();

        let a_d = eg.find(a_d_stub);
        let a_s = eg.find(a_s_stub);
        let c_d = eg.find(c_d_stub);
        let c_s = eg.find(c_s_stub);
        if a_first {
            assert!(a_d < c_d && a_s < c_s, "expected A-first id ordering");
        } else {
            assert!(c_d < a_d && c_s < a_s, "expected C-first id ordering");
        }

        let r_d_a = eg.add(LambdaCalcLanguage::App([a_d, x]));
        let r_d_c = eg.add(LambdaCalcLanguage::App([c_d, y]));
        eg.union(r_d_a, r_d_c);
        let r_s_a = eg.add(LambdaCalcLanguage::App([a_s, x]));
        let r_s_c = eg.add(LambdaCalcLanguage::App([c_s, y]));
        eg.union(r_s_a, r_s_c);
        eg.rebuild();

        let r_d = eg.find(r_d_a);
        let r_s = eg.find(r_s_a);
        (eg, r_d, r_s)
    }

    /// Soundness regression: tentative-true memoization in `shift_eq_struct`
    /// must not finalize a recursive `true` that depended on an assumption
    /// later falsified.
    ///
    /// Construction (s = 1, all checks at depth 0):
    ///   A_d = App(C_d, P_d), A_s = App(C_s, P_s)   // P_d / P_s distinct leaves
    ///   C_d = App(A_d, Z),   C_s = App(A_s, Z)     // Z, X, Y shared closed leaves
    ///   R_d = { App(A_d, X), App(C_d, Y) }         // two enodes in one e-class
    ///   R_s = { App(A_s, X), App(C_s, Y) }
    ///
    /// Ground truth: `R_d` and `R_s` are *not* shift-equal — every structural
    /// witness eventually requires `(P_d, P_s)`, which fails. Asserted in both
    /// canonical-id orderings so the test isn't sensitive to egg's enode sort.
    #[test]
    fn cyclic_tentative_true_memo_bug() {
        for a_first in [true, false] {
            let (eg, r_d, r_s) = build_cyclic_egraph(a_first);
            assert!(!shift_equal(r_d, r_s, 1, 0, &eg), "a_first={a_first}: shift_equal must return false — every structural witness requires (P_d, P_s)");
        }
    }
}
