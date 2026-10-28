//! Tests for the `TypeScript` family: a flat n-ary language which has
//! binders (`TsDisc::Lam(n)`) and applications (`TsDisc::App`) unlike `OpChildren`.

use egg::{Id, RecExpr};
use egg_stitch::lang::{LanguageFamily, Op, OpChildrenLanguage, OpDB, OpWithVar, StitchAnalysis, StitchDisc, StitchEgraph, StitchOp, TsDisc, TypeScript, Weights};

type Lang = OpChildrenLanguage<TsDisc<OpDB<Op>>>;

fn egraph(weights: Weights) -> StitchEgraph<Lang> {
    StitchEgraph::new(StitchAnalysis::new(weights))
}

fn leaf(g: &mut StitchEgraph<Lang>, name: &str) -> Id {
    g.add(OpChildrenLanguage { op: TsDisc::from_name(name), children: vec![] })
}

type Disc = TsDisc<OpDB<Op>>;

#[test]
fn ts_disc_round_trips_structural_ops() {
    // The parser and `display_recexpr` must agree on the structural spellings,
    // or parsed programs silently get `Node` symbols where binders belong.
    for (name, op) in [("app", Disc::App), ("lam2", Disc::Lam(2)), ("define", Disc::Define), ("pass", Disc::Pass)] {
        assert_eq!(Disc::from_name(name), op);
        assert_eq!(op.to_string(), name);
    }
    assert_eq!(Disc::from_name("$3"), Disc::Node(OpDB::Var(3)), "DB vars must be delegated to the leaf op");
    assert_eq!(Disc::make_db_var(3), Some(Disc::Node(OpDB::Var(3))));
}

#[test]
fn ts_disc_delegates_leaf_queries_to_the_leaf_op() {
    // `?x` and `$n` live in the leaf op, wrapped in `TsDisc::Node`,
    // so `as_var` & `de_bruijn_index` must delegate through their `Self::Node` arm.
    let mv: TsDisc<OpWithVar<OpDB<Op>>> = TsDisc::from_name("?x");
    assert_eq!(mv.as_var(), Some("?x".parse().unwrap()));
    assert_eq!(Disc::from_name("$4").de_bruijn_index(), Some(4));
    assert_eq!(Disc::App.de_bruijn_index(), None);
}

#[test]
fn wrap_lams_adds_one_node_that_binds_n() {
    let mut g = egraph(Weights::default());
    let body = leaf(&mut g, "x");
    let wrapped = TypeScript::wrap_lams::<OpDB<Op>>(body, 3, &mut g);
    assert_ne!(wrapped, g.find(body), "wrap must produce a new eclass");
    let node = g[wrapped].nodes.first().expect("wrapped eclass is non-empty").clone();
    assert_eq!(node.op, TsDisc::Lam(3), "wrap must produce one Lam(3) node");
    assert_eq!(node.children.len(), 1);
    assert_eq!(node.op.binds_child(0), 3);
}

#[test]
fn lams_cost_is_flat_in_n() {
    let w = Weights { sym_var_cost: 1, app_cost: 1, lam_cost: 7 };
    for n in 1..=5u32 {
        assert_eq!(TypeScript::lams_cost(n, &w), 7, "lams_cost must not scale with n");
    }
}

#[test]
fn lams_cost_matches_the_egraph_size_delta() {
    let w = Weights { sym_var_cost: 1, app_cost: 1, lam_cost: 7 };
    for n in 1..=4u32 {
        let mut g = egraph(w);
        let body = leaf(&mut g, "x");
        let before = g[body].data.size;
        let wrapped = TypeScript::wrap_lams::<OpDB<Op>>(body, n, &mut g);
        let delta = g[wrapped].data.size - before;
        assert_eq!(delta, TypeScript::lams_cost(n, &w), "lams_cost({n}) must equal the size the e-graph actually grew by");
    }
}

#[test]
fn stub_application_size_is_arity_independent() {
    let w = Weights { sym_var_cost: 2, app_cost: 5, lam_cost: 1 };
    for arity in 0..=10usize {
        assert_eq!(TypeScript::stub_application_size(arity, &w), 7, "app_cost + sym_var_cost, regardless of arity");
    }
}

#[test]
fn stub_application_size_matches_the_egraph_size_delta() {
    let w = Weights { sym_var_cost: 2, app_cost: 5, lam_cost: 1 };
    for arity in 0..=4usize {
        let mut g = egraph(w);
        let kids: Vec<Id> = (0..arity).map(|i| leaf(&mut g, &format!("a{i}"))).collect();
        let kid_total: u32 = kids.iter().map(|&k| g[k].data.size).sum();
        let stub = TypeScript::add_stub_application::<OpDB<Op>>("fn_0", kids, &mut g);
        let spine = g[stub].data.size - kid_total;
        assert_eq!(spine, TypeScript::stub_application_size(arity, &w), "stub_application_size({arity}) must equal the spine the e-graph actually grew by");
    }
}

type PatternLang = OpChildrenLanguage<TsDisc<OpWithVar<OpDB<Op>>>>;

fn pattern_leaf(r: &mut RecExpr<PatternLang>, op: TsDisc<OpWithVar<OpDB<Op>>>) -> Id {
    r.add(OpChildrenLanguage { op, children: vec![] })
}

fn metavar_head(r: &mut RecExpr<PatternLang>, k: u32) -> Id {
    pattern_leaf(r, TsDisc::Node(OpWithVar::Var(egg::Var::from(k))))
}

fn assert_unwrap_inverts_wrap(db_args: &[i32]) {
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let wrapped = TypeScript::wrap_pattern_with_db_apps::<OpDB<Op>>(&mut r, head, db_args);
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), wrapped), head, "unwrap must recover the metavar head for db_args = {db_args:?}");
}

#[test]
fn wrap_pattern_builds_one_flat_app() {
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let wrapped = TypeScript::wrap_pattern_with_db_apps::<OpDB<Op>>(&mut r, head, &[1, 0]);
    let node = &r.as_ref()[usize::from(wrapped)];
    let args: Vec<_> = node.children[1..].iter().map(|&c| r.as_ref()[usize::from(c)].op.de_bruijn_index()).collect();
    assert_eq!(node.op, TsDisc::App, "wrap_pattern_with_db_apps must produce an `App` root");
    assert_eq!(node.children.len(), 3, "`App` must have 3 children: the head plus one per DB arg");
    assert_eq!(node.children[0], head);
    assert_eq!(args, [Some(1), Some(0)], "DB args must be [$1, $0]");
}

#[test]
fn wrap_pattern_with_no_args_is_the_identity() {
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    assert_eq!(TypeScript::wrap_pattern_with_db_apps::<OpDB<Op>>(&mut r, head, &[]), head);
}

#[test]
fn unwrap_inverts_wrap_ho_arity_1() {
    assert_unwrap_inverts_wrap(&[0]);
}

#[test]
fn unwrap_inverts_wrap_ho_arity_2_contiguous() {
    assert_unwrap_inverts_wrap(&[1, 0]);
}

#[test]
fn unwrap_inverts_wrap_ho_arity_3_contiguous() {
    assert_unwrap_inverts_wrap(&[2, 1, 0]);
}

#[test]
fn unwrap_inverts_wrap_ho_arity_2_noncontiguous() {
    // Non-contiguous vis like `[0, 2]` → `db_args = [2, 0]`.
    assert_unwrap_inverts_wrap(&[2, 0]);
}

#[test]
fn unwrap_pattern_of_non_metavar_headed_app_is_the_identity() {
    // `(app f $1 $0)` has the shape of an eta-wrap, but its head is a
    // regular op rather than a metavar. Collapsing it would silently rewrite a
    // real call into its callee.
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let f = pattern_leaf(&mut r, TsDisc::from_name("f"));
    let dbvar1 = pattern_leaf(&mut r, TsDisc::from_name("$1"));
    let dbvar0 = pattern_leaf(&mut r, TsDisc::from_name("$0"));
    let app = r.add(OpChildrenLanguage { op: TsDisc::App, children: vec![f, dbvar1, dbvar0] });
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), app), app);
}

#[test]
fn unwrap_pattern_of_ascending_db_args_is_the_identity() {
    // The wrapper always emits strictly descending indices. An ascending run is
    // some other term that happens to be var-headed; leave it alone.
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let v0 = pattern_leaf(&mut r, TsDisc::from_name("$0"));
    let v1 = pattern_leaf(&mut r, TsDisc::from_name("$1"));
    let app = r.add(OpChildrenLanguage { op: TsDisc::App, children: vec![head, v0, v1] });
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), app), app);
}

#[test]
fn unwrap_pattern_of_bare_metavar_is_the_identity() {
    let mut r: RecExpr<PatternLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<OpDB<Op>>(r.as_ref(), head), head);
}

#[test]
fn display_pattern_with_zero_arity_has_no_binder() {
    // A zero-arity abstraction is rendered by 'display_pattern_as_lambda'
    // as its bare body, not `(lam0 ...)`, which would parse back as a `Lam(0)`
    // binder that binds nothing.
    let nodes: Vec<PatternLang> = vec![
        OpChildrenLanguage {
            op: TsDisc::App,
            children: vec![Id::from(1), Id::from(2)],
        },
        OpChildrenLanguage { op: TsDisc::from_name("f"), children: vec![] },
        OpChildrenLanguage { op: TsDisc::from_name("x"), children: vec![] },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<OpDB<Op>>(&nodes, &[], &[], &[]);

    assert_eq!(rendered, "(app f x)", "arity 0 must render the body unwrapped");
}

#[test]
fn display_pattern_shifts_captured_indices_by_occurrence_depth() {
    // `(app (lam1 ?#0) (lam1 (lam1 ?#0)))` with `var_depth[0] = 1`, capturing `$0`.
    // The deeper occurrence sits under one extra binder, so its captured index
    // must shift from `$0` to `$1` (`occ_shift`); the shallower one is unshifted.
    let shallow = Id::from(3);
    let deep = Id::from(5);
    let nodes: Vec<PatternLang> = vec![
        OpChildrenLanguage {
            op: TsDisc::App,
            children: vec![Id::from(1), Id::from(2)],
        },
        OpChildrenLanguage { op: TsDisc::Lam(1), children: vec![shallow] },
        OpChildrenLanguage { op: TsDisc::Lam(1), children: vec![Id::from(4)] },
        OpChildrenLanguage {
            op: TsDisc::Node(OpWithVar::Var(egg::Var::from(0u32))),
            children: vec![],
        },
        OpChildrenLanguage { op: TsDisc::Lam(1), children: vec![deep] },
        OpChildrenLanguage {
            op: TsDisc::Node(OpWithVar::Var(egg::Var::from(0u32))),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<OpDB<Op>>(&nodes, &[vec![shallow, deep]], &[1], &[vec![0]]);

    assert!(rendered.contains("(app $1 $0)"), "the occurrence at var_depth captures $0 unshifted, got {rendered}");
    assert!(rendered.contains("(app $2 $1)"), "the occurrence one binder deeper captures $1, got {rendered}");
}
