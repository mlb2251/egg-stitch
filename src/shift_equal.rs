use crate::lang::{StitchDisc, StitchEgraph, StitchLanguage};
use egg::Id;
use rustc_hash::FxHashMap;

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
    shift_eq_struct(egraph, deeper, shallower, hi - lo, 0, &mut FxHashMap::default())
}

/// True iff there exist enodes `na ∈ deeper` and `nb ∈ shallower` such that
/// `na` is the shift-up-by-`s` form of `nb`: same discriminant and arity,
/// child eclasses recursively shift-equal at the appropriate child depths,
/// and any free DB-var leaf in `nb` (index `≥ init_depth`) is replaced by an
/// index `s` larger in `na`. Bound indices (`< init_depth`) must match
/// exactly. Tentative-true memoization breaks cycles in the e-graph.
fn shift_eq_struct<L: StitchLanguage>(egraph: &StitchEgraph<L>, deeper: Id, shallower: Id, s: u32, init_depth: u32, memo: &mut FxHashMap<(Id, Id, u32), bool>) -> bool {
    let deeper = egraph.find(deeper);
    let shallower = egraph.find(shallower);
    if deeper == shallower {
        // Same e-class viewed at different recursion depths: identical
        // physics to the top-level shared-capture case, just relative to
        // the current recursion frame.
        return fv_outside_gap(egraph, deeper, init_depth, init_depth + s);
    }
    if let Some(&r) = memo.get(&(deeper, shallower, init_depth)) {
        return r;
    }
    memo.insert((deeper, shallower, init_depth), true);
    let result = egraph[deeper].nodes.iter().any(|na| egraph[shallower].nodes.iter().any(|nb| enode_shift_eq::<L>(egraph, na, nb, s, init_depth, memo)));
    memo.insert((deeper, shallower, init_depth), result);
    result
}

/// One-enode-pair step of `shift_eq_struct`: matches DB-var leaves modulo
/// the `s`-shift on free indices, otherwise requires identical discriminant
/// and structurally shift-equal children (with per-child binder bumps).
fn enode_shift_eq<L: StitchLanguage>(egraph: &StitchEgraph<L>, na: &L, nb: &L, s: u32, init_depth: u32, memo: &mut FxHashMap<(Id, Id, u32), bool>) -> bool {
    let da = na.discriminant();
    let db = nb.discriminant();
    match (da.de_bruijn_index(), db.de_bruijn_index()) {
        (Some(i), Some(j)) => {
            let expected = if j < init_depth as i32 { j } else { j + s as i32 };
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
        shift_eq_struct(egraph, ca, cb, s, new_depth, memo)
    })
}
