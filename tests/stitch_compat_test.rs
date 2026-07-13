//! Tests ported from `../Stitch.jl/tests/` and `../stitch/tests/`
//! (the `data/basic/` folders in both repos).
//!
//! Each test invokes the `egg-stitch` binary twice — once with best-first
//! (10 000 expansions) and once with SMC (1 000 particles × 1 000 steps,
//! temperature 1 000), both with `--check-slow` enabled — pipes each run's
//! `--output` JSON to a temp file, and compares the two results against a
//! frozen fixture (`foo.json` → `foo.out.json`). When both backends agree the
//! fixture is just the shared `RunResult`; when they diverge the fixture is
//! `{"best-first": <RunResult>, "smc": <RunResult>}`. Non-deterministic and
//! input-dependent fields (`timestamp`, `elapsed_secs`, `input_file`,
//! `rules_file`) and per-algorithm bookkeeping (`search`, plus
//! `num_steps_run` / `num_expansions` / `best_iteration` on each library
//! entry) are stripped before comparison.
//!
//! Only the smallest, most deterministic cases are included — egg-stitch's SMC
//! is stochastic, so we pick corpora where both backends reliably converge.
//! Most of the remaining `basic/` cases use DeBruijn indices (`$0`, `$1`, ...)
//! or symbol variables (`%1`, `&x:0`). egg-stitch parses these as plain symbol
//! tokens with no scope awareness, so any abstraction it finds over them isn't
//! semantically valid under Stitch.jl's lambda-calculus reading — we skip all
//! such fixtures. A handful (`simple_hof`, `safe_ctx_thread_bug`) also have a
//! list in operator position, which egg's `RecExpr` parser rejects outright.
//! `minimum-matches-seq` depends on Stitch.jl's special `/seq` matching.
//!
//! From `../stitch/data/basic/` we additionally port `simple3`; `simple4`,
//! `simple5`, and `lio_test*` use DeBruijn vars, `symbol_weighting_test_*`
//! depends on per-symbol cost weights, and the rest duplicate Stitch.jl
//! fixtures we already cover.
//!
//! To regenerate all fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test stitch_compat_test -- --test-threads=1
//! ```

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

fn expected_path(input: &str) -> String {
    // Mirror the input layout under `data/expected_outputs/`: `data/domains/<...>`
    // and the hand-built `data/test/<...>` corpora both map by stripping `data/`.
    let relative = input.strip_prefix("data/domains/").or_else(|| input.strip_prefix("data/")).expect("expected input under data/domains/ or data/");
    let stem = relative.strip_suffix(".json").unwrap_or(relative);
    format!("data/expected_outputs/{stem}.out.json")
}

/// Path for the temporary `--output` JSON of a single backend run. Uses pid +
/// a process-global atomic counter so it's unique per *call*, not just per
/// (input, search): several tests run best-first on the same input (e.g. tests
/// that vary only CLI flags), and a shared path would let parallel tests race
/// on the same file. Stem + search are kept only for human readability.
fn temp_output_path(input: &str, search: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("egg-stitch-compat-{}-{}-{}-{}.json", std::process::id(), stem, search, n))
}

/// Invokes the cargo-built binary, writes its `--output` JSON to a temp file,
/// reads it back, and strips non-deterministic fields. Best-first uses the
/// default 50 000-step budget.
fn run_backend(search: &str, input: &str, extra_args: &[&str]) -> Value {
    run_backend_steps(search, input, "50000", extra_args)
}

/// Like [`run_backend`] but with an explicit best-first `--num-steps` budget
/// (ignored by SMC). Used for corpora where 50 000 steps is impractical — e.g.
/// an e-graph cyclic *through a binder*, whose pattern search space is unbounded
/// and whose per-expansion work grows super-linearly.
fn run_backend_steps(search: &str, input: &str, bf_steps: &str, extra_args: &[&str]) -> Value {
    let out = temp_output_path(input, search);
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", search, "--input", input, "--check-slow", "--num-abstractions", "1", "--output", out_str]);
    if search == "best-first" {
        cmd.args(["--num-steps", bf_steps]);
    } else {
        cmd.args(["--num-particles", "1000", "--num-steps", "1000", "--temperature", "100"]);
    }
    cmd.args(extra_args);
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "{search} run failed for {input}");

    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    let mut v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", out.display()));
    if let Some(obj) = v.as_object_mut() {
        for k in ["timestamp", "elapsed_secs", "iteration_times", "input_file", "rules_file", "search"] {
            obj.remove(k);
        }
    }
    if let Some(library) = v.get_mut("library").and_then(|l| l.as_array_mut()) {
        for entry in library {
            if let Some(obj) = entry.as_object_mut() {
                for k in ["num_steps_run", "num_expansions", "best_iteration"] {
                    obj.remove(k);
                }
            }
        }
    }
    v
}

/// Strips the `pattern` and `lambda` fields from every entry in `library` (in
/// place). Used when SMC's chosen e-class representative is non-deterministic
/// (e.g. once commutativity rewrites unify multiple equivalent pattern
/// strings). The `lambda` field is derived from the pattern so it varies
/// together — strip both to keep the comparison stable.
fn strip_library_patterns(v: &mut Value) {
    strip_library_field(v, "pattern");
    strip_library_field(v, "lambda");
}

/// Strips a named field from every entry in `library` (in place).
fn strip_library_field(v: &mut Value, key: &str) {
    let Some(library) = v.get_mut("library").and_then(|l| l.as_array_mut()) else { return };
    for entry in library {
        if let Some(obj) = entry.as_object_mut() {
            obj.remove(key);
        }
    }
}

/// Runs both backends, combines their outputs side-by-side, and checks the
/// result against the frozen fixture (or writes it under `BLESS=1`). When
/// `check_pattern` is false, the per-abstraction `pattern` string is stripped
/// from both backends' libraries before comparing.
fn check_fixture(input: &str, extra_args: &[&str], check_pattern: bool) {
    let mut bf = run_backend("best-first", input, extra_args);
    let mut smc = run_backend("smc", input, extra_args);
    // best_history is populated by best-first only; strip from both so the
    // bf/smc equality check measures search-result agreement, not trace shape.
    strip_library_field(&mut bf, "best_history");
    strip_library_field(&mut smc, "best_history");
    if !check_pattern {
        strip_library_patterns(&mut bf);
        strip_library_patterns(&mut smc);
    }
    // Collapse to a single entry when both backends agree; otherwise record
    // both side-by-side so the divergence is visible in the fixture.
    // `heap_sizes_at_end` is a best-first-only field (SMC has no heap), so a
    // bare difference on it isn't a real divergence — compare with it removed
    // and keep the best-first value (which carries the heap sizes) when they
    // otherwise agree.
    let agree = {
        let mut bf_cmp = bf.clone();
        if let Some(obj) = bf_cmp.as_object_mut() {
            obj.remove("heap_sizes_at_end");
        }
        bf_cmp == smc
    };
    let combined = if agree { bf } else { json!({"best-first": bf, "smc": smc}) };
    bless_or_check(&expected_path(input), &combined, input);
}

/// Like `check_fixture` but runs only the best-first backend. Use when SMC
/// converges unreliably for the given corpus (so its output isn't worth
/// pinning) but best-first's enumeration is still a meaningful regression
/// signal. The fixture format is the same single `RunResult` shape that
/// `check_fixture` writes when both backends already agree.
fn check_fixture_bf_only(input: &str, extra_args: &[&str], check_pattern: bool) {
    check_fixture_bf_only_steps(input, "50000", extra_args, check_pattern);
}

/// [`check_fixture_bf_only`] with an explicit best-first `--num-steps` budget,
/// for corpora where the default 50 000 is impractical (see [`run_backend_steps`]).
fn check_fixture_bf_only_steps(input: &str, bf_steps: &str, extra_args: &[&str], check_pattern: bool) {
    let mut bf = run_backend_steps("best-first", input, bf_steps, extra_args);
    strip_library_field(&mut bf, "best_history");
    if !check_pattern {
        strip_library_patterns(&mut bf);
    }
    bless_or_check(&expected_path(input), &bf, input);
}

/// Blesses/checks a single best-first run against an *explicit* fixture path,
/// keeping the abstraction pattern. Use when several configs (varying only CLI
/// flags) run on one input and so can't share the input-derived fixture path —
/// the differing patterns are the point of the comparison.
fn bless_bf_run(input: &str, extra_args: &[&str], fixture: &str, label: &str) {
    let mut bf = run_backend("best-first", input, extra_args);
    strip_library_field(&mut bf, "best_history");
    bless_or_check(fixture, &bf, label);
}

/// Shared blessing/checking step for the two `check_fixture*` helpers.
fn bless_or_check(path: &str, value: &Value, input: &str) {
    let value = common::sorted(value);
    if std::env::var("BLESS").is_ok() {
        let mut text = serde_json::to_string_pretty(&value).expect("serialize expected");
        text.push('\n');
        fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let mut expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        common::sort_keys(&mut expected);
        assert_eq!(value, expected, "fixture mismatch for {input} (run with BLESS=1 to update)");
    }
}

#[test]
fn identical() {
    check_fixture("data/domains/stitch/identical.json", &[], true);
}

/// Drop-fv regression with a divergent $0-bearing arm: three programs share
/// `(lam (lam (+ _)))` over constants A/B/C, while the fourth uses a
/// different head (`-`) over `$0`. The cost optimizer must pick the
/// drop-fv candidate (S_0 = {}, keep the three `+`-constants, leave the
/// `-`-arm unrewritten). Pinned through the planned cost-loop optimization
/// so the lower-bound prune doesn't accidentally discard the drop-fv
/// candidate.
#[test]
fn drop_fv_minus_arm() {
    check_fixture_bf_only("data/domains/ho-bugs/drop_fv_minus.json", LAMBDA, true);
}

/// Same shape as `drop_fv_minus_arm` but the divergent arm shares the `+`
/// head, so all four programs unify under `(lam (lam (+ ?#0)))`. The fourth
/// arm's `?#0 → $0` has fv `[0]`, forcing the optimizer to choose between
/// dropping that subst (drop-fv) or paying the wrap-lam cost. Head-based
/// pattern discrimination — which let `drop_fv_minus_arm` collapse on main
/// without the drop-fv mechanism — isn't available here.
#[test]
fn drop_fv_plus_arm() {
    check_fixture_bf_only("data/domains/ho-bugs/drop_fv_plus.json", LAMBDA, true);
}

/// HO-arity-2 capture regression. The η-wrap convention in `wrap_subst_args`
/// pairs with `wrap_pattern_with_db_apps`'s splice order. Pre-fix the splice
/// ran `($0 $1)` while the wrap produced bodies assuming `($1 $0)`, so
/// β-reducing a capture of local-$1 came out as local-$0. Identity at HO
/// arity 1, so all earlier HO tests passed unchanged. The β-equivalence
/// sweep in `scripts/check_all_outputs.py` catches the semantic version too.
#[test]
fn ho_arity2_capture() {
    check_fixture("data/domains/ho-bugs/arity2_capture.json", &["--language", "lambda-calc"], true);
}

/// Regression: `shift_free_egraph`'s memo is keyed by `(canonical, initial_depth)`,
/// which is only valid within a single metavar slot's transformation. When two
/// slots have different `(d_k, h, rank_map)` and share a captured-arg eclass
/// (or a sub-eclass during recursion), the cache hit from one slot reused the
/// other slot's permuted-shift result. Pre-fix the corpus below saw `$1`
/// become `$0` in slot 0's wrapped arg because slot 1's `$1 → $0` mapping
/// had been cached at the same `(canonical, initial_depth)` key. Fix: use a
/// fresh memo per slot.
#[test]
fn cross_slot_shift_memo() {
    check_fixture_bf_only("data/domains/ho-bugs/cross_slot_shift_memo.json", &["--language", "lambda-calc"], true);
}

/// Regression: the search picks `fn_{N+1}` (or higher) when the input already
/// contains a leaf named `fn_N`, so re-running stitch on an already-abstracted
/// corpus doesn't produce a name that aliases an existing symbol. The blessed
/// fixture pins the chosen index — pre-fix the new abstraction would have been
/// named `fn_0`, collapsing onto the input's existing `fn_0` symbol.
#[test]
fn fn_name_collision() {
    check_fixture("data/domains/ho-bugs/fn_name_collision.json", &[], true);
}

/// Stitch.jl finds the arity-0 body
/// `(a b c d e f g h (A B C) (A B C) (A B C) (A B C))`. Pre-fix, egg-stitch's
/// e-class equality unified the four `(A B C)` subterms and picked the
/// arity-1 `(a b c d e f g h #0 #0 #0 #0)` — but that metavar is useless
/// (every match maps `?#0` to the same closed e-class), so the
/// `has_useless_var` result gate now rejects it and the arity-0 answer is
/// returned instead.
#[test]
fn cex() {
    check_fixture("data/domains/stitch/cex.json", &[], true);
}

/// Parallel to [`cex`] with `--allow-useless-vars`. The default gate rejects
/// the arity-1 `(a b c d e f g h #0 #0 #0 #0)` — its `#0` is useless (every
/// slot binds the same closed `(A B C)`) — and falls back to the arity-0 body.
/// The flag lifts that gate (and forces the useless/dominance prunes off), so
/// the search returns the reused-useless-var abstraction instead. Best-first is
/// deterministic, so its body is pinned exactly; SMC may settle on an
/// equivalent alternate optimum, so there we only require some var reused 4×.
#[test]
fn cex_allow_useless_vars() {
    let input = "data/domains/stitch/cex.json";
    let flag = &["--allow-useless-vars"];
    let bf = run_backend("best-first", input, flag);
    assert_eq!(abstraction_bodies(&bf), vec!["(a b c d e f g h #0 #0 #0 #0)".to_string()], "best-first should return the useless-var abstraction the default gate forbids");
    let smc = run_backend("smc", input, flag);
    for r in [&bf, &smc] {
        let bodies = abstraction_bodies(r);
        assert_eq!(bodies.len(), 1, "expected one abstraction, got {bodies:?}");
        assert!(max_var_reuse(&bodies[0]) >= 4, "expected a useless var reused 4×, got body: {}", bodies[0]);
    }
}

#[test]
fn minimum_matches() {
    check_fixture("data/domains/stitch/minimum-matches.json", &[], true);
}

#[test]
fn simple1() {
    check_fixture("data/domains/stitch/simple1.json", &[], true);
}

#[test]
fn simple2() {
    check_fixture("data/domains/stitch/simple2.json", &[], true);
}

/// From `../stitch/data/basic/`. Rust stitch finds `(#0 (lam_1 (#0 #0)))` under
/// its per-primitive cost weights; under egg-stitch's unit-cost AST model the
/// compression doesn't pay, so no abstraction is returned.
#[test]
fn simple3() {
    check_fixture("data/domains/stitch/simple3.json", &[], true);
}

#[test]
fn tmp_minimal() {
    check_fixture("data/domains/stitch/tmp_minimal.json", &[], true);
}

/// Exercises shifted-variant search under lambda-calc: the two programs share
/// the subterm `(+ $0 3 4 (lam (+ $1 6 7)))` at different binding depths, so
/// any abstraction that captures it as a single metavariable must use the
/// shifted variant at the shallower occurrence.
#[test]
fn reuse_at_different_depths() {
    check_fixture("data/domains/stitch/reuse-at-different-depths.json", &["--language", "lambda-calc"], true);
    // CRITICAL: this is the whole point of the `variables-at-multiple-depths`
    // branch. The two programs share `(+ $0 3 4 (lam (+ $1 6 7)))` at depths
    // that differ by one — `$0` and `$1` are shift-variants of the same value.
    // A correct shift-aware reuse merges the two metavar occurrences into a
    // single arity-1 abstraction; any future change that loses this and falls
    // back to an arity-2 (non-reused) abstraction has reverted the branch's
    // flagship behavior. Pin arity == 1 directly so the assertion survives
    // re-blessing.
    for search in ["best-first", "smc"] {
        let v = run_backend(search, "data/domains/stitch/reuse-at-different-depths.json", &["--language", "lambda-calc"]);
        let library = v.get("library").and_then(|l| l.as_array()).unwrap_or_else(|| panic!("{search}: missing library"));
        assert_eq!(library.len(), 1, "{search}: expected exactly one abstraction, got {library:#?}");
        let arity = library[0].get("arity").and_then(|a| a.as_u64()).unwrap_or_else(|| panic!("{search}: arity missing"));
        assert_eq!(
            arity, 1,
            "{search}: shifted-variant reuse must collapse both occurrences into a single metavar (arity 1), got arity {arity} — this regresses the whole point of the variables-at-multiple-depths branch"
        );
    }
}

/// Regression: `shift_equal`'s `a == b` shortcut used to accept any same
/// e-class as reuse-compatible at any pair of depths, but a non-closed leaf
/// like `$0` at depths 0 and 2 references different binders. The unsound
/// merge produced `fn_0: (fold ?#0 1 (lam (lam (* ?#0 $1))))` whose
/// β-expansion replaced the inner `$0` with `$2` — see the matching list/
/// CI failure. The fix requires empty fv when collapsing same-id captures
/// across depths.
#[test]
fn same_leaf_different_depths_is_not_reused() {
    check_fixture("data/domains/stitch/same-leaf-different-depths.json", &["--language", "lambda-calc"], true);
}

/// Sibling of `same_leaf_different_depths_is_not_reused`, but for `shift_equal`'s
/// *recursive* same-e-class shortcut rather than the top-level `a == b` one.
/// In `(lam (lam (g (lam (f $1 $2)) (f $1 $1))))` the only abstraction is the
/// cross-depth reuse `(g (lam ?#0) ?#0)`, whose captures `(f $1 $2)` and
/// `(f $1 $1)` share the e-class `(f $1)` (fv {1}) sitting exactly at the gap
/// boundary. The buggy `fv_outside_gap` shortcut accepted that as shift-invariant,
/// so the merge was unsound: inlining `fn_0` reconstructs `(f $2 $2)` instead of
/// `(f $1 $2)`. The fix rejects the reuse, so a sound search finds *no*
/// abstraction (empty fixture, corpus unchanged). A regressed build re-discovers
/// the bad abstraction and mismatches here; `scripts/check_all_outputs.py`'s
/// β-equivalence sweep also rejects a bad re-bless.
#[test]
fn cross_depth_fv_outside_gap_is_not_reused() {
    check_fixture("data/domains/ho-bugs/cross_depth_fv_outside_gap.json", LAMBDA, true);
}

/// Exercises `--rules`: with the bidirectional `(+ 0 ?x) <=> ?x` in play,
/// the `(+ _ (* _ _))` shape aligns across all five programs (the fifth,
/// `(* 7 (* (- v) (- v)))`, gets a `(+ 0 _)`-wrapped representation in its
/// e-class so the inner `(* (- v) (- v))` becomes a match too).
#[test]
fn nested() {
    check_fixture("data/domains/stitch/nested.json", &["-r", "data/domains/stitch/nested.rewrites"], true);
}

const ARITH_RULES: &str = "data/domains/simple-arithmetic/arithmetic.rewrites";

#[test]
fn arithmetic_aplusbplusc() {
    check_fixture("data/domains/simple-arithmetic/aplusbplusc.json", &["-r", ARITH_RULES], false);
}

/// SMC-only: with 1000 particles / patience 50, SMC occasionally settles into
/// the arity-1 saddle `(+ ?#0 (* 1 2 3 4))` (cost 30) at iteration 5 and runs
/// out the patience window before discovering the arity-2 cost-28 answer. Best-
/// first finds the optimum deterministically, so pin only that.
#[test]
fn arithmetic_aplusbplus1234() {
    check_fixture_bf_only("data/domains/simple-arithmetic/aplusbplus1234.json", &["-r", ARITH_RULES], false);
}

/// End-to-end check of the `constant_folding: !numbers` directive. The corpus
/// writes one program's literal as `(+ 1 2)` and the other two as `3`; folding
/// saturates them into the same e-class, so the winning abstraction bakes the
/// constant in — `(f (g 3 x y z) ?#0)`, arity 1 — instead of having to pass the
/// differing literal as a second parameter. Best-first finds this
/// deterministically; the fixture pins that folding (parse → saturation →
/// search) actually changes the output.
#[test]
fn constant_folding_unifies_literal() {
    check_fixture_bf_only("data/domains/simple-arithmetic/const_fold.json", &["-r", "data/domains/simple-arithmetic/const_fold.rewrites"], true);
}

/// Folding firing *after* non-minimizing rewrites. The corpus writes one angle
/// as `(* 4 pi)` and another as `(* 2 tau)`; unifying them needs the whole
/// chain: `tau => (* 2 pi)` (expands, growing the term), multiplicative
/// associativity (`(* 2 (* 2 pi)) => (* (* 2 2) pi)`), then `!numbers` folding
/// (`(* 2 2) => 4`) to reach `(* 4 pi)`. The folded subterm `(* 2 2)` exists in
/// neither input — it's created mid-saturation — so this pins that folding
/// composes with other DSRs rather than only collapsing literals already
/// written adjacently. Best-first then bakes the shared angle into an arity-1
/// abstraction `(seg (rot (* 2 tau)) ?#0)`.
#[test]
fn constant_folding_after_rewrites() {
    check_fixture_bf_only("data/domains/simple-arithmetic/fold_after_rewrite.json", &["-r", "data/domains/simple-arithmetic/fold_after_rewrite.rewrites"], true);
}

/// `!integers`: folds `(+ 1 2) => 3` so the abstraction bakes in `3`, while the
/// float operation `(+ 1.5 2.5)` is out of scope and stays unfolded in the body
/// `(f (g 3 (+ 1.5 2.5) z) ?#0)`.
#[test]
fn constant_folding_integers() {
    check_fixture_bf_only("data/domains/simple-arithmetic/const_fold_integers.json", &["-r", "data/domains/simple-arithmetic/const_fold_integers.rewrites"], true);
}

/// `!floats`: folds `(+ 1.5 2.5) => 4.0` (float result, decimal preserved) to
/// unify with the literal `4.0`, while the integer operation `(+ 1 2)` is out of
/// scope and stays unfolded in the body `(f (g 4.0 (+ 1 2) z) ?#0)`.
#[test]
fn constant_folding_floats() {
    check_fixture_bf_only("data/domains/simple-arithmetic/const_fold_floats.json", &["-r", "data/domains/simple-arithmetic/const_fold_floats.rewrites"], true);
}

/// `!integersarefloats`: reads every literal as a float, so the integer operation
/// `(+ 1 2)` folds to `3.0` and the integer literal `4` unifies with `4.0` (via
/// `n => n.0`) — one abstraction `(f (g 3.0 4.0 z) ?#0)` covers programs written
/// in either form.
#[test]
fn constant_folding_integers_are_floats() {
    check_fixture_bf_only("data/domains/simple-arithmetic/const_fold_int_as_float.json", &["-r", "data/domains/simple-arithmetic/const_fold_int_as_float.rewrites"], true);
}

/// `(params (ops (cos pi)))`: the operator list pulls in the unary trig function
/// `cos` and the constant `pi`. `(cos pi)` folds — `pi => 3.141592653589793`,
/// then `cos(…) => -1.0` — unifying the first program with the literal `-1.0` the
/// others write, so the abstraction bakes the angle in: `(f (g -1.0 y z) ?#0)`.
/// Pins that the operator list reaches `sin`/`cos`/`pi`, not just arithmetic.
#[test]
fn constant_folding_ops_list_trig() {
    check_fixture_bf_only("data/domains/simple-arithmetic/fold_ops_trig.json", &["-r", "data/domains/simple-arithmetic/fold_ops_trig.rewrites"], true);
}

/// `(params (ops (+)))`: the operator list restricts folding to `+` only. `(+ 1 2)`
/// folds to `3` (unifying with the literal `3`, baked into the abstraction), but
/// `(* 4 5)` — its operator left out of the list — does NOT fold, so it fails to
/// unify with the literal `20` the other programs write and must be passed as an
/// argument (arity 2). Pins that the list is honored as an allowlist: the `+`
/// side bakes in, the excluded `*` side does not.
#[test]
fn constant_folding_ops_list_restricts() {
    check_fixture_bf_only("data/domains/simple-arithmetic/fold_ops_restrict.json", &["-r", "data/domains/simple-arithmetic/fold_ops_restrict.rewrites"], true);
}

/// `!round (params (places 6))`, exercising both directions. POSITIVE: the first
/// `g` arg (`0.7071067811865476` vs `0.707107`) rounds to the same value, so all
/// three programs unify on it (`matches 3`) and it is baked into the abstraction.
/// NEGATIVE: the second arg (`0.1234561` vs `0.1234567`) is close but rounds to
/// `0.123456` vs `0.123457` at 6 places, so it does NOT unify and stays a parameter
/// — giving the arity-2 `(f (g 0.7071067811865476 ?#0 z) ?#1)` (the baked literal
/// is one representative of the unified e-class).
#[test]
fn constant_folding_round_places6() {
    check_fixture_bf_only("data/domains/simple-arithmetic/fold_round6.json", &["-r", "data/domains/simple-arithmetic/fold_round6.rewrites"], true);
}

/// `!round (params (places 3))`: the `places` argument controls granularity, again
/// both directions. POSITIVE: the first `g` arg (`0.7071`/`0.7072`/`0.707`) all
/// collapse to `0.707` at 3 places (they would NOT unify at 6), so all three unify
/// and it is baked in. NEGATIVE: the second arg (`0.1114` vs `0.1116`) is close but
/// rounds to `0.111` vs `0.112`, so it stays a parameter — giving the arity-2
/// `(f (g 0.7072 ?#0 z) ?#1)`.
#[test]
fn constant_folding_round_places3() {
    check_fixture_bf_only("data/domains/simple-arithmetic/fold_round3.json", &["-r", "data/domains/simple-arithmetic/fold_round3.rewrites"], true);
}

#[test]
fn common_start() {
    check_fixture("data/domains/basic-apps/common-start.json", &["-r", ARITH_RULES, "--language", "lambda-calc"], true);
}

/// Regression for the `shift_equal` binder-cycle non-termination
/// (`src/shift_equal.rs`). The rules `a => (lambda a)` / `b => (lambda b)` carry
/// no metavariables, so they preserve `fv(c)=fv(MinTerm(c))`, yet each concrete
/// RHS hashconses back into the matched class — equality saturation closes two
/// distinct cycles *through the `lambda` binder*. Best-first then reaches a
/// pattern with two metavars at different binder depths capturing those cyclic
/// classes and calls `shift_equal(a, b, 0, 1)`.
///
/// Pre-fix, that call grew its `(deeper, shallower, init_depth)` key without
/// bound (one bump per binder around the cycle) and overflowed the stack —
/// aborting this run. With `init_depth` clamped at `shift_clamp` the coinduction
/// closes, so the run completes and produces a stable result. A small step
/// budget keeps it fast (the cyclic e-graph makes the search space unbounded).
#[test]
fn binder_cycle_terminates() {
    // Pre-fix the run aborts with a stack overflow inside `shift_equal`'s
    // coinduction, so `run_backend_steps`'s success assertion is itself the
    // regression signal; the fixture then pins the (now-terminating) result.
    check_fixture_bf_only_steps("data/domains/cyclic-binder/lam_self_cycle.json", "300", &["--language", "lambda-calc", "--rules", "data/domains/cyclic-binder/lam_self_cycle.rewrites"], true);
}

/// Collapse an s-expression to a sorted multiset of its atoms, discarding
/// structure. Used by `arith_rewrites` because plus is associative+commutative,
/// so several distinct abstraction shapes are all equally valid solutions —
/// comparing atom multisets accepts any of them without enumerating each tree.
fn all_symbols_hack(x: &str) -> Vec<String> {
    let x = x.replace("(", " ").replace(")", " ");
    let mut symbols: Vec<_> = x.split_whitespace().map(|s| s.to_string()).collect();
    symbols.sort();
    symbols
}

/// egg prints metavars as `?#0`; normalize to the cleaner `#0` form.
fn egg_to_stitch(s: &str) -> String {
    s.replace("?#", "#")
}

/// Returns the abstraction bodies (with `fn_N: ` prefix stripped) found in the
/// run's library.
fn abstraction_bodies(run: &Value) -> Vec<String> {
    run.get("library")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    let p = e.get("pattern").and_then(|p| p.as_str()).expect("pattern string");
                    egg_to_stitch(p.split_once(": ").expect("pattern prefixed with fn_N:").1)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Largest number of times any single metavar `#k` is reused in `body` (in
/// stitch notation, as returned by [`abstraction_bodies`]). A "useless" var —
/// bound to the same closed arg at every call site — surfaces here as a high
/// reuse count on a body the default `has_useless_var` gate would reject. Only
/// scans `#0..#7`; these corpora cap arity well below the point where `#1`
/// would substring-match `#10`.
fn max_var_reuse(body: &str) -> usize {
    (0..8).map(|k| body.matches(&format!("#{k}")).count()).max().unwrap_or(0)
}

/// Returns the top-level rewritten corpus, falling back to the supplied
/// original program list when the field is missing.
fn rewritten_corpus(run: &Value, original: &[String]) -> Vec<String> {
    if let Some(arr) = run.get("rewritten_programs").and_then(|p| p.as_array()) {
        return arr.iter().filter_map(|s| s.as_str().map(String::from)).collect();
    }
    original.to_vec()
}

#[test]
fn arith_rewrites() {
    let input = "data/domains/basic-apps/multi-arg-assoc.json";
    let extra_args = &["-r", "data/domains/basic-apps/app-arith.rewrites", "--language", "lambda-calc", "--max-arity", "0", "--seed", "0"];
    let bf = run_backend("best-first", input, extra_args);
    let smc = run_backend("smc", input, extra_args);
    let original: Vec<String> = serde_json::from_str(&fs::read_to_string(input).unwrap_or_else(|e| panic!("read {input}: {e}"))).unwrap_or_else(|e| panic!("parse {input}: {e}"));
    for r in &[bf, smc] {
        let bodies = abstraction_bodies(r);
        assert!(bodies.len() == 1, "expected exactly one abstraction");
        let abstr = all_symbols_hack(&bodies[0]);
        // Plus is associative+commutative, so several abstraction shapes are
        // equally valid. With the `<=>` rules now genuinely bidirectional, the
        // search also reaches the fully-flattened `(+ a b c d)` (one `+`); the
        // older nested 2-/3-`+` shapes remain valid too.
        if abstr != ["+", "a", "b", "c", "d"] && abstr != ["+", "+", "a", "b", "c", "d"] && abstr != ["+", "+", "+", "a", "b", "c", "d"] {
            panic!("bad abstr: {:?}", abstr);
        }
        let rewr = rewritten_corpus(r, &original).iter().map(|x| all_symbols_hack(x)).collect::<Vec<_>>();
        let rewr = rewr.iter().map(|x| x.iter().filter(|x| **x != <&str as Into<String>>::into("+")).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(
            rewr,
            vec![
                // these 3 can be rewritten
                vec!["fn_0", "g"],
                vec!["f", "fn_0"],
                vec!["e", "fn_0"],
                // this one can't be
                vec!["*", "a", "b", "c", "d", "e"]
            ]
        )
    }
}

#[test]
fn varying_head() {
    check_fixture("data/domains/basic-apps/varying-head.json", &["--language", "lambda-calc"], true);
}

#[test]
fn multiple_with_apps() {
    check_fixture("data/domains/basic-apps/multiple-with-apps.json", &["--language", "lambda-calc"], true);
}

// ---- fixtures with binders and De Bruijn variables ----
//
// These exercise the depth-tracking pattern search (`var_depth`) and the
// extract→shift→wrap step in `apply_abstraction`. Programs are parsed as
// `LambdaCalcLanguage<OpDB<Op>>` (selected by `--language lambda-calc`), so
// `lam` becomes a real binder and `$n` a real De Bruijn leaf.

/// Cost weights matching `../stitch/`'s defaults: leaves cost 100, structural
/// nodes (`@`/`lam`) cost 1. This is what stitch's own basic/ tests run under
/// and is what makes lambdas/apps essentially "free" relative to symbol
/// content, so the discovered abstractions match stitch's choices.
const STITCH_LAMBDA_ARGS: &[&str] = &["--language", "lambda-calc", "--sym-var-cost", "100"];

/// Two `(lam …)` programs sharing a structural skeleton; abstraction sits
/// under the top lambda and captures the common core around `$0`. SMC is
/// skipped — multiple near-equivalent patterns sit close enough that 1000
/// particles × 1000 steps don't converge reliably.
#[test]
fn stitch_map_minimal() {
    check_fixture_bf_only("data/domains/stitch/map_minimal.json", STITCH_LAMBDA_ARGS, true);
}

/// Context-threading test: duplicated subterms under two lambdas differ at one
/// leaf. With `--language lambda-calc` the matcher sees `lam`/`$0` as binding
/// nodes, so the abstraction sits inside the lambda and references `$0`.
#[test]
fn stitch_ctx_thread_1() {
    check_fixture_bf_only("data/domains/stitch/ctx_thread_1.json", STITCH_LAMBDA_ARGS, true);
}

/// Like `ctx_thread_1` but without the outer `A` wrapper — the two programs
/// start with `(lam (lam …))` directly. Exercises hole matching when the
/// pattern root sits at (or near) the program root.
#[test]
fn stitch_ctx_thread_2() {
    check_fixture_bf_only("data/domains/stitch/ctx_thread_2.json", STITCH_LAMBDA_ARGS, true);
}

/// Variant whose duplicated subterms reference both `$0` and `$1`. Exercises
/// holes whose matches contain multiple distinct pattern-internal free vars.
#[test]
fn stitch_ctx_thread_twice() {
    check_fixture_bf_only("data/domains/stitch/ctx_thread_twice.json", STITCH_LAMBDA_ARGS, true);
}

/// Higher-order pattern: three programs each repeat `(app f $0)` twice for
/// some `f` ∈ {`inc`, `dec`, `(app plus $0)`}. The metavar captures the
/// applied head (closed for `inc`/`dec`, open for `(app plus $0)`), so this
/// exercises mixed closed/open captures across matches.
#[test]
fn stitch_hof() {
    check_fixture_bf_only("data/domains/stitch/hof.json", STITCH_LAMBDA_ARGS, true);
}

/// Two recursive `map`-like programs differing in the operation applied to
/// each element. Exercises pattern enumeration through a deeply nested
/// fixed-point combinator (`Y`) and curried applications.
#[test]
fn stitch_map() {
    check_fixture_bf_only("data/domains/stitch/map.json", STITCH_LAMBDA_ARGS, true);
}

/// Variant of `map` using flat n-ary `(+ ...)` / `(- ...)` instead of curried
/// binary `+`. Exercises auto-currying at parse time.
#[test]
fn stitch_map2() {
    check_fixture_bf_only("data/domains/stitch/map2.json", STITCH_LAMBDA_ARGS, true);
}

/// Two short programs sharing only `(a (lam (cons (car $0) ...)))` skeletons —
/// stitch reports "Cost Improvement: 1.00x better" (no inventions found). We
/// pin this so a future change that *does* find an abstraction here is caught
/// (and can be reviewed against stitch).
#[test]
fn stitch_no_invention_cons_car() {
    check_fixture_bf_only("data/domains/stitch/no_invention_cons_car.json", STITCH_LAMBDA_ARGS, true);
}

/// Exercises list-headed application `((is_nil $0) nil (cons (+ $0)))` —
/// `is_nil` is applied to `$0` and the result is itself applied to two more
/// args. egg's default `RecExpr` parser rejects head-as-list, but our custom
/// `LambdaCalc::parse_program` curries naturally over arbitrary heads.
#[test]
fn stitch_safe_ctx_thread_bug() {
    check_fixture_bf_only("data/domains/stitch/safe_ctx_thread_bug.json", STITCH_LAMBDA_ARGS, true);
}

/// End-to-end check that the intersection-based fv analysis lets all three
/// programs rewrite under the natural arity-1 abstraction, even when the
/// `(* 0 ?x) => 0` rule pollutes one match's capture eclass.
///
/// Corpus: three programs of shape `(big (chain (of (g X (lam X)))))` with
/// `X` ∈ {`xx`, `yy`, `(* 0 $0)`}. Under the rule every program's body
/// collapses to the same shape, and `(big (chain (of (g ?#0 (lam ?#0)))))`
/// matches all three with ?#0 = `xx`, `yy`, `0`. For the third program ?#0
/// captures the eclass `{0, (* 0 $0)}`; intersection fv reports `{}` so
/// `subst_is_sound` accepts it and WeightedSize extraction picks `0`.
#[test]
fn fv_overapprox_annihilator() {
    check_fixture_bf_only(
        "data/domains/fv-overapprox/annihilator.json",
        &["-r", "data/domains/fv-overapprox/annihilator.rewrites", "--language", "lambda-calc", "--sym-var-cost", "100", "--max-arity", "1"],
        true,
    );
}

/// Self-application: each program has two copies of `(f $0)` for some `f`,
/// and the abstraction is `(?#0 ?#0)` applied to those copies. Exercises
/// metavar reuse where the captured subterm has open fv (referring to the
/// program's own lam) — sound under stitch convention because the fv is
/// outer-context relative to the depth-0 pattern.
#[test]
fn stitch_simple_hof() {
    check_fixture_bf_only("data/domains/stitch/simple_hof.json", STITCH_LAMBDA_ARGS, true);
}

/// Two programs sharing `(+ 2 3 4 (lam (+ $1 6 7)))`; the inner `$1` would
/// escape its `lam` if extracted into an abstraction body, so the lambda-calc
/// fv check must reject any candidate that includes the inner `lam`.
#[test]
fn stitch_free_no_args() {
    check_fixture_bf_only("data/domains/stitch/free-no-args.json", STITCH_LAMBDA_ARGS, true);
}

#[test]
fn stitch_free_no_args_huge_lam_stack() {
    check_fixture_bf_only("data/domains/stitch/free-no-args-huge-lam-stack.json", STITCH_LAMBDA_ARGS, true);
}

// === HO capture tests (formerly tests/higher_order_test.rs) ===
//
// These pin best-first behavior on corpora designed to exercise HO arity > 0
// captures (η-wrap in the body, shift-and-λ-wrap at each call site). Unlike
// the stitch-compat suite they don't carry `--sym-var-cost 100`, since the
// fixtures were blessed against egg-stitch's default cost model.

const LAMBDA: &[&str] = &["--language", "lambda-calc"];

/// Five programs sharing `(lam (foo (bar _)))` where the trailing slot is a
/// distinct closed-head application of `$0`. Captures use HO arity 1 to lift
/// the open `(@ X $0)` subterms under the surrounding lam.
#[test]
fn ho_shared_lam_uniform_bottom() {
    check_fixture_bf_only("data/domains/higher-order/uniform-bottom.json", LAMBDA, true);
}

/// Programs whose bottom shapes vary in *how* they use `$0` (head, middle,
/// trailing, bare). The HO pattern `(lam (foo (bar ?#0)))` covers all
/// variants by η-wrapping each capture.
#[test]
fn ho_shared_lam_varying_bottom() {
    check_fixture_bf_only("data/domains/higher-order/varying-bottom.json", LAMBDA, true);
}

/// Minimal: each program is just `(lam (h $0))` for varying head leaf. The
/// only shared structure is `(lam _)`, so any compression must put a `lam`
/// inside the abstraction body — pure HO at `var_depth > 0`.
#[test]
fn ho_minimal_lam_varying_head() {
    check_fixture_bf_only("data/domains/higher-order/minimal-head.json", LAMBDA, true);
}

/// Same varying-bottom inner shapes as `varying-bottom.json`, but wrapped in
/// a chunky outer `(+ a b c d e f (lam …))` so there's a lot of shared
/// non-lam structure surrounding the variation. Tests whether outer context
/// shifts the optimum from inside-lam to a deeper abstraction that includes
/// the outer skeleton.
#[test]
fn ho_shared_lam_with_outer_context() {
    check_fixture_bf_only("data/domains/higher-order/outer-context.json", LAMBDA, true);
}

/// Regression: `inline_useless_nonfrozen` must skip cross-depth-reused
/// metavars. When `?#k` is shared between a shallow occurrence at `d_a` and a
/// deep one at `d_b > d_a` via `subset_matches_reuse`, `shift_equal` can
/// accept the reuse purely on a context-fv check — but a single literal at
/// both sites would mean different binders. Pre-fix the search reached
/// `(lam (map (lam (?#0 ?#1)) ?#1))` (cross-depth `?#1`), then the dominant
/// inline collapsed `?#1` to literal `$0`, producing
/// `(lam (map (lam (?#0 fn_55 $0)) $0))`. The match set still claimed to
/// match `(lam (map (lam (index fn_55 $1)) $0))`, whose deep `$1` references
/// the *outer* binder — so the rewrite came out as `(fn_57 index)` and
/// β-reduced to a program with `$0` in place of the original `$1`. The fixture
/// pins the post-fix output (the inline is skipped, search settles on an
/// arity-1 mod-pattern at cost 457). This is the smallest list-domain corpus
/// that exhibits the bug; smaller hand-crafted inputs lose the corpus pressure
/// that pushes best-first onto the cross-depth-reuse path.
#[test]
fn cross_depth_useless_inline() {
    check_fixture_bf_only("data/domains/ho-bugs/cross_depth_useless_inline.json", LAMBDA, true);
}

/// Parallel to [`cross_depth_useless_inline`] with `--allow-useless-vars`, which
/// forces `opt_useless_inline` off. The unsound cross-depth inline the original
/// test guards against can therefore never fire, so best-first settles on the
/// same sound arity-1 mod-pattern `(lam (map #0 $0))` — no useless var, no
/// cross-depth collapse. Confirms disabling the inline short-circuit doesn't
/// reintroduce the bug (the fix isn't load-bearing only when the opt is on).
#[test]
fn cross_depth_useless_inline_allow_useless_vars() {
    let bf = run_backend("best-first", "data/domains/ho-bugs/cross_depth_useless_inline.json", &["--language", "lambda-calc", "--allow-useless-vars"]);
    assert_eq!(abstraction_bodies(&bf), vec!["(lam (map #0 $0))".to_string()], "inline off should still yield the sound arity-1 pattern");
}

/// Regression (minimised from the `logo` domain): a cross-depth reuse that
/// over-shifts a concrete DB-var leaf on inline. The three programs share the
/// skeleton `(lam (logo_forLoop ?N (lam (lam (logo_FWRT ?A ?B $0))) $0))`,
/// where the inner-loop accumulator `$0` (deep, binder depth 3) and the
/// `logo_forLoop` init `$0` (shallow, binder depth 1) hash-cons to the *same*
/// `$0` e-class. `shift_equal`'s `a == b` shortcut accepts merging them
/// cross-depth (their fv `{0}` sits *below* the gap `[1, 3)`), even though the
/// two `$0`s reference different binders. Best-first then dominance-reuses the
/// two slots and inlines the merged (useless) var: the deep occurrence is
/// shifted to `$2`, yielding the unsound
/// `(lam (logo_forLoop ?#0 (lam (lam (logo_FWRT ?#1 (logo_DIVA logo_UA 4) $2))) $0))`.
/// β-reduced, the abstraction puts `$2` where every original program has `$0`,
/// so `check_equiv` rejects it. The fixture pins the *sound* output (deep `$0`);
/// until the reuse is fixed this test fails on the `$2` mismatch.
#[test]
fn cross_depth_forloop_db_var_inline() {
    check_fixture_bf_only("data/domains/ho-bugs/cross_depth_forloop_db_var_inline.json", LAMBDA, true);
}

/// Conditional-branch unification driven by a DSR equivalence. Half the
/// programs place a big subterm `(a b c d e)` at one slot and half place
/// `(g h i j k)`. The `(if true x y) == x` / `(if false x y) == y` rewrites let
/// a *bare* branch be recognized as an arm of the conditional, so the optimal
/// abstraction is `(f (if ?#0 (a b c d e) (g h i j k)) ?#1)`: it captures both
/// big branches once in the body and each call site passes only a one-token
/// boolean condition instead of the whole 5-node subterm. A plain metavar hole
/// `(f ?#0 ?#1)` would have to pass that subterm at every site, so the
/// condition-parameterized `if` genuinely wins.
///
/// The `--max-forced-expansion` prune interacts with this optimum. Because
/// `(if true x y) => x` unions the `if` node into the bare branch's e-class, the
/// minimal extraction there is the branch itself — so the `if`-wrapped match is
/// non-minimal at every site, forcing a fixed 7 units of expansion (the extra
/// `if` node plus the unselected 5-node branch, less the 1-token condition). The
/// three tests below pin this: at the default (`none`, prune off) the optimum is
/// found (cost 39); a cap below the forcing (5) prunes it and a worse arity-1
/// abstraction wins (cost 44); a cap at or above it (10) clears the prune and
/// recovers the optimum (cost 39). (This is exactly the incompleteness of the
/// forced-expansion prune — kept cheap to spot.)
const IF_INPUT: &str = "data/domains/conditional/if_branch_unify.json";
const IF_RULES: &[&str] = &["--rules", "data/domains/conditional/if_branch.rewrites"];

/// Default (`--max-forced-expansion none`, prune off): the equivalence-driven
/// `if`-unification optimum is found (cost 39). Owns the plain fixture.
#[test]
fn conditional_branch_unify() {
    check_fixture_bf_only(IF_INPUT, IF_RULES, true);
}

/// `--max-forced-expansion 5`: the `if`-wrapped match forces 7 units of expansion
/// at every site, over the cap, so the prune drops the optimum and a worse arity-1
/// abstraction wins (cost 44). Pins the `.cap5` fixture.
#[test]
fn conditional_branch_unify_cap5_drops_if_abstraction() {
    let args = &["--rules", "data/domains/conditional/if_branch.rewrites", "--max-forced-expansion", "5"];
    let mut bf = run_backend_steps("best-first", IF_INPUT, "50000", args);
    strip_library_field(&mut bf, "best_history");
    bless_or_check("data/expected_outputs/conditional/if_branch_unify.cap5.out.json", &bf, "if_branch_unify (cap 5)");
}

/// `--max-forced-expansion 10`: raising the cap past the forced expansion (7)
/// recovers the equivalence-driven `if`-unification optimum (cost 39). Pins the
/// `.cap10` fixture.
#[test]
fn conditional_branch_unify_cap10_recovers_if_abstraction() {
    let args = &["--rules", "data/domains/conditional/if_branch.rewrites", "--max-forced-expansion", "10"];
    let mut bf = run_backend_steps("best-first", IF_INPUT, "50000", args);
    strip_library_field(&mut bf, "best_history");
    bless_or_check("data/expected_outputs/conditional/if_branch_unify.cap10.out.json", &bf, "if_branch_unify (cap 10)");
}

/// Crossed-spin wrap collapse: the optimum
/// `(g (da³(P ?#0 ?#1)) (db³(Q ?#0 ?#2)))` reuses `?#0` across two independent
/// wrappers that collapse on disjoint match families (`P` at the r1 roots, `Q`
/// at the r2 roots). Pins that the search recovers this cross-wrapper reuse
/// optimum (cost 54) rather than settling for a per-wrapper abstraction.
#[test]
fn crossed_wrap_collapse() {
    let args = &["--language", "op-children", "--rules", "data/test/crossed_wrap_collapse.rewrites", "--max-arity", "3"];
    check_fixture_bf_only("data/test/crossed_wrap_collapse.json", args, true);
}

/// Regression: a metavar reused across binder depths keeps a genuine
/// higher-order capture, and its η-app body args must be depth-shifted per
/// occurrence. `build_with_ho` / `display_pattern_as_lambda` applied the raw
/// `variable_indices[k]` at every occurrence, dropping the shift.
///
/// Unlike `cross_depth_forloop_db_var_inline` (which inlines the metavar to a
/// literal DB leaf), this corpus keeps the capture higher-order
/// (`variable_indices = [[0]]`, `var_depth = 1`) on a metavar merged across
/// depths 1 and 2. The five programs share the heavy skeleton
/// `(+ a b c d e f (lam (foo (bar _) (gg (lam (bar _))))))` — big enough above
/// the binding `lam` that best-first roots the abstraction *above* it, so the
/// captured `$0` is pattern-internal — while the inner slot varies in *where*
/// it uses the bound variable (`(h $0)`, `($0 q)`, `(k $0 r)`, `(m s $0 t)`,
/// bare `$0`), so no uniform literal body fits and the capture stays HO. The
/// shallow slot uses `$0`, the deep slot (one `lam` deeper) the shift-variant
/// `$1`.
///
/// The fixture pins the sound output (deep η-arg `$1`, lambda `($2 $1)`).
/// Pre-fix the export emits `($2 $0)` at the deep occurrence, so `(fn_0 …)`
/// β-reduces to `(bar $0)` where the original has `(bar $1)`. `--check-slow`
/// misses it (size and root-fv are preserved); `scripts/check_all_outputs.py`'s
/// β-equivalence sweep over the blessed fixture catches it. Until the
/// per-occurrence shift is restored this test fails on the `$0`/`$1` mismatch.
#[test]
fn cross_depth_ho_capture() {
    check_fixture_bf_only("data/domains/ho-bugs/cross_depth_ho_capture.json", LAMBDA, true);
}

/// Regression (minimised from `cogsci/dials` DSR + `--max-forced-expansion 3`):
/// a 3-way metavar reuse whose three occurrences are an *early single* and a
/// *later sibling pair* at a deeper level. The optimal abstraction reuses `?#1`
/// three times:
///
/// ```text
/// (C ?#0 (T (T (T l (M 1 0 -0.5 0)) (M ?#1 (/ pi 4) 0 0))
///           (M 1 0 (* ?#1 (* 0.5 (cos (/ pi 4)))) (* ?#1 (* 0.5 (sin (/ pi 4)))))))
/// ```
///
/// The DSRs (`reroll_2_x_r` / `r3roll_2_x`) re-root the corpus through `repeat`
/// wrappers, so best-first reaches the bare `(C ?#0 …)` form via a path that has
/// already merged some other vars. A reuse canonical-ordering that stales *every*
/// lower-indexed var on each merge then marks the middle (`cos`) occurrence
/// non-absorbable before it can be merged — stranding it, so the search settles
/// for a worse 2-way abstraction (the `M`-position merged with `sin`, `cos` left
/// alone). This pins that the 3-way stays reachable: the body must reuse one
/// metavar three times. Best-first is deterministic, so this is exact.
#[test]
fn dials_reroll_three_way_reuse() {
    let input = "data/test/dials_reroll_3way.json";
    let args = &["--rules", "data/test/dials_reroll_3way.rewrites", "--max-forced-expansion", "3", "--max-arity", "2"];
    let v = run_backend("best-first", input, args);
    let bodies = abstraction_bodies(&v);
    assert_eq!(bodies.len(), 1, "expected one abstraction, got {bodies:?}");
    // The 3-way reuse means one metavar (`#1`) occurs three times in the body;
    // the regressed 2-way abstraction has a max of two.
    let body = &bodies[0];
    let max_reuse = (0..8).map(|k| body.matches(&format!("#{k}")).count()).max().unwrap_or(0);
    assert!(max_reuse >= 3, "expected a metavar reused 3× (the 3-way reuse), got max {max_reuse} in body: {body}");
}

/// Regression (minimised from `cogsci/nuts-bolts` DSR): a 3-way reuse that the
/// search can only reach via a *non-canonical* merge order, because dominance and
/// expand order force the hand. The optimal abstraction reuses `?#0` three times:
///
/// ```text
/// (T (repeat (T l (M 1 0 -0.5 (/ 0.5 (tan (/ pi ?#0))))) ?#0 (M 1 (/ (* 2 pi) ?#0) 0 0)) (M ?#1 0 0 0))
/// ```
///
/// Two of the three occurrences — the `repeat` count and the `(/ (* 2 pi) ?#0)`
/// angle — are always equal, so reuse *dominance* force-merges them as a single
/// successor the moment they exist, which is *before* the third occurrence
/// (`(tan (/ pi ?#0))`, created by a deeper, later expand) is even present. So the
/// 3-way can only be completed by either (a) prepending the late `tan` occurrence
/// to the already-merged count/angle block, or (b) — with dominance off — a
/// "late" merge of two post-expand-stale slots. This distinguishes it from
/// `dials_reroll_three_way_reuse`, whose 3-way *is* reachable in canonical order:
/// a reuse canonicalization can pass dials yet still strand this one.
///
/// Pins that the 3-way stays reachable (one metavar reused three times) despite
/// the forced non-canonical order. The lone `scale_1_T` self-loop rule supplies
/// the `(T … (M 1.0 0 0 0))` wrapper nesting that sets up the expand order.
#[test]
fn nuts_bolts_dominance_three_way_reuse() {
    let input = "data/test/nuts_bolts_3way_reuse.json";
    let args = &["--rules", "data/test/nuts_bolts_3way_reuse.rewrites", "--max-arity", "2"];
    let v = run_backend("best-first", input, args);
    let bodies = abstraction_bodies(&v);
    assert_eq!(bodies.len(), 1, "expected one abstraction, got {bodies:?}");
    let max_reuse = max_var_reuse(&bodies[0]);
    assert!(max_reuse >= 3, "expected a metavar reused 3× (the 3-way reuse), got max {max_reuse} in body: {}", bodies[0]);
}

/// Parallel to [`nuts_bolts_dominance_three_way_reuse`] with `--allow-useless-vars`,
/// which forces `opt_dominance_reuse` off. The 3-way-reused `?#0` is *not*
/// useless (it binds a different value at each call site), so the optimum is
/// unchanged — but reaching it now goes through the "late" post-expand merge
/// path (option (b) in the original's doc) instead of the dominance
/// force-merge. Pins that the 3-way stays reachable with dominance disabled.
#[test]
fn nuts_bolts_dominance_three_way_reuse_allow_useless_vars() {
    let input = "data/test/nuts_bolts_3way_reuse.json";
    let args = &["--rules", "data/test/nuts_bolts_3way_reuse.rewrites", "--max-arity", "2", "--allow-useless-vars"];
    let v = run_backend("best-first", input, args);
    let bodies = abstraction_bodies(&v);
    assert_eq!(bodies.len(), 1, "expected one abstraction, got {bodies:?}");
    let max_reuse = max_var_reuse(&bodies[0]);
    assert!(max_reuse >= 3, "3-way reuse should survive dominance-off, got max {max_reuse} in body: {}", bodies[0]);
}

/// Hand-built `data/test/` corpora whose rule sets each introduce an identity
/// self-loop — a freely re-nestable wrapper (`(+ ?x 0) == ?x`,
/// `(if ?c ?x ?x) == ?x`, `(repeat ?x 1 ?m) == ?x`, `(f (g ?x)) == ?x`) —
/// alongside genuine equivalences (`4 == (+ 2 2)`). Best-first only, capped at
/// `--max-arity 2`: the self-loops make the space unbounded, so the
/// `--num-steps` cap in `run_backend` is what bounds the towers (`converge_tower`
/// / `nested_loop_tower` still hit the cap, leaving a non-zero `heap_sizes_at_end`
/// — the regression these pin until dominated-wrapper stripping drains them).
fn check_self_loop(input: &str, rules: &str) {
    check_fixture_bf_only(&format!("data/test/{input}.json"), &["--rules", &format!("data/test/{rules}.rewrites"), "--max-arity", "2"], true);
}

/// `4 == (+ 2 2)` unifies the two corpus shapes; `(+ ?x 0)` is the self-loop.
#[test]
fn self_loop_arith_unify() {
    check_self_loop("arith_unify", "arith");
}

/// Direct-child self-loop `(repeat ?x 1 ?m) => ?x` over a compressive corpus.
#[test]
fn self_loop_converge_tower() {
    check_self_loop("converge_tower", "converge_tower");
}

/// Grandchild self-loop `(f (g ?x)) => ?x` over a compressive corpus.
#[test]
fn self_loop_nested_loop_tower() {
    check_self_loop("nested_loop_tower", "nested_loop_tower");
}

/// `if_nest`/`if_same` self-loop over an `(if p _ _)` corpus.
#[test]
fn self_loop_if_unify() {
    check_self_loop("if_unify", "if");
}

// === op-children-db (flat n-ary nodes with De Bruijn `$n` leaves) ===
//
// `--language op-children-db` keeps OpChildren's flat n-ary shape but parses
// `$n` as a real free De Bruijn variable (the leaf-Op becomes `OpDB<Op>`).
// There are no binders, so every `$n` is free and `invalid_literal_expansion`
// bans it from any abstraction body — it can only ever be captured as an
// argument. The corpus below shares the skeleton `(f (g (h _)) $0)` across four
// programs, so the body-ban is the only thing that distinguishes the two
// languages' output.

const OP_CHILDREN_DB_INPUT: &str = "data/test/op_children_db_free_var.json";

/// op-children-db: the free `$0` is illegal in an abstraction body, so it is
/// lifted to a second parameter — the arity-2 `(f (g (h ?#0)) ?#1)`, with no
/// `$` anywhere in the body. The inline assertion is the feature invariant; the
/// fixture pins the exact (deterministic best-first) result.
#[test]
fn op_children_db_bans_free_var_from_body() {
    let mut bf = run_backend("best-first", OP_CHILDREN_DB_INPUT, &["--language", "op-children-db"]);
    strip_library_field(&mut bf, "best_history");
    for body in abstraction_bodies(&bf) {
        assert!(!body.contains('$'), "op-children-db must keep free DB vars out of the abstraction body, got `{body}`");
    }
    bless_or_check(&expected_path(OP_CHILDREN_DB_INPUT), &bf, OP_CHILDREN_DB_INPUT);
}

/// Contrast: the same corpus under plain `op-children` parses `$0` as an
/// ordinary symbol leaf (leaf-Op `Op`, no DB awareness), so it stays baked into
/// the arity-1 body `(f (g (h ?#0)) $0)`. Pins that the body-ban is the
/// leaf-op's doing, not the corpus's. Blessed to a separate `.plain` fixture to
/// avoid the shared-input path collision with the test above.
#[test]
fn op_children_plain_keeps_free_var_in_body() {
    let mut bf = run_backend("best-first", OP_CHILDREN_DB_INPUT, &[]);
    strip_library_field(&mut bf, "best_history");
    let bodies = abstraction_bodies(&bf);
    assert!(bodies.iter().any(|b| b.contains("$0")), "plain op-children should keep `$0` baked into the body, got {bodies:?}");
    bless_or_check("data/expected_outputs/test/op_children_db_free_var.plain.out.json", &bf, "op_children_db_free_var (plain)");
}

// === molecule domain (op-children) ===
//
// Molecules are rooted trees over `(<bond><degree> E n1 n2 ...)` atom nodes
// (see `data/domains/molecules/molecules.rewrites` for the encoding). They run
// under the default `op-children` language — no `--language` flag. These pins
// guard the two molecule-specific behaviours: the re-rooting + commutativity
// DSRs that recover cross-molecule alignment, and the scramble benchmark
// (`data/domains/molecules/scramble/`) where those DSRs stand in for a canonical
// SMILES encoding. All are best-first only (op-children SMC isn't pinned here);
// the full 5000-molecule `molecules.json` corpus is left out as too slow for a
// unit test (its β-equivalence is covered by `scripts/check_all_outputs.py`).

/// Two byte-identical ethanol trees, same rooting. With no rules the only
/// shared structure is the whole molecule, so the search abstracts it wholesale
/// — the trivial baseline before any symmetry DSR is in play.
#[test]
fn molecules_ethanol_same_rooting() {
    check_fixture_bf_only("data/test/ethanol_same_rooting.json", &[], true);
}

/// The *same* ethanol molecule written from two different roots (carbon-rooted
/// vs oxygen-rooted). Without rules the two trees share nothing structurally;
/// the re-rooting DSRs in `molecules.rewrites` make them a single e-class, so
/// the search recovers one whole-molecule abstraction covering both. This pins
/// that re-rooting alignment fires — the core molecule-domain feature.
#[test]
fn molecules_ethanol_rerooted() {
    check_fixture_bf_only("data/test/ethanol_two_rootings.json", &["-r", "data/domains/molecules/molecules.rewrites"], true);
}

/// The general molecule symmetry DSRs (re-rooting, proper rotations, and
/// witness-guarded transpositions) — the same file the main corpus uses.
const SYMMETRIES: &[&str] = &["-r", "data/domains/molecules/molecules.rewrites"];

/// Canonical-rooted scramble corpora, no rules — the reference encoding the
/// benchmark compares against. Best-first converges (heap drains) so the default
/// budget suffices and the result is exact. Each pins the single best
/// abstraction the canonical alignment admits.
#[test]
fn molecules_scramble_hexyl_canon() {
    check_fixture_bf_only("data/domains/molecules/scramble/hexyl.canon.json", &[], true);
}

#[test]
fn molecules_scramble_ester_canon() {
    check_fixture_bf_only("data/domains/molecules/scramble/ester.canon.json", &[], true);
}

#[test]
fn molecules_scramble_glycol_canon() {
    check_fixture_bf_only("data/domains/molecules/scramble/glycol.canon.json", &[], true);
}

// The scrambled corpora (random root + child order) only realign once the
// symmetry DSRs saturate, which makes the e-graph large enough that best-first
// never drains its heap — so, like the self-loop fixtures above, these are
// pinned at a fixed `--num-steps` budget (the optimum is reached early; the
// budget just bounds the otherwise-unbounded tail). `check_all_outputs.py`
// re-checks each rewritten corpus for equivalence under the same DSRs.

/// Scrambled hexyl recovered by the DSRs: the re-rooting rules realign the
/// scrambled trees onto the shared 6-carbon backbone, so the abstraction is the
/// re-rooted alkyl chain `(m1 H (s4 C ...))` — the DSR-as-canonicaliser payoff.
#[test]
fn molecules_scramble_hexyl_dsr() {
    check_fixture_bf_only_steps("data/domains/molecules/scramble/hexyl.scram.json", "2000", SYMMETRIES, true);
}

/// Scrambled ester. Its shared motif (`CC(=O)OC`) is small, so under
/// single-abstraction search the plain `(s1 H)` leaf still wins on raw count —
/// pinned as the current behaviour (the larger ester-group abstraction only
/// surfaces in the multi-abstraction benchmark run).
#[test]
fn molecules_scramble_ester_dsr() {
    check_fixture_bf_only_steps("data/domains/molecules/scramble/ester.scram.json", "2000", SYMMETRIES, true);
}

/// Scrambled glycol recovered by the DSRs onto its re-rooted polyether backbone
/// `(s2 O (s4 C ...))`.
#[test]
fn molecules_scramble_glycol_dsr() {
    check_fixture_bf_only_steps("data/domains/molecules/scramble/glycol.scram.json", "2000", SYMMETRIES, true);
}

// === loop rolling: live DSRs roll a loop --only-use-dsrs-at-start can't ===
//
// A minimal domain showing the qualitative win of applying rewrite rules live
// (during abstraction search) over applying them once up-front
// (`--only-use-dsrs-at-start`, which normalizes the egraph, extracts a single
// min-term per program, and searches a fresh rule-free egraph built from it).
// `(repeat body count step)` is a counted loop; the only rule,
// `(repeat ?x 1 ?n) => ?x` in `data/test/loop_rolling.rewrites`, says a count-1
// loop is just its body. The corpus shares one body across loops of *different*
// counts, so a single count-parameterized abstraction `(repeat (f a b c) ?#0
// step)` rolls them all — but only while the count-1 loop still looks like a
// loop, which min-term extraction destroys.

/// The shared count-1-flatten rule, applied live vs only at start.
const LOOP_RULES: &[&str] = &["--rules", "data/test/loop_rolling.rewrites"];
const LOOP_RULES_AT_START: &[&str] = &["--rules", "data/test/loop_rolling.rewrites", "--only-use-dsrs-at-start"];

/// Corpus mixes a count-1 loop with counts 2-4, so live and at-start *diverge*.
/// Live keeps the count-1 program's `(repeat (f a b c) 1 step)` form in the
/// e-class and rolls every program into the count-parameterized loop abstraction
/// `(repeat (f a b c) ?#0 step)` (cost 16, the `.live` fixture). But at-start's
/// min-term extraction fires `rep_1` on the count-1 program, flattening it to
/// the bare body `(f a b c)` *before* search — the loop skeleton is gone — so
/// the only thing left to share across all four programs is the body itself, and
/// at-start abstracts `(f a b c)` (cost 18, the `.at_start` fixture). The loop is
/// "otherwise not rollable": only live abstractions can roll it.
#[test]
fn loop_count1_flattens_so_only_live_rolls() {
    let input = "data/test/loop_rolling_with_count1.json";
    bless_bf_run(input, LOOP_RULES, "data/expected_outputs/test/loop_rolling_with_count1.live.out.json", "loop_rolling_with_count1 (live)");
    bless_bf_run(input, LOOP_RULES_AT_START, "data/expected_outputs/test/loop_rolling_with_count1.at_start.out.json", "loop_rolling_with_count1 (at-start)");
}

/// Control: drop the count-1 loop (counts 2-5), so `rep_1` never fires during
/// min-term extraction and the loop skeleton survives for both modes. Now both
/// roll the same count-parameterized abstraction `(repeat (f a b c) ?#0 step)`,
/// checked against a *single* shared fixture. This pins that the divergence
/// above is specifically the count-1 min-term flattening, not a general
/// inability of at-start to roll loops.
#[test]
fn loop_no_count1_both_roll() {
    let input = "data/test/loop_rolling_no_count1.json";
    let fixture = "data/expected_outputs/test/loop_rolling_no_count1.out.json";
    bless_bf_run(input, LOOP_RULES, fixture, "loop_rolling_no_count1 (live)");
    bless_bf_run(input, LOOP_RULES_AT_START, fixture, "loop_rolling_no_count1 (at-start)");
}
