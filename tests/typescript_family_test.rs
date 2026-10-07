//! Tests for the `TypeScript` family: a flat n-ary language whose
//! binders and applications are real (`TsOp::Lam(n)` / `TsOp::App`) rather than
//! absent as in `OpChildren`.
//!
//! The two `*_matches_the_egraph_size_delta` tests are the important ones. Each
//! cost hook is contractually "the summed node cost of the enodes this family
//! inserts", and `check_fast_vs_slow` compares that arithmetic against a
//! rebuilt e-graph — so a cost function that disagrees with its own constructor
//! is a live bug, not a tuning preference.

use egg::{Id, RecExpr};
use egg_stitch::lang::{LanguageFamily, OpChildrenLanguage, OpWithVar, StitchAnalysis, StitchDisc, StitchEgraph, StitchOp, TsOp, TypeScript, Weights};

type Lang = OpChildrenLanguage<TsOp>;

fn egraph(weights: Weights) -> StitchEgraph<Lang> {
    StitchEgraph::new(StitchAnalysis::new(weights))
}

fn leaf(g: &mut StitchEgraph<Lang>, name: &str) -> Id {
    g.add(OpChildrenLanguage { op: TsOp::from_name(name), children: vec![] })
}

#[test]
fn ts_op_round_trips_the_ops_the_family_builds() {
    // `TypeScript` constructs its binder and application through
    // `from_name` rather than a dedicated constructor, so a rename on either
    // side of this round trip would silently downgrade them to opaque symbols:
    // no compile error, no panic, just wrong costs and nodes that never unify
    // with the corpus. This test is the guard for that.
    assert_eq!(TsOp::from_name("app"), TsOp::App);
    assert_eq!(TsOp::from_name("lam2"), TsOp::Lam(2));
    assert_eq!(TsOp::App.to_string(), "app");
    assert_eq!(TsOp::Lam(2).to_string(), "lam2");
}

#[test]
fn wrap_lams_adds_one_node_that_binds_n() {
    let mut g = egraph(Weights::default());
    let body = leaf(&mut g, "x");
    let wrapped = TypeScript::wrap_lams::<TsOp>(body, 3, &mut g);
    assert_ne!(wrapped, g.find(body), "wrap must produce a new eclass");
    let node = g[wrapped].nodes.first().expect("wrapped eclass is non-empty").clone();
    assert_eq!(node.op, TsOp::Lam(3), "one Lam(3) node, not three Lam(1)s");
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
        let wrapped = TypeScript::wrap_lams::<TsOp>(body, n, &mut g);
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
        let stub = TypeScript::add_stub_application::<TsOp>("fn_0", kids, &mut g);
        let spine = g[stub].data.size - kid_total;
        assert_eq!(spine, TypeScript::stub_application_size(arity, &w), "stub_application_size({arity}) must equal the spine the e-graph actually grew by");
    }
}

#[test]
fn stub_application_is_one_flat_app_over_the_callee() {
    let mut g = egraph(Weights::default());
    let a = leaf(&mut g, "a");
    let b = leaf(&mut g, "b");
    let stub = TypeScript::add_stub_application::<TsOp>("fn_0", vec![a, b], &mut g);
    let node = g[stub].nodes.first().expect("stub eclass is non-empty").clone();
    assert_eq!(node.op, TsOp::App, "a call is an App node, not a head-as-op node");
    assert_eq!(node.children.len(), 3, "children are [callee, a, b] — flat, not curried");
    assert_eq!(g[node.children[0]].nodes.first().unwrap().op, TsOp::from_name("fn_0"));
}

type PatLang = OpChildrenLanguage<OpWithVar<TsOp>>;

fn pat_leaf(r: &mut RecExpr<PatLang>, op: OpWithVar<TsOp>) -> Id {
    r.add(OpChildrenLanguage { op, children: vec![] })
}

fn metavar_head(r: &mut RecExpr<PatLang>, k: u32) -> Id {
    pat_leaf(r, OpWithVar::Var(egg::Var::from(k)))
}

fn round_trip(db_args: &[i32]) {
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let wrapped = TypeScript::wrap_pattern_with_db_apps::<TsOp>(&mut r, head, db_args);
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<TsOp>(r.as_ref(), wrapped), head, "unwrap should recover the metavar head for db_args = {db_args:?}");
}

#[test]
fn wrap_pattern_builds_one_flat_app() {
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let wrapped = TypeScript::wrap_pattern_with_db_apps::<TsOp>(&mut r, head, &[1, 0]);
    let node = &r.as_ref()[usize::from(wrapped)];
    assert_eq!(node.op, OpWithVar::Node(TsOp::App), "one App node, not a curried chain");
    assert_eq!(node.children.len(), 3, "children are [head, $1, $0]");
    assert_eq!(node.children[0], head);
}

#[test]
fn wrap_pattern_with_no_args_is_the_identity() {
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    assert_eq!(TypeScript::wrap_pattern_with_db_apps::<TsOp>(&mut r, head, &[]), head);
}

#[test]
fn round_trip_ho_arity_1() {
    round_trip(&[0]);
}

#[test]
fn round_trip_ho_arity_2_contiguous() {
    round_trip(&[1, 0]);
}

#[test]
fn round_trip_ho_arity_3_contiguous() {
    round_trip(&[2, 1, 0]);
}

#[test]
fn round_trip_ho_arity_2_noncontiguous() {
    // Non-contiguous vis like `[0, 2]` → `db_args = [2, 0]`.
    round_trip(&[2, 0]);
}

#[test]
fn genuine_application_with_non_metavar_head_is_left_alone() {
    // `(app f $1 $0)` has the exact shape of an eta-wrap, but its head is a
    // regular op rather than a metavar. Collapsing it would silently rewrite a
    // real call into its callee.
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let f = pat_leaf(&mut r, OpWithVar::Node(TsOp::from_name("f")));
    let v1 = pat_leaf(&mut r, OpWithVar::Node(TsOp::Var(1)));
    let v0 = pat_leaf(&mut r, OpWithVar::Node(TsOp::Var(0)));
    let app = r.add(OpChildrenLanguage {
        op: OpWithVar::Node(TsOp::App),
        children: vec![f, v1, v0],
    });
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<TsOp>(r.as_ref(), app), app);
}

#[test]
fn ascending_db_args_are_not_an_eta_wrap() {
    // The wrapper always emits strictly descending indices. An ascending run is
    // some other term that happens to be var-headed; leave it alone.
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    let v0 = pat_leaf(&mut r, OpWithVar::Node(TsOp::Var(0)));
    let v1 = pat_leaf(&mut r, OpWithVar::Node(TsOp::Var(1)));
    let app = r.add(OpChildrenLanguage {
        op: OpWithVar::Node(TsOp::App),
        children: vec![head, v0, v1],
    });
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<TsOp>(r.as_ref(), app), app);
}

#[test]
fn metavar_alone_is_left_alone() {
    let mut r: RecExpr<PatLang> = RecExpr::default();
    let head = metavar_head(&mut r, 0);
    assert_eq!(TypeScript::unwrap_pattern_db_apps::<TsOp>(r.as_ref(), head), head);
}

#[test]
fn display_pattern_wraps_body_in_one_lam_node() {
    // Pattern body `(app f ?#0)` with arity 1: `?#0` becomes `$0` and the whole
    // thing is wrapped in a single `lam1`, not a stack of `lam1`s.
    // Nodes are in `RevExpr` order — root at index 0, children after it —
    // because that is what `Pattern::display_as_lambda` passes in.
    let hole = Id::from(2);
    let nodes: Vec<PatLang> = vec![
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::App),
            children: vec![Id::from(1), hole],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::from_name("f")),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(0u32)),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<TsOp>(&nodes, &[vec![hole]], &[0], &[vec![]]);

    assert!(rendered.starts_with("(lam1 "), "expected a single lam1 wrapper, got {rendered}");
    assert!(!rendered.contains("lam1 (lam1"), "binders must not be stacked, got {rendered}");
    assert!(rendered.contains("$0"), "the metavar should render as a de Bruijn leaf, got {rendered}");
}

#[test]
fn display_pattern_uses_one_lam_node_for_arity_two() {
    // Two slots share one binder node: `lam2`, never `(lam1 (lam1 …))`.
    let h0 = Id::from(2);
    let h1 = Id::from(3);
    let nodes: Vec<PatLang> = vec![
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::App),
            children: vec![Id::from(1), h0, h1],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::from_name("f")),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(0u32)),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(1u32)),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<TsOp>(&nodes, &[vec![h0], vec![h1]], &[0, 0], &[vec![], vec![]]);

    assert!(rendered.starts_with("(lam2 "), "expected a single lam2 wrapper, got {rendered}");
    assert!(!rendered.contains("lam1"), "arity-2 must be one lam2, not nested lam1s, got {rendered}");
}

#[test]
fn display_pattern_with_zero_arity_has_no_binder() {
    // A zero-arity abstraction is a closed term: there is no slot to bind, so
    // the body is returned bare rather than wrapped in a `lam0` that `TsOp`
    // would parse as `Lam(0)` — a binder that binds nothing.
    let nodes: Vec<PatLang> = vec![
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::App),
            children: vec![Id::from(1), Id::from(2)],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::from_name("f")),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::from_name("x")),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<TsOp>(&nodes, &[], &[], &[]);

    assert_eq!(rendered, "(app f x)", "arity 0 must render the body unwrapped");
}

#[test]
fn display_pattern_numbers_slots_right_to_left() {
    // `(app f ?#0 ?#1)` at arity 2: slot 0 is the *first* parameter and so gets
    // the *highest* index. Asserting on the exact string is the point — a
    // swapped `arity - 1 - k` still produces a well-formed `lam2`.
    let h0 = Id::from(2);
    let h1 = Id::from(3);
    let nodes: Vec<PatLang> = vec![
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::App),
            children: vec![Id::from(1), h0, h1],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::from_name("f")),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(0u32)),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(1u32)),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<TsOp>(&nodes, &[vec![h0], vec![h1]], &[0, 0], &[vec![], vec![]]);

    assert_eq!(rendered, "(lam2 (app f $1 $0))", "slot k must render as $(arity - 1 - k)");
}

#[test]
fn display_pattern_shifts_captured_indices_by_occurrence_depth() {
    // Two occurrences of the same higher-order slot at different binder depths.
    // Both capture the corpus index `$0`, but the deeper one sits under one
    // extra `lam1`, so its captured argument must shift up by that delta while
    // the shallower one does not — this is the `occ_shift` arithmetic, and it
    // is invisible to any test whose occurrences all sit at `var_depth`.
    //
    //   (app (lam1 ?#0) (lam1 (lam1 ?#0)))   with var_depth[0] = 1
    let shallow = Id::from(3);
    let deep = Id::from(5);
    let nodes: Vec<PatLang> = vec![
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::App),
            children: vec![Id::from(1), Id::from(2)],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::Lam(1)),
            children: vec![shallow],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::Lam(1)),
            children: vec![Id::from(4)],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(0u32)),
            children: vec![],
        },
        OpChildrenLanguage {
            op: OpWithVar::Node(TsOp::Lam(1)),
            children: vec![deep],
        },
        OpChildrenLanguage {
            op: OpWithVar::Var(egg::Var::from(0u32)),
            children: vec![],
        },
    ];

    let rendered = TypeScript::display_pattern_as_lambda::<TsOp>(&nodes, &[vec![shallow, deep]], &[1], &[vec![0]]);

    assert!(rendered.contains("(app $1 $0)"), "the occurrence at var_depth captures $0 unshifted, got {rendered}");
    assert!(rendered.contains("(app $2 $1)"), "the occurrence one binder deeper captures $1, got {rendered}");
}
