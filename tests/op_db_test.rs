//! Tests for the `OpDB<O>` leaf-op wrapper: parsing, display, and trait
//! delegation.

use egg_stitch::lang::{Op, OpDB, OpWithVar, StitchDisc, StitchOp, Weights};

#[test]
fn from_name_parses_var() {
    let v: OpDB<Op> = OpDB::<Op>::from_name("$0");
    assert_eq!(v, OpDB::Var(0));
    let v: OpDB<Op> = OpDB::<Op>::from_name("$42");
    assert_eq!(v, OpDB::Var(42));
}

#[test]
fn from_name_falls_through_to_inner_op_for_non_var() {
    let f: OpDB<Op> = OpDB::<Op>::from_name("foo");
    match f {
        OpDB::Node(_) => {}
        OpDB::Var(_) => panic!("plain symbol should not parse as Var"),
    }
    // `$foo` has a non-numeric tail and must fall through to the inner op,
    // not be silently swallowed.
    let f: OpDB<Op> = OpDB::<Op>::from_name("$foo");
    match f {
        OpDB::Node(_) => {}
        OpDB::Var(_) => panic!("`$foo` should not parse as Var"),
    }
}

#[test]
fn display_var_uses_dollar_prefix() {
    assert_eq!(OpDB::<Op>::Var(0).to_string(), "$0");
    assert_eq!(OpDB::<Op>::Var(7).to_string(), "$7");
}

#[test]
fn display_node_delegates() {
    let inner = Op::from_name("foo");
    let outer = OpDB::<Op>::Node(inner);
    assert_eq!(outer.to_string(), "foo");
}

#[test]
fn de_bruijn_index_reports_index_for_var_only() {
    assert_eq!(OpDB::<Op>::Var(3).de_bruijn_index(), Some(3));
    assert_eq!(OpDB::<Op>::Node(Op::from_name("foo")).de_bruijn_index(), None);
}

#[test]
fn plain_op_has_no_de_bruijn_index() {
    assert_eq!(Op::from_name("foo").de_bruijn_index(), None);
    // Default `binds_child` is false for any j.
    assert!(!Op::from_name("foo").binds_child(0));
}

#[test]
fn op_with_var_forwards_de_bruijn_index() {
    // OpWithVar<OpDB<Op>> is the pattern-side leaf type: `?x` meta-vars are
    // OpWithVar::Var; `$n` De Bruijn vars are OpWithVar::Node(OpDB::Var(n)).
    let pat_var: OpWithVar<OpDB<Op>> = OpWithVar::Var(egg::Var::from(0));
    assert_eq!(pat_var.de_bruijn_index(), None);
    assert_eq!(pat_var.as_var(), Some(egg::Var::from(0)));

    let db_var: OpWithVar<OpDB<Op>> = OpWithVar::Node(OpDB::Var(2));
    assert_eq!(db_var.de_bruijn_index(), Some(2));
    assert_eq!(db_var.as_var(), None);
}

#[test]
fn intrinsic_size_uses_sym_var_cost() {
    // Both Var and Node leaves should fall under the leaf cost slot.
    let weights = Weights { sym_var_cost: 5, app_cost: 1, lam_cost: 1 };
    assert_eq!(OpDB::<Op>::Var(0).intrinsic_size(&weights), 5);
    assert_eq!(OpDB::<Op>::Node(Op::from_name("foo")).intrinsic_size(&weights), 5);
}
