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
        cmd.args(["--num-steps", "10000"]);
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

/// Strips the `pattern` field from every entry in `library` (in place). Used
/// when SMC's chosen e-class representative is non-deterministic (e.g. once
/// commutativity rewrites unify multiple equivalent pattern strings).
fn strip_library_patterns(v: &mut Value) {
    let Some(library) = v.get_mut("library").and_then(|l| l.as_array_mut()) else { return };
    for entry in library {
        if let Some(obj) = entry.as_object_mut() {
            obj.remove("pattern");
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
    if !check_pattern {
        strip_library_patterns(&mut bf);
        strip_library_patterns(&mut smc);
    }
    // Collapse to a single entry when both backends agree; otherwise record
    // both side-by-side so the divergence is visible in the fixture.
    let combined = if bf == smc { bf } else { json!({"best-first": bf, "smc": smc}) };

    let path = expected_path(input);
    if std::env::var("BLESS").is_ok() {
        let mut text = serde_json::to_string_pretty(&combined).expect("serialize expected");
        text.push('\n');
        fs::write(&path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        assert_eq!(combined, expected, "fixture mismatch for {input} (run with BLESS=1 to update)");
    }
}

#[test]
fn identical() {
    check_fixture("data/domains/stitch/identical.json", &[], true);
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

/// Returns the rewritten corpus from the last library entry, falling back to
/// the supplied original program list when no abstraction was found.
fn rewritten_corpus(run: &Value, original: &[String]) -> Vec<String> {
    if let Some(last) = run.get("library").and_then(|l| l.as_array()).and_then(|l| l.last())
        && let Some(arr) = last.get("rewritten_programs").and_then(|p| p.as_array())
    {
        return arr.iter().filter_map(|s| s.as_str().map(String::from)).collect();
    }
    original.to_vec()
}

#[test]
fn arith_rewrites() {
    let input = "data/domains/basic-apps/multi-arg-assoc.json";
    let extra_args = &["-r", "data/domains/basic-apps/app-arith.rewrites", "--language", "lambda-calc", "--max-arity", "0"];
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
