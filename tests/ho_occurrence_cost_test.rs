//! Tests for `LanguageFamily::ho_occurrence_cost` — the per-occurrence cost of
//! the eta-wrap added for a higher-order metavar.

use egg::{Id, RecExpr};
use egg_stitch::cost::compute_recexpr_size;
use egg_stitch::lang::{LambdaCalc, LambdaCalcLanguage, LanguageFamily, Op, OpChildrenLanguage, OpDB, OpWithVar, TsOp, TypeScript, Weights};

const W: Weights = Weights { sym_var_cost: 2, app_cost: 5, lam_cost: 7 };

/// Cost of the nodes an η-wrap adds around `head`, measured from the built
/// expression: the wrapped subtree's size minus the head's own size.
fn wrap_size<L: egg_stitch::lang::StitchLanguage>(expr: &RecExpr<L>, wrapped: Id, head: Id) -> u32 {
    (compute_recexpr_size(expr, wrapped, &W) - compute_recexpr_size(expr, head, &W)) as u32
}

#[test]
fn typescript_charges_one_app_however_many_indices() {
    assert_eq!(TypeScript::ho_occurrence_cost(1, &W), 5 + 2);
    assert_eq!(TypeScript::ho_occurrence_cost(2, &W), 5 + 2 * 2);
    assert_eq!(TypeScript::ho_occurrence_cost(3, &W), 5 + 3 * 2);
}

#[test]
fn typescript_cost_matches_the_wrap_it_builds() {
    for h in 1..=4u32 {
        let mut r: RecExpr<OpChildrenLanguage<OpWithVar<TsOp>>> = RecExpr::default();
        let head = r.add(TypeScript::make_var::<TsOp>(egg::Var::from(0u32)));
        let db_args: Vec<i32> = (0..h as i32).rev().collect();
        let wrapped = TypeScript::wrap_pattern_with_db_apps::<TsOp>(&mut r, head, &db_args);
        assert_eq!(wrap_size(&r, wrapped, head), TypeScript::ho_occurrence_cost(h, &W), "ho_occurrence_cost({h}) must equal the wrap the family actually builds");
    }
}

#[test]
fn lambda_calc_charges_one_app_per_index() {
    assert_eq!(LambdaCalc::ho_occurrence_cost(1, &W), 5 + 2);
    assert_eq!(LambdaCalc::ho_occurrence_cost(2, &W), 2 * (5 + 2));
    assert_eq!(LambdaCalc::ho_occurrence_cost(3, &W), 3 * (5 + 2));
}

#[test]
fn lambda_calc_cost_matches_the_wrap_it_builds() {
    for h in 1..=4u32 {
        let mut r: RecExpr<LambdaCalcLanguage<OpWithVar<OpDB<Op>>>> = RecExpr::default();
        let head = r.add(LambdaCalc::make_var::<OpDB<Op>>(egg::Var::from(0u32)));
        let db_args: Vec<i32> = (0..h as i32).rev().collect();
        let wrapped = LambdaCalc::wrap_pattern_with_db_apps::<OpDB<Op>>(&mut r, head, &db_args);
        assert_eq!(wrap_size(&r, wrapped, head), LambdaCalc::ho_occurrence_cost(h, &W), "ho_occurrence_cost({h}) must equal the wrap the family actually builds");
    }
}
