//! End-to-end tests for `--language typescript`: de Bruijn handling across
//! multi-slot binders (`lam{n}`, `define`) and higher-order capture.
//!
//! `typescript_family_test.rs` unit-tests the family hooks one at a time. These
//! tests run the whole binary (`load_egraph` → search → rewrite → display) on
//! small hand-written corpora and check the output three independent ways:
//!
//! 1. **Scope invariants**, computed by the tiny s-expression walker at the
//!    bottom of this file. It encodes the binder rules (`lam{n}` binds n slots
//!    over its body; `define` binds 1 over its *second* child) independently of
//!    `TsOp::binds_child`, so a bug there can't hide by agreeing with itself.
//!    Every library `lambda` must be closed, every rewritten program must have
//!    the same free variables as its original, and every call site must be one
//!    flat `(app fn_k …)` carrying exactly `arity_k` arguments.
//! 2. **The β oracle** (`scripts/check_equiv.py --flat`): inline the library
//!    into each rewritten program, β-normalise, and compare with the original.
//!    This is what catches a wrong de Bruijn index. The oracle's flat desugaring
//!    is deliberately non-injective (`(lam2 X)` = `(lam1 (lam1 X))`,
//!    `(f a)` = `(app f a)`), which is why (1) also checks the flat shapes.
//! 3. **Pinned shapes** on corpora small enough to reason about by hand. Each
//!    pinned string was checked against the oracle, and so was its most likely
//!    off-by-one mutant (the oracle rejects every one of those mutants).
//!
//! Corpora live in `data/test/ts/`, not `data/domains/`: the follow-reaches
//! sweep feeds `data/domains/` to reference stitch, which can't parse the flat
//! dialect.
//!
//! All conventions follow the settled rightmost rule: in `(lam{n} …)`, `$0` is
//! the *last* parameter.

use std::collections::BTreeSet;
use std::process::Command;

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
/// Returns `(best_first, smc)` so callers can pin shapes on top.
fn run_both_checked(corpus: &str) -> (Value, Value) {
    let bf = run_ts("best-first", corpus, &[]);
    let smc = run_ts("smc", corpus, &[]);
    assert_all_invariants(&bf, &format!("{corpus}[best-first]"));
    assert_all_invariants(&smc, &format!("{corpus}[smc]"));
    (bf, smc)
}

// ─── invariants ─────────────────────────────────────────────────────────────

/// Everything that must hold for *any* abstraction the search picks.
fn assert_all_invariants(run: &Value, label: &str) {
    assert!(!library(run).is_empty(), "{label}: no abstraction found; every corpus here is built to compress, and an empty library would make the oracle pass vacuously");
    assert_library_closed(run, label);
    assert_scope_preserved(run, label);
    assert_call_sites_flat(run, label);
    assert_binder_groups(run, label);
    assert_oracle_accepts(run, label);
}

/// Every library `lambda` is a closed term: all `?#k` became bound DB indices
/// and no free variable from the matched context leaked into a body.
fn assert_library_closed(run: &Value, label: &str) {
    for (i, e) in library(run).iter().enumerate() {
        let l = e["lambda"].as_str().expect("lambda");
        let fv = free_vars(&parse(l));
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
        assert_eq!(free_vars(&parse(o)), free_vars(&parse(r)), "{label}: program {i} changed its free variables\n  original : {o}\n  rewritten: {r}");
    }
}

/// Every use of `fn_k` — in rewritten programs and inside later library
/// bodies — is the callee of one flat `app` with exactly `arity_k` arguments.
/// Guards the stub shape the β oracle can't see (it reads `(fn_0 a)` and
/// `(app fn_0 a)` as the same term).
fn assert_call_sites_flat(run: &Value, label: &str) {
    let arities: Vec<usize> = (0..library(run).len()).map(|i| arity(run, i)).collect();
    let mut terms = programs(run, "rewritten_programs");
    terms.extend((0..arities.len()).map(|i| lambda(run, i)));
    for t in &terms {
        let mut uses = Vec::new();
        fn_uses(&parse(t), &mut uses);
        for u in uses {
            match u {
                FnUse::Call(k, argc) => assert_eq!(argc, arities[k], "{label}: fn_{k} called with {argc} args, arity is {}: {t}", arities[k]),
                FnUse::Misplaced(k) => panic!("{label}: fn_{k} appears outside a flat `(app fn_{k} …)`: {t}"),
            }
        }
    }
}

/// A lambda with `arity > 0` opens with exactly one `lam{arity}` binder group,
/// and the search never *invents* a `lam0` (a binder that binds nothing).
/// A `lam0` copied from the corpus is fine: ts-codec emits one for every
/// zero-parameter arrow (`() => x`), so it is only suspicious when no original
/// program contains one.
fn assert_binder_groups(run: &Value, label: &str) {
    let corpus_has_lam0 = programs(run, "original_programs").iter().any(|p| p.contains("(lam0 "));
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

/// Runs the flat β oracle on `run`. Skipped (with a note) when `python3` is
/// missing locally; a hard failure under CI, where losing it loses coverage.
fn assert_oracle_accepts(run: &Value, label: &str) {
    let path = std::env::temp_dir().join(format!("egg-stitch-ts-oracle-{}-{}.json", std::process::id(), label.replace(['/', '[', ']'], "_")));
    std::fs::write(&path, serde_json::to_string(run).unwrap()).unwrap();
    let out = Command::new("python3").args(["scripts/check_equiv.py", "--flat"]).arg(&path).output();
    let _ = std::fs::remove_file(&path);
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            assert!(std::env::var_os("CI").is_none(), "{label}: python3 unavailable under CI: {e}");
            eprintln!("{label}: skipping β oracle ({e})");
            return;
        }
    };
    assert!(out.status.success(), "{label}: β oracle rejected the run:\n{}", String::from_utf8_lossy(&out.stdout));
}

// ─── A. de Bruijn indices across multi-slot binders ─────────────────────────

/// A metavar that lands *inside* a `lam2` sits two binders deep, so its head
/// index is `$0 + 2 = $2`. An implementation that bumps depth by one per binder
/// *node* (rather than by `binds_child`) renders `$1`, which the oracle rejects.
#[test]
fn metavar_under_lam2_is_shifted_by_two() {
    let (bf, _) = run_both_checked("lam2_metavar_depth");
    assert_eq!(lambda(&bf, 0), "(lam1 (app reduce xs z (lam2 (app max (app $2 $1) $0))))");
}

/// Metamorphic sibling of the test above: prepend unused parameters to every
/// corpus lambda (`lam2` → `lam3`, and the silly widths `lam10`, `lam100`).
/// Tests the following:
/// 1. Under the rightmost convention the body's own `$1`/`$0` don't move,
///    only the abstraction's slot does (`$2` → `$n`).
/// 2. `lams_cost` is flat in n, so the compression must be identical.
/// 3. Nothing assumes a single-digit width (`lam100` parses, prints, and
///    shifts by 100).
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

/// Free-variable boundary. Every program repeats a window under its own
/// `lam1`; inside the window's `lam2`, the varying callee is applied to either
/// `$1` (the *largest bound* index at depth 2) or `$2` (the *smallest free*
/// one: it points at the program's `lam1`, outside the window).
///
/// - `$1` is bound, so it is baked into the body (arity 1).
/// - `$2` is free, so it must behave exactly like a symbol that is constant
///   within a program but differs between programs: the same lambda, the same
///   cost, and the call site passes it (shifted down by 2, to `$0`) in the
///   position the symbol twin passes `y{i}`.
///
/// A `<=` for `<` in the free-var test would bake `$2` into the body (caught by
/// the closedness invariant); a shift of 1 instead of 2 would pass `$1` (caught
/// by the oracle).
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

/// `define` binds its *second* child. A slot in the value position (depth 0)
/// and the body's own `$0` both print as `$0` but point at different binders:
/// `(lam1 (define (app load $0) (app use $0 k)))`. If `define` bound child 0,
/// the slot would print as `$1` and the oracle would reject it.
#[test]
fn define_value_slot_is_outside_the_let() {
    let (bf, _) = run_both_checked("define_value_slot");
    assert_eq!(lambda(&bf, 0), "(lam1 (define (app load $0) (app use $0 k)))");
}

/// The other side of the same rule: a slot in the `define` body is one binder
/// deep, so it is `$1` while the let-bound value stays `$0`.
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

// ─── B. higher-order capture ────────────────────────────────────────────────

/// The existing one-index fixture (`stitch/ts_arity2_capture`) re-checked
/// against the scope invariants, which its snapshot doesn't assert.
#[test]
fn one_index_capture_existing_fixture() {
    let run = common::run("best-first", "data/domains/ho-bugs/ts_arity2_capture.json", &["--language", "typescript"]);
    assert_all_invariants(&run, "ts_arity2_capture");
}

/// The two-index path the snapshot manifest calls out as uncovered: the hole
/// `(app g $1 $0)` references *both* slots of the enclosing `lam2`, so `?#0`
/// captures two indices and renders as `(app $2 $1 $0)`.
///
/// Program 1 is the asymmetric one. Its captured term `(app h $0 $1)` has the
/// arguments flipped, so it can't η-reduce to `h`; the call-site argument has
/// to be exactly `(lam2 (app h $0 $1))`. Reversing the `db_args` order (or the
/// slot convention) still gives well-formed output but swaps h's arguments
/// after β, which the oracle catches.
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

/// The CPS shape: a two-`define` window whose continuation refers to variables
/// bound *inside* the window. The abstraction must take the rest of the
/// computation as a callback that re-binds both let-values, e.g.
/// `(lam1 (define (app fetch u) (define (app parse $0) (app $2 $1 $0))))`.
/// Which indices a slot captures depends on the capture analysis, so the exact
/// string isn't pinned: the invariants and the oracle carry this one.
#[test]
fn continuation_capture_across_define() {
    let (bf, _) = run_both_checked("cps_define_capture");
    assert_eq!(arity(&bf, 0), 1, "one continuation slot: {}", lambda(&bf, 0));
    assert!(lambda(&bf, 0).contains("(define (app parse $0)"), "the window should keep both defines: {}", lambda(&bf, 0));
}

/// Partial η-expansion. Inside a `lam3`, every filler uses the first and last
/// parameters (`$2`, `$0`) in a different non-trivial way (nested, duplicated,
/// swapped, under an `if`) and never the middle one. The wrap must capture
/// only what's used: the body applies `(app $3 $2 $0)` and every call site
/// passes a `lam2`, with the filler's `$2` renumbered to `$1`.
///
/// Full expansion (capture all three, pass a `lam3`) is semantically fine,
/// so the oracle accepts it; the pinned body and the `lam2` width are what
/// reject it. The oracle does reject a wrong or swapped captured index.
#[test]
fn partial_capture_skips_unused_binder_slots() {
    let (bf, smc) = run_both_checked("ho_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (app F e (lam3 (app P $1 (app $3 $2 $0)))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(2); 4], "{name}: every call site should pass a lam2, not a lam3");
        let rewr = programs(run, "rewritten_programs");
        assert_eq!(rewr[1], "(app fn_0 (lam2 (app h $0 (app num_add $1 one))))", "{name}: $2 → $1, $0 stays");
        assert_eq!(rewr[3], "(app fn_0 (lam2 (if (app lt $0 $1) (app m $0) (app m $1))))", "{name}");
    }
}

/// The silly-width version of the test above: a `lam100` whose fillers use
/// only `$99` (first parameter) and `$0` (last). The capture is `[99, 0]`,
/// 98 slots apart, so the call site is still a `lam2`.
#[test]
fn partial_capture_under_lam100_spans_the_whole_group() {
    let (bf, _) = run_both_checked("lam100_ho_capture");
    assert_eq!(lambda(&bf, 0), "(lam1 (app F e (lam100 (app P $50 (app $100 $99 $0)))))");
    assert_eq!(call_site_wraps(&bf), vec![Some(2); 4]);
    assert_eq!(programs(&bf, "rewritten_programs")[3], "(app fn_0 (lam2 (app m $0 (app n $1))))");
}

/// η-expansion through `define`'s *value* child. The value of the inner
/// `define` sits under the outer `define` only, so its depth is 1 (not 2):
/// the hole renders as `(app $1 $0)`, capturing the outer let. If `define`
/// were treated as binding its value child, or both children, the head would
/// be `$2` and the oracle would reject it.
#[test]
fn define_value_slot_captures_the_outer_let() {
    let (bf, smc) = run_both_checked("define_value_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app $1 $0) (app use $0))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(1); 4], "{name}");
    }
}

/// Partial η-expansion across two `define`s. The continuation only ever uses
/// the *outer* let (`$1`), never the inner one (`$0`). So the capture is `[1]`:
/// the body applies `(app $2 $1)` and the call sites pass a `lam1`. Compare
/// `continuation_capture_across_define`, whose programs use both between them
/// and so capture both.
#[test]
fn define_continuation_captures_only_the_used_let() {
    let (bf, smc) = run_both_checked("define_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app parse $0) (app $2 $1))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(1); 4], "{name}");
        assert_eq!(programs(run, "rewritten_programs")[3], "(app fn_0 (lam1 (if (app ok $0) (app send $0) done)))", "{name}");
    }
}

/// One higher-order slot used at two binder depths: once directly under the
/// program's `lam1`, once under an extra inner binder. The deeper occurrence's
/// captured index must shift by that binder's slot count (`occ_shift`). The
/// `lam2` sibling changes the inner binder to bind two slots, so the shift is
/// 2 rather than 1. That's the case a "+1 per binder" assumption gets wrong.
/// Costs must match exactly, since `lams_cost` is flat in n.
#[test]
fn cross_depth_capture_shifts_by_binder_width() {
    let (l1, _) = run_both_checked("cross_depth_lam1");
    let (l2, _) = run_both_checked("cross_depth_lam2");
    assert_eq!(l1["final_cost"], l2["final_cost"], "lam1 vs lam2 inner binder must compress identically");
    assert_eq!(arity(&l1, 0), arity(&l2, 0));
    assert_eq!(library(&l1)[0]["num_matches"], library(&l2)[0]["num_matches"]);
}

// ─── C. pipeline-level ──────────────────────────────────────────────────────

/// Stacked libraries: later bodies may call earlier `fn_k`, which the oracle
/// inlines transitively and `assert_call_sites_flat` checks for arity.
#[test]
fn stacked_abstractions_stay_sound() {
    for search in ["best-first", "smc"] {
        let run = run_ts_n(search, "mixed", "3", &[]);
        assert!(library(&run).len() >= 2, "{search}: expected a stacked library on the mixed corpus");
        assert_all_invariants(&run, &format!("mixed[{search}]"));
    }
}

/// Differential test: lower-bound pruning must never discard the optimum. With
/// `TsOp`'s flat costs, a `lams_cost` or `ho_occurrence_cost` that over-states
/// its node count makes the bound unsound, and pruning then changes the
/// result.
#[test]
fn lower_bound_pruning_preserves_the_optimum() {
    for corpus in ["lam2_metavar_depth", "define_body_slot", "ho_two_index_capture", "cps_define_capture", "cross_depth_lam2", "ho_partial_capture", "define_value_capture", "free_var_barely_free"] {
        let on = run_ts("best-first", corpus, &["--lower-bound", "on"]);
        let off = run_ts("best-first", corpus, &["--lower-bound", "off"]);
        assert_eq!(on["final_cost"], off["final_cost"], "{corpus}: pruning changed the optimum");
    }
}

/// When best-first exhausts its frontier on a tiny corpus, its answer is
/// optimal, so SMC can match it but never beat it. SMC beating it means
/// best-first mis-scored or dropped a candidate.
#[test]
fn smc_never_beats_a_converged_best_first() {
    for corpus in ["lam2_metavar_depth", "define_value_slot", "ho_two_index_capture", "cross_depth_lam1"] {
        let bf = run_ts("best-first", corpus, &[]);
        let converged = bf["heap_sizes_at_end"].as_array().is_some_and(|h| h.iter().all(|x| x == 0));
        if !converged {
            continue;
        }
        let smc = run_ts("smc", corpus, &[]);
        assert!(smc["final_cost"].as_u64() >= bf["final_cost"].as_u64(), "{corpus}: SMC beat an exhausted best-first");
    }
}

/// `--follow` on a flat higher-order pattern: the η-wrapped display
/// `(app ?#0 $1 $0)` must parse through the default flat parser and be
/// collapsed by `unwrap_pattern_db_apps` when matched.
#[test]
fn follow_reaches_two_index_capture() {
    use clap::Parser;
    use egg_stitch::lang::{LanguageFamily, TsOp, TypeScript};
    use egg_stitch::{Args, io, smc};
    use rand::SeedableRng;

    let follow = "(app F e (lam2 (app P $0 (app ?#0 $1 $0))))";
    let input = format!("{DIR}/ho_two_index_capture.json");
    let args = Args::parse_from(["egg-stitch", "--search", "smc", "--input", &input, "--language", "typescript", "--num-steps", "500", "--num-particles", "500", "--temperature", "100", "--follow", follow]);
    let (data, _, _) = io::load_egraph::<TypeScript, TsOp>(&args.input, None, false, args.weights, args.iter_limit, args.node_limit);
    let result = smc::smc(data, &args, &mut rand::rngs::StdRng::seed_from_u64(0));
    let target = TypeScript::parse_follow_pattern::<TsOp>(follow).expect("parse follow");
    let (cost, best) = result.best.as_ref().expect("SMC should reach the follow target");
    assert!(best.state.matches_follow(&target), "best (cost {cost}) {} should match {follow}", best.state.pattern);
}

// ─── independent scope walker ───────────────────────────────────────────────

/// A parsed flat s-expression.
#[derive(Debug)]
enum Sx {
    Atom(String),
    List(Vec<Sx>),
}

/// Parses one s-expression (no quoting or comments, which the dialect doesn't use).
fn parse(s: &str) -> Sx {
    let toks: Vec<String> = s.replace('(', " ( ").replace(')', " ) ").split_whitespace().map(String::from).collect();
    let mut pos = 0;
    let sx = parse_at(&toks, &mut pos);
    assert_eq!(pos, toks.len(), "trailing tokens in {s:?}");
    sx
}

/// Recursive-descent step for [`parse`].
fn parse_at(toks: &[String], pos: &mut usize) -> Sx {
    let tok = &toks[*pos];
    *pos += 1;
    if tok != "(" {
        return Sx::Atom(tok.clone());
    }
    let mut items = Vec::new();
    while toks[*pos] != ")" {
        items.push(parse_at(toks, pos));
    }
    *pos += 1;
    Sx::List(items)
}

/// Slots that head `head` binds over its `j`-th argument (head excluded):
/// `lam{n}` binds n over its body, `define` binds 1 over its second child.
/// Written out here rather than borrowed from `TsOp::binds_child` on purpose.
fn binds(head: &str, j: usize) -> i32 {
    match (head, j) {
        ("define", 1) => 1,
        (h, 0) => h.strip_prefix("lam").and_then(|n| n.parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

/// Free de Bruijn indices of `sx`, relative to its root.
fn free_vars(sx: &Sx) -> BTreeSet<i32> {
    /// Collects free indices of `sx`, which sits under `depth` binders.
    fn go(sx: &Sx, depth: i32, out: &mut BTreeSet<i32>) {
        match sx {
            Sx::Atom(a) => {
                if let Some(n) = a.strip_prefix('$').and_then(|r| r.parse::<i32>().ok())
                    && n >= depth
                {
                    out.insert(n - depth);
                }
            }
            Sx::List(items) => {
                let head = match items.first() {
                    Some(Sx::Atom(h)) => h.as_str(),
                    _ => "",
                };
                for (j, child) in items.iter().skip(1).enumerate() {
                    go(child, depth + binds(head, j), out);
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    go(sx, 0, &mut out);
    out
}

/// One occurrence of a library function name.
enum FnUse {
    /// `(app fn_k a_1 … a_argc)`.
    Call(usize, usize),
    /// `fn_k` anywhere else: as an op head, an argument, a bare leaf.
    Misplaced(usize),
}

/// `Some(k)` iff `atom` is a library name `fn_k`.
fn fn_index(atom: &str) -> Option<usize> {
    atom.strip_prefix("fn_").and_then(|k| k.parse().ok())
}

/// Collects every [`FnUse`] in `sx`.
fn fn_uses(sx: &Sx, out: &mut Vec<FnUse>) {
    match sx {
        Sx::Atom(a) => out.extend(fn_index(a).map(FnUse::Misplaced)),
        Sx::List(items) => {
            let mut rest = &items[..];
            if let [Sx::Atom(app), Sx::Atom(callee), args @ ..] = items.as_slice()
                && app == "app"
                && let Some(k) = fn_index(callee)
            {
                out.push(FnUse::Call(k, args.len()));
                rest = args;
            }
            rest.iter().for_each(|c| fn_uses(c, out));
        }
    }
}

/// For each rewritten program that is a top-level call `(app fn_k X …)`, the
/// binder width `n` if its first argument is an η-wrap `(lam{n} …)`, else
/// `None`. Lets a test assert how much a higher-order slot captured without
/// pinning every filler.
fn call_site_wraps(run: &Value) -> Vec<Option<usize>> {
    programs(run, "rewritten_programs")
        .iter()
        .map(|p| match parse(p) {
            Sx::List(items) => match items.as_slice() {
                [Sx::Atom(app), Sx::Atom(callee), Sx::List(arg), ..] if app == "app" && fn_index(callee).is_some() => match arg.first() {
                    Some(Sx::Atom(h)) => h.strip_prefix("lam").and_then(|n| n.parse().ok()),
                    _ => None,
                },
                _ => None,
            },
            Sx::Atom(_) => None,
        })
        .collect()
}

/// The walker itself needs a test, or a bug in it would quietly weaken every
/// invariant above. These are the binder rules the whole file relies on.
#[test]
fn scope_walker_agrees_with_the_binder_rules() {
    let fv = |s: &str| free_vars(&parse(s)).into_iter().collect::<Vec<_>>();
    assert_eq!(fv("(lam2 (app f $0 $1 $2))"), vec![0], "lam2 binds $0,$1; $2 escapes as $0");
    assert_eq!(fv("(define $0 $0)"), vec![0], "define's value is outside the let");
    assert_eq!(fv("(define v $1)"), vec![0], "define's body is inside it");
    assert_eq!(fv("(app lam2 $0)"), vec![0], "`lam2` as an argument binds nothing");
    assert_eq!(fv("(lam0 $0)"), vec![0], "lam0 binds nothing");
    assert_eq!(fv("(lam100 (app f $99 $100))"), vec![0], "multi-digit widths parse");
    let wraps = call_site_wraps(&serde_json::json!({"rewritten_programs": ["(app fn_0 (lam2 $0))", "(app fn_0 f)", "(lam1 (app fn_0 (lam1 $0)))"]}));
    assert_eq!(wraps, vec![Some(2), None, None], "only a top-level call's first argument counts");
    let mut uses = Vec::new();
    fn_uses(&parse("(app fn_0 (fn_1 a) (app fn_2))"), &mut uses);
    assert!(matches!(uses.as_slice(), [FnUse::Call(0, 2), FnUse::Misplaced(1), FnUse::Call(2, 0)]));
}
