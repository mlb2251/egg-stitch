use egg::{FromOp};

use crate::lang::{Op, StitchLang};

pub trait Appifiable: egg::Language + FromOp {
    fn get_op(&self) -> &Op;
    fn get_children(&self) -> &[egg::Id];
    fn construct(op: Op, children: Vec<egg::Id>) -> Self;  
    fn leaf(op: Op) -> Self {
        Self::construct(op, vec![])
    }
    fn is_app(&self) -> bool;
}

impl Appifiable for StitchLang {
    fn get_op(&self) -> &Op {
        &self.op
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


pub fn insert_apps<L>(expr: egg::RecExpr<L>) -> egg::RecExpr<L>
    where L: Appifiable
{
    let mut new_expr = egg::RecExpr::default();
    
    add_with_apps(&mut new_expr, &expr, expr.len() - 1);

    new_expr
}

fn add_with_apps<L>(new_expr: &mut egg::RecExpr<L>, expr: &egg::RecExpr<L>, ptr: usize) -> egg::Id
    where L: Appifiable
{
    let node = &expr.as_ref()[ptr];
    if node.get_children().len() == 0 {
        // leaf node
        return new_expr.add(node.clone());
    }
    // 1+ children. Need to perform appification.
    let mut current = new_expr.add(L::leaf(node.get_op().clone()));
    for &child in node.get_children() {
        let child_node = L::from_op("@", vec![current, add_with_apps(new_expr, expr, child.into())]).expect("Failed to create app node");
        current = new_expr.add(child_node);
        
    }
    return current;
}

pub fn remove_apps<L>(expr: egg::RecExpr<L>) -> egg::RecExpr<L>
    where L: Appifiable
{
    let mut new_expr = egg::RecExpr::default();
    let root = expr.as_ref().len() - 1;
    add_without_apps(&mut new_expr, &expr, root);
    new_expr
}

fn add_without_apps<L>(new_expr: &mut egg::RecExpr<L>, expr: &egg::RecExpr<L>, mut ptr: usize) -> egg::Id
    where L: Appifiable
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
    assert!(node.get_children().len() == 0, "Non-app nodes should have no children");
    let node = L::construct(node.get_op().clone(), children_reversed.into_iter().rev().collect());
    new_expr.add(node)
}