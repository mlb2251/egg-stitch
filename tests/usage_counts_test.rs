/// Regression test for `compute_usage_counts`.
///
/// The old implementation iterated eclasses in descending canonical-id order,
/// implicitly assuming that order is topological (parents before children).
/// After unions, a parent eclass's canonical id can be *lower* than one of its
/// children's, so the parent's count gets applied to the child only *after* the
/// child has already been iterated and skipped — leaving descendants of that
/// child with no count.
use egg::{FromOp, Language};
use egg_stitch::{
    lang::{Op, OpChildrenLanguage, StitchEgraph},
    search::compute_usage_counts,
};

#[test]
fn usage_counts_with_non_topological_canonical_ids() {
    let mut egraph: StitchEgraph<OpChildrenLanguage<Op>> = egg::EGraph::default();

    // Root chain: (P (a y)). The deep child (a y) starts at a low id; we will
    // bump its canonical id above the root's via a union below.
    let y = egraph.add_expr(&"y".parse().unwrap());
    let ay = egraph.add(OpChildrenLanguage::<Op>::from_op("a", vec![y]).unwrap());
    let root = egraph.add(OpChildrenLanguage::<Op>::from_op("P", vec![ay]).unwrap());

    // Separate (a z) with two parent-references via (Q (a z) (a z)). egg unions
    // by parent-count (the side with more parents becomes the canonical root),
    // so unioning (a y) with (a z) makes (a z)'s higher id win.
    let z = egraph.add_expr(&"z".parse().unwrap());
    let az = egraph.add(OpChildrenLanguage::<Op>::from_op("a", vec![z]).unwrap());
    let _q = egraph.add(OpChildrenLanguage::<Op>::from_op("Q", vec![az, az]).unwrap());

    egraph.union(ay, az);
    egraph.rebuild();

    let canon_root = egraph.find(root);
    let canon_ay = egraph.find(ay);

    // Precondition for triggering the bug: the child's canonical id exceeds the
    // root's. If egg's union direction ever changes, fail loudly rather than
    // silently passing a test that no longer exercises the bug.
    assert!(canon_ay > canon_root, "test relies on child canon > parent canon; got root={:?}, child={:?}", canon_root, canon_ay);

    let counts = compute_usage_counts(&egraph, root);

    assert_eq!(counts.get(&canon_root).copied(), Some(1), "root should have count 1");
    assert_eq!(counts.get(&canon_ay).copied(), Some(1), "child of root should have count 1");

    // The first enode of the merged (a ?) eclass has one child (y or z). Under
    // correct top-down propagation that leaf must also receive count 1; the
    // buggy implementation skips it because the child eclass was iterated
    // before its parent propagated.
    let first_child = egraph.find(egraph[canon_ay].nodes.first().unwrap().children()[0]);
    assert_eq!(counts.get(&first_child).copied(), Some(1), "leaf descendant did not receive a count — top-down propagation failed");
}
