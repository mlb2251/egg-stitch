use egg::{FromOp, Id};
use egg_stitch::lang::{Op, StitchEgraph, StitchLang};

fn fv_of(prog: &str) -> Vec<u32> {
    let expr: egg::RecExpr<StitchLang> = prog.parse().unwrap();
    let mut eg: StitchEgraph = egg::EGraph::default();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    let mut v: Vec<u32> = eg[id].data.fv.iter().copied().collect();
    v.sort();
    v
}

#[test]
fn from_op_tags_binding_nodes() {
    assert_eq!(StitchLang::from_op("$3", vec![]).unwrap().op, Op::Var(3));
    assert_eq!(StitchLang::from_op("lam", vec![Id::from(0)]).unwrap().op, Op::Lam);
    // Non-numeric `$…` stays opaque.
    assert!(matches!(StitchLang::from_op("$foo", vec![]).unwrap().op, Op::Sym(_)));
    assert!(matches!(StitchLang::from_op("foo", vec![]).unwrap().op, Op::Sym(_)));
}

#[test]
fn fv_var() {
    assert_eq!(fv_of("$0"), vec![0]);
    assert_eq!(fv_of("$3"), vec![3]);
}

#[test]
fn fv_lam_binds_zero() {
    assert_eq!(fv_of("(lam $0)"), Vec::<u32>::new());
    assert_eq!(fv_of("(lam $1)"), vec![0]);
    assert_eq!(fv_of("(lam (lam $1))"), Vec::<u32>::new());
    assert_eq!(fv_of("(lam (lam $2))"), vec![0]);
}

#[test]
fn fv_union_across_children() {
    assert_eq!(fv_of("(+ $0 $2)"), vec![0, 2]);
}

#[test]
fn size_includes_lam_and_var() {
    let expr: egg::RecExpr<StitchLang> = "(lam (+ $0 1))".parse().unwrap();
    let mut eg: StitchEgraph = egg::EGraph::default();
    let id = eg.add_expr(&expr);
    eg.rebuild();
    // lam, +, $0, 1 → 4 nodes
    assert_eq!(eg[id].data.size, 4);
}
