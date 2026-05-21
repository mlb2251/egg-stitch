use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use std::collections::HashMap;

/// Structural equality of two subtrees in the follow tree. Needed because
/// RecExpr doesn't hash-cons — repeated `?#0` nodes get distinct Ids.
fn follow_subtrees_equal<F: LanguageFamily, O: StitchOp>(follow: &RevExpr<F::Apply<OpWithVar<O>>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    let (na, nb) = (&follow[a], &follow[b]);
    match (na.discriminant().as_var(), nb.discriminant().as_var()) {
        (Some(va), Some(vb)) => va == vb,
        (None, None) => na.matches(nb) && na.children().iter().zip(nb.children().iter()).all(|(&ca, &cb)| follow_subtrees_equal::<F, O>(follow, ca, cb)),
        _ => false,
    }
}

/// Unifies a pattern against a follow target, returning the substitution that
/// makes the pattern a structural prefix of the follow — pattern Vars map to
/// whatever follow subtree they cover; everything else must match exactly.
/// `None` means the pattern is not a prefix of the follow.
pub fn follow_unify<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, follow: &RevExpr<F::Apply<OpWithVar<O>>>) -> Option<HashMap<egg::Var, Id>> {
    let mut bindings = HashMap::new();
    walk::<F, O>(pattern, Id::from(0), follow, Id::from(0), &mut bindings).then_some(bindings)
}

fn walk<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, pid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, bindings: &mut HashMap<egg::Var, Id>) -> bool {
    let (pn, fn_) = (&pattern[pid], &follow[fid]);
    if let Some(v) = pn.discriminant().as_var() {
        return match bindings.entry(v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal::<F, O>(follow, *e.get(), fid),
        };
    }
    if fn_.discriminant().as_var().is_some() {
        return false;
    }
    pn.matches(fn_) && pn.children().iter().zip(fn_.children().iter()).all(|(&pc, &fc)| walk::<F, O>(pattern, pc, follow, fc, bindings))
}

/// If `fid` in `follow` is structurally `wrap_pattern_with_db_apps(?v, vis)` —
/// a follow Var, optionally η-applied to `vis`'s de-Bruijn args — returns `?v`.
/// This is the per-binding witness that an `unify` result is alpha-equivalent
/// to the follow, not merely a prefix.
pub fn binding_as_exact_var<F: LanguageFamily, O: StitchOp>(follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, vis: &[i32]) -> Option<egg::Var> {
    if vis.is_empty() {
        return follow[fid].discriminant().as_var();
    }
    // Materialize the expected wrap shape with a placeholder Var at the head;
    // walking it against the follow lets us extract whatever Var the follow
    // has at that head position. The placeholder's identity is irrelevant —
    // `wrap_pattern_with_db_apps` inserts no other Var nodes, so any Var we
    // encounter while walking `expected` is the placeholder.
    let mut expected: egg::RecExpr<F::Apply<OpWithVar<O>>> = egg::RecExpr::default();
    let head = expected.add(F::make_var::<O>(egg::Var::from(0u32)));
    let db_args: Vec<i32> = vis.iter().rev().copied().collect();
    let root = F::wrap_pattern_with_db_apps::<O>(&mut expected, head, &db_args);
    match_with_hole::<F, O>(&expected, root, follow, fid).ok().flatten()
}

/// Walks `expected` against `follow[fid]`. The single Var in `expected` is a
/// placeholder: the follow must have a Var there, and that Var is returned.
/// Everything else must match structurally.
fn match_with_hole<F: LanguageFamily, O: StitchOp>(expected: &egg::RecExpr<F::Apply<OpWithVar<O>>>, eid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id) -> Result<Option<egg::Var>, ()> {
    let en = &expected[eid];
    let fn_ = &follow[fid];
    if en.discriminant().as_var().is_some() {
        return fn_.discriminant().as_var().map(Some).ok_or(());
    }
    if !en.matches(fn_) {
        return Err(());
    }
    let mut captured = None;
    for (&e, &f) in en.children().iter().zip(fn_.children().iter()) {
        if let Some(v) = match_with_hole::<F, O>(expected, e, follow, f)? {
            if captured.is_some() {
                return Err(());
            }
            captured = Some(v);
        }
    }
    Ok(captured)
}
