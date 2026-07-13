//! De Bruijn index shifting for pattern fragments.
//!
//! When a metavar reused across binder depths is concretized, the captured
//! value (tracked at the shallow depth) is lifted to each deeper occurrence's
//! depth — only its *free* indices move; indices bound inside the fragment
//! stay put. [`shift_db_disc`] does this for a single leaf discriminant (used
//! by `Pattern::expand` for literal expansions); [`shift_extraction`] does it
//! for a whole postorder fragment (used by `Pattern::concretize`).

use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use egg::{Id, Language};
use rustc_hash::FxHashMap;

/// Shifts the De Bruijn index carried by a leaf discriminant up by `delta`,
/// leaving structural discriminants and non-DB leaves untouched.
pub(crate) fn shift_db_disc<F: LanguageFamily, O: StitchOp>(disc: F::Discriminant<O>, delta: i32) -> F::Discriminant<O> {
    if delta == 0 {
        return disc;
    }
    F::map_discriminant(disc, |leaf: O| match leaf.de_bruijn_index() {
        Some(i) => O::make_db_var(i + delta).expect("DB-var leaf must reconstruct after shift"),
        None => leaf,
    })
}

/// Capture-aware copy of postorder `extraction` (root last): free DB indices
/// shift up by `delta`, indices bound inside the extraction stay. Returns the
/// new list and its root. Memoised on `(id, cutoff)`; cutoff bumps under each
/// `binds_child` slot, matching `enode_fv`.
pub(crate) fn shift_extraction<F: LanguageFamily, O: StitchOp>(extraction: &[F::Apply<OpWithVar<O>>], root: Id, delta: i32) -> (Vec<F::Apply<OpWithVar<O>>>, Id) {
    let mut out: Vec<F::Apply<OpWithVar<O>>> = Vec::new();
    let mut memo: FxHashMap<(Id, u32), Id> = FxHashMap::default();
    let r = shift_extraction_rec::<F, O>(extraction, root, 0, delta, &mut out, &mut memo);
    (out, r)
}

/// Recursive worker for [`shift_extraction`]: emits the shifted form of node
/// `id` (whose free/bound boundary is `cutoff` binders) into `out`, returning
/// its new postorder index.
fn shift_extraction_rec<F: LanguageFamily, O: StitchOp>(extraction: &[F::Apply<OpWithVar<O>>], id: Id, cutoff: u32, delta: i32, out: &mut Vec<F::Apply<OpWithVar<O>>>, memo: &mut FxHashMap<(Id, u32), Id>) -> Id {
    if let Some(&m) = memo.get(&(id, cutoff)) {
        return m;
    }
    let node = &extraction[usize::from(id)];
    let disc = node.discriminant();
    let new_children: Vec<Id> = node
        .children()
        .iter()
        .enumerate()
        .map(|(j, &c)| {
            let child_cutoff = cutoff + if disc.binds_child(j) { 1 } else { 0 };
            shift_extraction_rec::<F, O>(extraction, c, child_cutoff, delta, out, memo)
        })
        .collect();
    let new_disc = F::map_discriminant(disc, |leaf: OpWithVar<O>| match leaf.de_bruijn_index() {
        // Free index (points above the extraction): shift to the new depth.
        Some(i) if i >= cutoff as i32 => OpWithVar::make_db_var(i + delta).expect("DB-var leaf must reconstruct after shift"),
        // Bound index or non-DB leaf: unchanged.
        _ => leaf,
    });
    out.push(F::make(new_disc, new_children));
    let new_id = Id::from(out.len() - 1);
    memo.insert((id, cutoff), new_id);
    new_id
}

#[cfg(test)]
mod tests {
    use super::shift_extraction;
    use crate::lang::{LambdaCalc, LambdaCalcLanguage, Op, OpDB, OpWithVar, StitchLanguage};
    use egg::{Id, RecExpr};

    /// Pattern-side lambda-calc term (program leaves + would-be pattern vars).
    type Pat = LambdaCalcLanguage<OpWithVar<OpDB<Op>>>;

    /// Parse a lambda-calc term, shift its free DB indices up by `delta`, and
    /// render the result back. A `RecExpr` is already postorder with the root
    /// last, i.e. exactly the extraction format `shift_extraction` consumes.
    fn shifted(s: &str, delta: i32) -> String {
        let expr: RecExpr<Pat> = Pat::parse_program(s).expect("parse");
        let root = Id::from(expr.as_ref().len() - 1);
        let (out, out_root) = shift_extraction::<LambdaCalc, OpDB<Op>>(expr.as_ref(), root, delta);
        let out_expr: RecExpr<Pat> = out.into();
        assert_eq!(usize::from(out_root), out_expr.as_ref().len() - 1, "root must be the last node");
        Pat::display_recexpr(&out_expr)
    }

    #[test]
    fn free_indices_shift() {
        assert_eq!(shifted("$0", 2), "$2");
        assert_eq!(shifted("(foo $0 $1)", 1), "(foo $1 $2)");
    }

    #[test]
    fn delta_zero_is_identity() {
        assert_eq!(shifted("(foo $0 (lam $1))", 0), "(foo $0 (lam $1))");
    }

    #[test]
    fn bound_stays_free_shifts() {
        // Under one `lam`: `$0` is bound (unchanged), `$1` is free (shifts by 2).
        assert_eq!(shifted("(lam (foo $0 $1))", 2), "(lam (foo $0 $3))");
    }

    #[test]
    fn nested_binders() {
        // Two `lam`s bind `$0`/`$1`; only the free `$2` shifts (by 3).
        assert_eq!(shifted("(lam (lam (foo $0 $1 $2)))", 3), "(lam (lam (foo $0 $1 $5)))");
    }
}
