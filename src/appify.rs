use egg::{ENodeOrVar, FromOp, Id, Symbol, Var};

use crate::{
    lang::{Op, StitchEgraph, StitchLang},
    pattern::Pattern,
};

pub trait Appifiable<O>: egg::Language + FromOp {
    fn get_op(&self) -> O;
    fn get_children(&self) -> &[egg::Id];
    fn construct(op: O, children: Vec<egg::Id>) -> Self;
    fn leaf(op: O) -> Self {
        Self::construct(op, vec![])
    }
    fn is_app(&self) -> bool;
}

impl Appifiable<Op> for StitchLang {
    fn get_op(&self) -> Op {
        self.op
    }
    fn get_children(&self) -> &[egg::Id] {
        &self.children
    }
    fn construct(op: Op, children: Vec<egg::Id>) -> Self {
        StitchLang { op, children }
    }
    fn is_app(&self) -> bool {
        self.op.as_str() == "@"
    }
}

#[derive(Clone, Debug)]
enum OpPlus<O> {
    Op(O),
    Var(Var),
}

impl<L, O> Appifiable<OpPlus<O>> for ENodeOrVar<L>
where
    L: Appifiable<O>,
    O: Clone,
{
    fn get_op(&self) -> OpPlus<O> {
        match self {
            ENodeOrVar::ENode(node) => OpPlus::Op(node.get_op()),
            ENodeOrVar::Var(var) => OpPlus::Var(*var),
        }
    }

    fn get_children(&self) -> &[egg::Id] {
        match self {
            ENodeOrVar::ENode(node) => node.get_children(),
            ENodeOrVar::Var(_) => &[],
        }
    }

    fn construct(op: OpPlus<O>, children: Vec<egg::Id>) -> Self {
        match op {
            OpPlus::Op(op) => ENodeOrVar::ENode(L::construct(op, children)),
            OpPlus::Var(var) => {
                // TODO this is a hack, we shouldn't be using symbols here
                let symbol_version = L::from_op(&format!("{:?}", var), children).expect("Failed to construct var node");
                ENodeOrVar::ENode(symbol_version)
            }
        }
    }

    fn is_app(&self) -> bool {
        match self {
            ENodeOrVar::ENode(node) => node.is_app(),
            ENodeOrVar::Var(_) => false,
        }
    }
}

pub fn insert_apps<L, O>(expr: egg::RecExpr<L>) -> egg::RecExpr<L>
where
    L: Appifiable<O>,
    O: Clone,
{
    let mut new_expr = egg::RecExpr::default();

    add_with_apps(&mut new_expr, &expr, expr.len() - 1);

    new_expr
}

fn add_with_apps<L, O>(new_expr: &mut egg::RecExpr<L>, expr: &egg::RecExpr<L>, ptr: usize) -> egg::Id
where
    L: Appifiable<O>,
    O: Clone,
{
    let node = &expr.as_ref()[ptr];
    if node.get_children().is_empty() {
        // leaf node
        return new_expr.add(node.clone());
    }
    // 1+ children. Need to perform appification.
    let mut current = new_expr.add(L::leaf(node.get_op().clone()));
    for &child in node.get_children() {
        let child_node = L::from_op("@", vec![current, add_with_apps(new_expr, expr, child.into())]).expect("Failed to create app node");
        current = new_expr.add(child_node);
    }
    current
}

pub fn remove_apps<L, O>(expr: egg::RecExpr<L>) -> egg::RecExpr<L>
where
    L: Appifiable<O>,
    O: Clone,
{
    let mut new_expr = egg::RecExpr::default();
    let root = expr.as_ref().len() - 1;
    add_without_apps(&mut new_expr, &expr, root);
    new_expr
}

fn add_without_apps<L, O>(new_expr: &mut egg::RecExpr<L>, expr: &egg::RecExpr<L>, mut ptr: usize) -> egg::Id
where
    L: Appifiable<O>,
    O: Clone,
{
    let expr_ref = &expr.as_ref();
    let mut children_reversed = vec![];
    while expr_ref[ptr].is_app() {
        let app_node = &expr.as_ref()[ptr];
        assert!(app_node.get_children().len() == 2, "App nodes should have exactly two children");
        children_reversed.push(add_without_apps(new_expr, expr, app_node.get_children()[1].into()));
        ptr = app_node.get_children()[0].into();
    }
    let node = &expr_ref[ptr];
    assert!(!node.is_app(), "Should have removed all apps by now");
    assert!(node.get_children().is_empty(), "Non-app nodes should have no children; but received {:?}", node);
    let node = L::construct(node.get_op().clone(), children_reversed.into_iter().rev().collect());
    new_expr.add(node)
}

pub fn remove_apps_in_pattern(pat: Pattern) -> Pattern {
    Pattern {
        pattern: remove_apps::<ENodeOrVar<_>, OpPlus<Op>>(pat.pattern.into()).into(),
        vars: pat.vars,
    }
}

pub fn construct_appified_stub(sym: Symbol, children: Vec<Id>, egraph: &mut StitchEgraph) -> Id {
    let mut current = egraph.add(StitchLang::leaf(Op::Sym(sym)));
    for child in children {
        current = egraph.add(StitchLang::construct(Op::Sym("@".into()), vec![current, child]));
    }
    current
}
