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

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

fn expected_path(input: &str) -> String {
    // Mirror the `data/domains/<...>/foo.json` layout under `data/expected_outputs/`.
    let relative = input.strip_prefix("data/domains/").expect("expected input under data/domains/");
    let stem = relative.strip_suffix(".json").unwrap_or(relative);
    format!("data/expected_outputs/{stem}.out.json")
}

/// Path for the temporary `--output` JSON of a single backend run. Includes
/// pid + input stem + search to stay unique across parallel tests.
fn temp_output_path(input: &str, search: &str) -> std::path::PathBuf {
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    std::env::temp_dir().join(format!("egg-stitch-compat-{}-{}-{}.json", std::process::id(), stem, search))
}

/// Invokes the cargo-built binary, writes its `--output` JSON to a temp file,
/// reads it back, and strips non-deterministic fields.
fn run_backend(search: &str, input: &str, extra_args: &[&str]) -> Value {
    let out = temp_output_path(input, search);
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", search, "--input", input, "--check-slow", "--num-abstractions", "1", "--output", out_str]);
    if search == "best-first" {
        cmd.args(["--num-steps", "50000"]);
    } else {
        cmd.args(["--num-particles", "1000", "--num-steps", "1000", "--temperature", "1000"]);
    }
    cmd.args(extra_args);
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "{search} run failed for {input}");

    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    let mut v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", out.display()));
    if let Some(obj) = v.as_object_mut() {
        for k in ["timestamp", "elapsed_secs", "input_file", "rules_file", "search"] {
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
    let combined = if bf == smc { bf } else { json!({"best-first": bf, "smc": smc}) };
    bless_or_check(&expected_path(input), &combined, input);
}

/// Like `check_fixture` but runs only the best-first backend. Use when SMC
/// converges unreliably for the given corpus (so its output isn't worth
/// pinning) but best-first's enumeration is still a meaningful regression
/// signal. The fixture format is the same single `RunResult` shape that
/// `check_fixture` writes when both backends already agree.
fn check_fixture_bf_only(input: &str, extra_args: &[&str], check_pattern: bool) {
    let mut bf = run_backend("best-first", input, extra_args);
    strip_library_field(&mut bf, "best_history");
    if !check_pattern {
        strip_library_patterns(&mut bf);
    }
    bless_or_check(&expected_path(input), &bf, input);
}

/// Shared blessing/checking step for the two `check_fixture*` helpers.
fn bless_or_check(path: &str, value: &Value, input: &str) {
    if std::env::var("BLESS").is_ok() {
        let mut text = serde_json::to_string_pretty(value).expect("serialize expected");
        text.push('\n');
        fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        assert_eq!(value, &expected, "fixture mismatch for {input} (run with BLESS=1 to update)");
    }
}

#[test]
fn identical() {
    check_fixture("data/domains/stitch/identical.json", &[], true);
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

/// Diverges from Stitch.jl: Stitch.jl finds the arity-0 body
/// `(a b c d e f g h (A B C) (A B C) (A B C) (A B C))`; egg-stitch's e-class
/// equality unifies the four `(A B C)` subterms and picks the arity-1
/// `(a b c d e f g h #0 #0 #0 #0)` instead.
#[test]
fn cex() {
    check_fixture("data/domains/stitch/cex.json", &[], true);
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

#[test]
fn arithmetic_aplusbplus1234() {
    check_fixture("data/domains/simple-arithmetic/aplusbplus1234.json", &["-r", ARITH_RULES], false);
}

#[test]
fn common_start() {
    check_fixture("data/domains/basic-apps/common-start.json", &["-r", ARITH_RULES, "--language", "lambda-calc"], true);
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
        if abstr != ["+", "+", "a", "b", "c", "d"] && abstr != ["+", "+", "+", "a", "b", "c", "d"] {
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
