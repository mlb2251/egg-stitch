use egg::{ENodeOrVar, Id, Symbol};
use egg_stitch::lang::{Op, StitchLang};
use egg_stitch::pattern::Pattern;

/// Build a StitchLang enode with `arity` placeholder children. `expand` overwrites
/// the children, so the dummy Ids here are never read.
fn op(name: &str, arity: usize) -> StitchLang {
    StitchLang { op: Op::Sym(Symbol::from(name)), children: vec![Id::from(0); arity] }
}

fn lam() -> StitchLang {
    StitchLang { op: Op::Lam, children: vec![Id::from(0); 1] }
}

/// Asserts the canonical-form invariant: every id in `vars[k]` holds `Var(k)`,
/// and nothing in `vars` is non-Var.
fn assert_vars_canonical(p: &Pattern) {
    for (k, ids) in p.vars.iter().enumerate() {
        let expected = egg::Var::from(k as u32);
        for id in ids {
            match &p.pattern[*id] {
                ENodeOrVar::Var(v) => assert_eq!(*v, expected, "vars[{}] = {:?}: expected {:?}, got {:?}", k, ids, expected, v),
                other => panic!("vars[{}] contains non-Var: {:?}", k, other),
            }
        }
    }
}

#[test]
fn single_var_is_canonical() {
    let p = Pattern::single_var();
    assert_eq!(p.vars.len(), 1);
    assert_eq!(p.to_string(), "?#0");
    assert_vars_canonical(&p);
}

#[test]
fn expand_fresh_var_binary() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2));
    assert_eq!(p.vars.len(), 2);
    assert_eq!(p.to_string(), "(+ ?#0 ?#1)");
    assert_vars_canonical(&p);
}

#[test]
fn expand_nested_left_first() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
    p.expand(0, &op("-", 2)); // (+ (- ?#0 ?#1) ?#2)
    assert_eq!(p.to_string(), "(+ (- ?#0 ?#1) ?#2)");
    assert_eq!(p.vars.len(), 3);
    assert_vars_canonical(&p);
}

#[test]
fn expand_right_keeps_earlier_vars_first() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
    p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
    assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#2))");
    assert_eq!(p.vars.len(), 3);
    assert_vars_canonical(&p);
}

#[test]
fn expand_ternary() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("f", 3));
    assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#2)");
    assert_eq!(p.vars.len(), 3);
    assert_vars_canonical(&p);
}

#[test]
fn reuse_adjacent() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
    p.reuse(0, 1); // (+ ?#0 ?#0)
    assert_eq!(p.to_string(), "(+ ?#0 ?#0)");
    assert_eq!(p.vars.len(), 1);
    assert_vars_canonical(&p);
}

#[test]
fn reuse_normalizes_reversed_args() {
    let mut p1 = Pattern::single_var();
    p1.expand(0, &op("+", 2));
    p1.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
    p1.reuse(0, 2);

    let mut p2 = Pattern::single_var();
    p2.expand(0, &op("+", 2));
    p2.expand(1, &op("*", 2));
    p2.reuse(2, 0); // reversed

    assert_eq!(p1.to_string(), "(+ ?#0 (* ?#1 ?#0))");
    assert_eq!(p1.to_string(), p2.to_string());
    assert_eq!(p1.vars.len(), p2.vars.len());
    assert_vars_canonical(&p1);
    assert_vars_canonical(&p2);

    // Downstream expansion should agree: "var 0" must mean the same thing in both.
    p1.expand(0, &op("h", 1));
    p2.expand(0, &op("h", 1));
    assert_eq!(p1.to_string(), p2.to_string());
    assert_vars_canonical(&p1);
    assert_vars_canonical(&p2);
}

#[test]
fn reuse_with_intervening_var() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("f", 3)); // (f ?#0 ?#1 ?#2)
    p.reuse(0, 2); // (f ?#0 ?#1 ?#0)
    assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#0)");
    assert_eq!(p.vars.len(), 2);
    assert_vars_canonical(&p);
}

#[test]
fn expand_reused_var_preserves_dag_sharing() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
    p.reuse(0, 1); // (+ ?#0 ?#0)
    assert_eq!(p.vars.len(), 1);
    p.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))
    assert_eq!(p.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
    assert_eq!(p.vars.len(), 2);
    assert_vars_canonical(&p);

    // The two new vars must each have a single RecExpr slot (DAG sharing),
    // not one per tree occurrence.
    assert_eq!(p.vars[0].len(), 1);
    assert_eq!(p.vars[1].len(), 1);
}

#[test]
fn expand_then_reuse_across_structure() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
    p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
    p.reuse(1, 2); // (+ ?#0 (* ?#1 ?#1))
    assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#1))");
    assert_eq!(p.vars.len(), 2);
    assert_vars_canonical(&p);
}

#[test]
fn single_var_depth_is_zero() {
    let p = Pattern::single_var();
    assert_eq!(p.var_depth, vec![0]);
}

#[test]
fn expand_lam_bumps_child_depth() {
    let mut p = Pattern::single_var();
    p.expand(0, &lam());
    assert_eq!(p.to_string(), "(lam ?#0)");
    assert_eq!(p.var_depth, vec![1]);
    p.expand(0, &lam());
    assert_eq!(p.to_string(), "(lam (lam ?#0))");
    assert_eq!(p.var_depth, vec![2]);
}

#[test]
fn expand_non_lam_keeps_depth() {
    let mut p = Pattern::single_var();
    p.expand(0, &lam()); // (lam ?#0), depths [1]
    p.expand(0, &op("+", 2)); // (lam (+ ?#0 ?#1)), depths [1, 1]
    assert_eq!(p.to_string(), "(lam (+ ?#0 ?#1))");
    assert_eq!(p.var_depth, vec![1, 1]);
}

#[test]
fn expand_mixed_depths_are_independent() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1), depths [0, 0]
    p.expand(0, &lam()); // (+ (lam ?#0) ?#1), depths [1, 0]
    assert_eq!(p.to_string(), "(+ (lam ?#0) ?#1)");
    assert_eq!(p.var_depth, vec![1, 0]);
}

#[test]
fn reuse_at_equal_depth_ok() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // depths [0, 0]
    p.reuse(0, 1);
    assert_eq!(p.to_string(), "(+ ?#0 ?#0)");
    assert_eq!(p.var_depth, vec![0]);
}

#[test]
#[should_panic(expected = "differing binder depths")]
fn reuse_at_unequal_depth_panics() {
    let mut p = Pattern::single_var();
    p.expand(0, &op("+", 2)); // (+ ?#0 ?#1), depths [0, 0]
    p.expand(0, &lam()); // (+ (lam ?#0) ?#1), depths [1, 0]
    p.reuse(0, 1); // different depths → reject
}

#[test]
fn to_string_distinguishes_non_equivalent_shapes() {
    let mut a = Pattern::single_var();
    a.expand(0, &op("+", 2));
    a.reuse(0, 1); // (+ ?#0 ?#0)
    a.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))

    let mut b = Pattern::single_var();
    b.expand(0, &op("+", 2));
    b.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) ?#2)
    b.expand(2, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#2 ?#3))

    assert_ne!(a.to_string(), b.to_string());
    assert_eq!(a.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
    assert_eq!(b.to_string(), "(+ (* ?#0 ?#1) (* ?#2 ?#3))");
    assert_vars_canonical(&a);
    assert_vars_canonical(&b);
}
