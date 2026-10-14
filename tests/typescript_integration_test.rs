//! End-to-end tests for `--language typescript`: de Bruijn handling across
//! multi-slot binders (`lam{n}`, `define`) and higher-order capture. Corpora
//! live in `data/test/ts/`. The outputs are checked two ways:
//!
//! 1. **Scope invariants**, verified using the egg/egg-stitch's parser and
//!    free-variable rule (`cost::recexpr_fv`). The invariants are:
//!    - Every library `lambda` must be closed.
//!    - Every rewritten program must have the same free variables as its original.
//!    - Every call site must be one flat `(app fn_k …)` carrying exactly `arity_k`
//!      arguments.
//! 2. Asserts on expected outputs for small corpora.

use std::collections::BTreeSet;

use egg::{Id, RecExpr};
use egg_stitch::cost::recexpr_fv;
use egg_stitch::lang::{OpChildrenLanguage, StitchLanguage, TsOp};
use serde_json::Value;

mod common;

const DIR: &str = "data/test/ts";

// ─── running ────────────────────────────────────────────────────────────────

/// One `--language typescript` run under the stitch convention
/// (`--check-slow`, so any fast/slow cost mismatch panics the binary).
fn run_ts(search: &str, corpus: &str, extra: &[&str]) -> Value {
    run_ts_n(search, corpus, "1", extra)
}

/// Like [`run_ts`] but with `--num-abstractions n`, for stacked libraries.
fn run_ts_n(search: &str, corpus: &str, n: &str, extra: &[&str]) -> Value {
    let input = format!("{DIR}/{corpus}.json");
    let mut args = vec!["--language", "typescript"];
    args.extend_from_slice(extra);
    common::run_with_abstractions(search, &input, n, &args)
}

// ─── accessors ──────────────────────────────────────────────────────────────

/// The run's `library` array (empty when no abstraction was found).
fn library(run: &Value) -> Vec<Value> {
    run.get("library").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// A string-array field such as `original_programs` / `rewritten_programs`.
fn programs(run: &Value, key: &str) -> Vec<String> {
    run[key].as_array().unwrap_or_else(|| panic!("missing {key}")).iter().map(|p| p.as_str().expect("program string").to_string()).collect()
}

/// The `lambda` field of library entry `i`.
fn lambda(run: &Value, i: usize) -> String {
    library(run)[i]["lambda"].as_str().expect("lambda string").to_string()
}

/// The `arity` field of library entry `i`.
fn arity(run: &Value, i: usize) -> usize {
    library(run)[i]["arity"].as_u64().expect("arity") as usize
}

/// Runs one corpus under both backends and checks every invariant on each.
/// Returns `(best_first, smc)`.
fn run_both_checked(corpus: &str) -> (Value, Value) {
    let bf = run_ts("best-first", corpus, &[]);
    let smc = run_ts("smc", corpus, &[]);
    assert_all_invariants(&bf, &format!("{corpus}[best-first]"));
    assert_all_invariants(&smc, &format!("{corpus}[smc]"));
    (bf, smc)
}

// ─── invariants ─────────────────────────────────────────────────────────────

/// Everything that must hold for any abstraction the search picks.
fn assert_all_invariants(run: &Value, label: &str) {
    assert!(!library(run).is_empty(), "{label}: no abstraction found; every corpus here is built to compress, and an empty library would make the oracle pass vacuously");
    assert_library_closed(run, label);
    assert_scope_preserved(run, label);
    assert_call_sites_flat(run, label);
    assert_binder_groups(run, label);
}

/// Every library `lambda` is a closed term: all `?#k` became bound DB indices
/// and no free variable from the matched context leaked into a body.
fn assert_library_closed(run: &Value, label: &str) {
    for (i, e) in library(run).iter().enumerate() {
        let l = e["lambda"].as_str().expect("lambda");
        let fv = free_vars(l);
        assert!(fv.is_empty(), "{label}: fn_{i} lambda has free vars {fv:?}: {l}");
    }
}

/// Rewriting never captures or frees a variable: each rewritten program has
/// exactly the free variables of its original.
fn assert_scope_preserved(run: &Value, label: &str) {
    let orig = programs(run, "original_programs");
    let rewr = programs(run, "rewritten_programs");
    assert_eq!(orig.len(), rewr.len(), "{label}: program count changed");
    for (i, (o, r)) in orig.iter().zip(&rewr).enumerate() {
        assert_eq!(free_vars(o), free_vars(r), "{label}: program {i} changed its free variables\n  original : {o}\n  rewritten: {r}");
    }
}

/// Every use of `fn_k` is the callee of one flat `app` with exactly `arity_k` arguments.
fn assert_call_sites_flat(run: &Value, label: &str) {
    let arities: Vec<usize> = (0..library(run).len()).map(|i| arity(run, i)).collect();
    let mut terms = programs(run, "rewritten_programs");
    terms.extend((0..arities.len()).map(|i| lambda(run, i)));
    for t in &terms {
        for u in fn_uses(t) {
            match u {
                FnUse::Call(k, argc) => assert_eq!(argc, arities[k], "{label}: fn_{k} called with {argc} args, arity is {}: {t}", arities[k]),
                FnUse::Misplaced(k) => panic!("{label}: fn_{k} appears outside a flat `(app fn_{k} …)`: {t}"),
            }
        }
    }
}

/// Every library lambda with `arity > 0` opens with exactly one `lam{arity}`,
/// and neither the library nor the rewritten programs contain a `lam0` the
/// corpus didn't have.
fn assert_binder_groups(run: &Value, label: &str) {
    let orig = programs(run, "original_programs");
    let corpus_has_lam0 = orig.iter().any(|p| p.contains("(lam0 "));
    for (i, (o, r)) in orig.iter().zip(programs(run, "rewritten_programs")).enumerate() {
        let (no, nr) = (o.matches("(lam0 ").count(), r.matches("(lam0 ").count());
        assert!(nr <= no, "{label}: rewritten program {i} has {nr} lam0s, its original only {no}\n  original : {o}\n  rewritten: {r}");
    }
    for i in 0..library(run).len() {
        let l = lambda(run, i);
        assert!(!l.starts_with("(lam0 "), "{label}: fn_{i} is wrapped in a lam0: {l}");
        assert!(corpus_has_lam0 || !l.contains("lam0"), "{label}: fn_{i} contains a lam0 the corpus never had: {l}");
        let a = arity(run, i);
        if a > 0 {
            assert!(l.starts_with(&format!("(lam{a} ")), "{label}: fn_{i} (arity {a}) should open with one lam{a}: {l}");
        }
    }
}

// ─── A. de Bruijn indices across multi-slot binders ─────────────────────────

/// A metavar that lands *inside* a `lam2` sits two binders deep, so its head
/// index is `$0 + 2 = $2`.
#[test]
fn metavar_under_lam2_is_shifted_by_two() {
    let (bf, _) = run_both_checked("lam2_metavar_depth");
    assert_eq!(lambda(&bf, 0), "(lam1 (app reduce xs z (lam2 (app max (app $2 $1) $0))))");
}

/// Metamorphic sibling of the test above: prepend unused parameters to every
/// corpus lambda (`lam2` → `lam3`, `lam10`, `lam100`). Tests the following:
/// 1. Under the rightmost convention the body's own `$1`/`$0` don't move,
///    only the abstraction's slot does (`$2` → `$n`).
/// 2. `lams_cost` is flat in n, so the compression must be identical.
#[test]
fn widening_a_binder_group_changes_only_the_slot_index() {
    let (l2, _) = run_both_checked("lam2_metavar_depth");
    for n in [3, 10, 100] {
        let (ln, _) = run_both_checked(&format!("lam{n}_metavar_depth"));
        assert_eq!(lambda(&ln, 0), format!("(lam1 (app reduce xs z (lam{n} (app max (app ${n} $1) $0))))"));
        assert_eq!(l2["final_cost"], ln["final_cost"], "a lam{n} must cost what a lam2 costs");
        assert_eq!(library(&l2)[0]["num_matches"], library(&ln)[0]["num_matches"]);
    }
}

/// Variables bound inside the abstracted window stay in the body, while those
/// bound outside it are abstracted exactly like symbols, including at the
/// boundary (the largest bound index vs. the smallest free one).
#[test]
fn barely_free_var_is_passed_like_a_symbol() {
    let (bound, _) = run_both_checked("free_var_barely_bound");
    assert_eq!(lambda(&bound, 0), "(lam1 (app reduce xs z (lam2 (app max (app $2 $1) $0))))");

    let (free, _) = run_both_checked("free_var_barely_free");
    let (sym, _) = run_both_checked("free_var_as_symbol");
    assert_eq!(lambda(&free, 0), "(lam2 (app reduce xs z (lam2 (app max (app $3 $2) $0))))");
    assert_eq!(lambda(&free, 0), lambda(&sym, 0), "a free var must abstract exactly like a symbol");
    assert_eq!(free["final_cost"], sym["final_cost"]);
    assert_eq!(library(&free)[0]["num_matches"], library(&sym)[0]["num_matches"]);
    for (i, (f, s)) in programs(&free, "rewritten_programs").iter().zip(programs(&sym, "rewritten_programs")).enumerate() {
        assert_eq!(*f, s.replace(&format!("y{}", i + 1), "$0"), "program {i}: the free var should sit where the symbol does");
    }
}

/// `() => x` extracts as `(lam0 x)`. The thunk's `lam0` is ordinary corpus
/// structure: it must survive into the body verbatim. The oracle can't check
/// this one (it desugars `(lam0 X)` to `X`, so dropping the `lam0` still
/// β-checks), which is why the string is pinned.
#[test]
fn thunk_lam0_is_kept_as_structure() {
    let (bf, _) = run_both_checked("lam0_thunk");
    assert_eq!(lambda(&bf, 0), "(lam1 (app setTimeout (lam0 (app $0 x)) ms))");
}

/// A de Bruijn variable within `define`'s first child isn't shifted by the binder
/// `define` introduces.
#[test]
fn define_value_slot_is_outside_the_let() {
    let (bf, _) = run_both_checked("define_value_slot");
    assert_eq!(lambda(&bf, 0), "(lam1 (define (app load $0) (app use $0 k)))");
}

/// A de Bruijn variable within `define`'s second child is shifted by the binder
/// `define` introduces.
#[test]
fn define_body_slot_is_inside_the_let() {
    let (bf, _) = run_both_checked("define_body_slot");
    assert_eq!(lambda(&bf, 0), "(lam1 (define (app load a) (app use $0 $1)))");
}

/// Each program reuses a variable bound *outside* the repeated window
/// (`(app bar $0 _)` under the program's own `lam1`). A body that baked in that
/// `$0` would be an open term; the closedness invariant rejects it, so the `$0`
/// must travel as an argument instead.
///
/// It is also the backward-ordering check at arity 2: slot 0 (the *first*
/// call-site argument, here the context var) is the *highest* index, `$1`.
/// The oracle alone can't see the convention: flipping both the index names
/// and the call-site argument order still β-checks, so the pair is pinned.
#[test]
fn free_context_var_never_leaks_into_a_body() {
    let (bf, smc) = run_both_checked("free_var_context");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam2 (app bar $1 (app cfg q r s) $0))", "{name}");
        assert_eq!(programs(run, "rewritten_programs")[1], "(lam1 (app baz (app fn_0 $0 d) (app fn_0 $0 e)))", "{name}: first argument fills the highest index");
    }
}

/// Flat version of lambda-calc `ho_arity2_capture`. Without currying, a flat
/// `app` only matches calls with the same number of arguments, so the slot is
/// filled only by closed symbols and stays first-order (no η-wrap), unlike the
/// higher-order slot in the lambda-calc version.
#[test]
fn flat_arity_mismatch_stays_first_order() {
    let run = run_ts("best-first", "flat_arity_mismatch", &[]);
    assert_all_invariants(&run, "flat_arity_mismatch");
    assert_eq!(arity(&run, 0), 1);
    assert_eq!(lambda(&run, 0), "(lam1 (app F e (lam2 (app P $0 (app $2 $1)))))");
    let orig = programs(&run, "original_programs");
    let rewr = programs(&run, "rewritten_programs");
    assert_eq!(rewr[0], "(app fn_0 g)", "closed filler is passed bare, not lam1-wrapped");
    assert_eq!(rewr[1], orig[1], "the 3-child call can't match the 2-child pattern");
    assert_eq!(rewr[2], "(app fn_0 k)");
}

// ─── B. higher-order capture ────────────────────────────────────────────────

/// A higher-order slot that captures both indices of a `lam2` keeps them in
/// source order, including for a filler that uses them swapped.
#[test]
fn two_index_capture_keeps_argument_order() {
    let (bf, smc) = run_both_checked("ho_two_index_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(arity(run, 0), 1, "{name}: one higher-order slot");
        assert_eq!(lambda(run, 0), "(lam1 (app F e (lam2 (app P $0 (app $2 $1 $0)))))", "{name}");
        let rewr = programs(run, "rewritten_programs");
        assert!(rewr.iter().all(|p| p.starts_with("(app fn_0 ")), "{name}: all four programs should use fn_0: {rewr:#?}");
        assert!(rewr[1].contains("(app h $0 $1)"), "{name}: captured args must keep source order: {}", rewr[1]);
    }
}

/// A continuation that uses variables bound by `define`s inside the
/// abstraction becomes one higher-order slot, with both `define`s kept in the
/// body. The exact string isn't pinned, since which indices the slot captures
/// depends on the capture analysis.
#[test]
fn continuation_capture_across_define() {
    let (bf, _) = run_both_checked("cps_define_capture");
    assert_eq!(arity(&bf, 0), 1, "one continuation slot: {}", lambda(&bf, 0));
    assert!(lambda(&bf, 0).contains("(define (app parse $0)"), "the window should keep both defines: {}", lambda(&bf, 0));
}

/// A higher-order slot captures only the binder indices its fillers use,
/// skipping unused ones. Full expansion (capturing all three, passing a
/// `lam3`) still passes the oracle, so the body and `lam2` width are pinned.
#[test]
fn partial_capture_skips_unused_binder_slots() {
    let (bf, smc) = run_both_checked("ho_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (app F e (lam3 (app P $1 (app $3 $2 $0)))))", "{name}");
        assert!(every_call_passes_lam(run, 2), "{name}: every call site should pass a lam2, not a lam3");
        let rewr = programs(run, "rewritten_programs");
        assert_eq!(rewr[1], "(app fn_0 (lam2 (app h $0 (app num_add $1 one))))", "{name}: $2 → $1, $0 stays");
        assert_eq!(rewr[3], "(app fn_0 (lam2 (if (app lt $0 $1) (app m $0) (app m $1))))", "{name}");
    }
}

/// Partial capture works across a wide binder group: using only the first and
/// last of 100 slots still captures just those two.
#[test]
fn partial_capture_under_lam100_spans_the_whole_group() {
    let (bf, _) = run_both_checked("lam100_ho_capture");
    assert_eq!(lambda(&bf, 0), "(lam1 (app F e (lam100 (app P $50 (app $100 $99 $0)))))");
    assert!(every_call_passes_lam(&bf, 2), "every call site should pass a lam2");
    assert_eq!(programs(&bf, "rewritten_programs")[3], "(app fn_0 (lam2 (app m $0 (app n $1))))");
}

/// With an inner `define` enclosed by an outer `define`, a higher-order slot in the
/// inner `define`'s first child captures the variable bound by the outer `define`,
/// not the inner `define`.
#[test]
fn inner_define_value_slot_only_captures_outer_define() {
    let (bf, smc) = run_both_checked("define_value_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app $1 $0) (app use $0))))", "{name}");
        assert!(every_call_passes_lam(run, 1), "{name}: every call site should pass a lam1");
    }
}

/// With an inner `define` enclosed by an outer `define`, a higher-order capture in
/// the inner `define`'s second child captures only the used variables.
#[test]
fn define_continuation_captures_only_the_used_let() {
    let (bf, smc) = run_both_checked("define_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app parse $0) (app $2 $1))))", "{name}");
        assert!(every_call_passes_lam(run, 1), "{name}: every call site should pass a lam1");
        assert_eq!(programs(run, "rewritten_programs")[3], "(app fn_0 (lam1 (if (app ok $0) (app send $0) pass)))", "{name}");
    }
}

/// A higher-order slot used at two binder depths shifts its deeper captures by
/// the inner binder's width, so a `lam1` or `lam2` inner binder compresses the
/// same.
#[test]
fn cross_depth_capture_shifts_by_binder_width() {
    let (l1, _) = run_both_checked("cross_depth_lam1");
    let (l2, _) = run_both_checked("cross_depth_lam2");
    assert_eq!(l1["final_cost"], l2["final_cost"], "lam1 vs lam2 inner binder must compress identically");
    assert_eq!(arity(&l1, 0), arity(&l2, 0));
    assert_eq!(library(&l1)[0]["num_matches"], library(&l2)[0]["num_matches"]);
}

// ─── C. pipeline-level ──────────────────────────────────────────────────────

/// A run that learns several abstractions keeps every invariant
/// for each library entry and the programs rewritten with it.
#[test]
fn multiple_abstractions_stay_sound() {
    for search in ["best-first", "smc"] {
        let run = run_ts_n(search, "mixed", "3", &[]);
        assert!(library(&run).len() >= 2, "{search}: expected a stacked library on the mixed corpus");
        assert_all_invariants(&run, &format!("mixed[{search}]"));
    }
}

// ─── scope checks via the egg/egg-stitch's parser and free-variable rule ─────────

/// A TypeScript-family term, as the search itself represents it.
type TsExpr = RecExpr<OpChildrenLanguage<TsOp>>;

/// Parses `s` with the crate's TypeScript parser, panicking with `s` on failure.
fn parse(s: &str) -> TsExpr {
    OpChildrenLanguage::<TsOp>::parse_program(s).unwrap_or_else(|e| panic!("{e}"))
}

/// Free de Bruijn indices of `s`, relative to its root, via `cost::recexpr_fv`.
fn free_vars(s: &str) -> BTreeSet<i32> {
    recexpr_fv(&parse(s)).pop().unwrap_or_default().into_iter().collect()
}

/// One occurrence of a library function name.
enum FnUse {
    /// `(app fn_k a_1 … a_argc)`.
    Call(usize, usize),
    /// `fn_k` anywhere else: as an op head, an argument, a bare leaf.
    Misplaced(usize),
}

/// `Some(k)` iff `op` is a library name `fn_k`.
fn fn_index(op: &TsOp) -> Option<usize> {
    match op {
        TsOp::Sym(s) => s.as_str().strip_prefix("fn_").and_then(|k| k.parse().ok()),
        _ => None,
    }
}

/// Every [`FnUse`] in `s`, in pre-order. A call recurses only into its arguments.
fn fn_uses(s: &str) -> Vec<FnUse> {
    /// Collects the uses under node `id`.
    fn go(e: &TsExpr, id: Id, out: &mut Vec<FnUse>) {
        let node = &e[id];
        out.extend(fn_index(&node.op).map(FnUse::Misplaced));
        let mut rest = node.children.as_slice();
        if let (TsOp::App, [callee, args @ ..]) = (&node.op, rest)
            && e[*callee].children.is_empty()
            && let Some(k) = fn_index(&e[*callee].op)
        {
            out.push(FnUse::Call(k, args.len()));
            rest = args;
        }
        rest.iter().for_each(|&c| go(e, c, out));
    }
    let e = parse(s);
    let mut out = Vec::new();
    go(&e, Id::from(e.as_ref().len() - 1), &mut out);
    out
}

/// True iff every rewritten program is a call `(app fn_0 (lam{n} …))` whose
/// first argument is an η-wrap of width `n`. Lets a test assert how much a
/// higher-order slot captured.
fn every_call_passes_lam(run: &Value, n: usize) -> bool {
    programs(run, "rewritten_programs").iter().all(|p| p.starts_with(&format!("(app fn_0 (lam{n} ")))
}

/// Small tests for the test helper functions. (The binder rules `free_vars` relies on are tested in `tests/fv_analysis_test.rs`.)
#[test]
fn call_site_helpers_classify_correctly() {
    assert!(matches!(fn_uses("(app fn_0 (fn_1 a) (app fn_2))").as_slice(), [FnUse::Call(0, 2), FnUse::Misplaced(1), FnUse::Call(2, 0)]));
    let run = |p: &str| serde_json::json!({ "rewritten_programs": [p] });
    assert!(every_call_passes_lam(&run("(app fn_0 (lam2 $0))"), 2));
    assert!(!every_call_passes_lam(&run("(app fn_0 (lam20 $0))"), 2), "lam2 must not match lam20");
}
