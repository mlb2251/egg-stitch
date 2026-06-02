//! Bless/check (snapshot) tests for the small hand-built corpora under
//! `data/test/`. Each pairs an input corpus with a rewrite-rule set that
//! introduces an identity self-loop — a freely re-nestable wrapper, e.g.
//! `(+ ?x 0) == ?x`, `(if ?c ?x ?x) == ?x`, `(repeat ?x 1 ?m) == ?x` — alongside
//! genuine equivalences (`4 == (+ 2 2)`, `(if true ?x ?y) == ?x`). Best-first's
//! output is pinned as a regression fixture so behavior changes show up as a
//! fixture diff.
//!
//! Only best-first is pinned: it enumerates canonical patterns deterministically
//! (SMC is stochastic). `--num-steps` is capped because the self-loop rules make
//! the search space unbounded — the cap keeps each run fast and the snapshot
//! stable. The non-deterministic / input-dependent fields (`timestamp`,
//! `elapsed_secs`, `input_file`, `rules_file`, `search`) and per-step bookkeeping
//! on every library entry (`num_steps_run`, `num_expansions`, `best_iteration`,
//! `best_history`) are stripped before comparison.
//!
//! To regenerate all fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test data_test_bless_test -- --test-threads=1
//! ```

use serde_json::Value;
use std::{fs, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// Directory holding the `<input>.json` corpora and `<rules>.rewrites` rule files.
const DATA_DIR: &str = "data/test";

/// Heap-pop budget. The self-loop rules make enumeration unbounded, so a cap is
/// required; this is large enough to pin a stable best while keeping runs fast.
const NUM_STEPS: &str = "2000";

/// Fixture path for an `<input>__<rules>` case, mirroring the
/// `data/expected_outputs/<...>` layout used by the other snapshot suites.
fn expected_path(name: &str) -> String {
    format!("data/expected_outputs/test/{name}.out.json")
}

/// Runs best-first on `data/test/<input>.json` with `data/test/<rules>.rewrites`,
/// writes its `--output` JSON to a unique temp file, reads it back, and strips
/// the non-deterministic / bookkeeping fields so the result is a stable snapshot.
fn run_bfs(input: &str, rules: &str) -> Value {
    let input_path = format!("{DATA_DIR}/{input}.json");
    let rules_path = format!("{DATA_DIR}/{rules}.rewrites");
    let out = std::env::temp_dir().join(format!("egg-stitch-datatest-{}-{}-{}.json", std::process::id(), input, rules));
    let out_str = out.to_str().expect("utf-8 temp path");
    let status = Command::new(BIN)
        .args(["--search", "best-first", "--input", &input_path, "--rules", &rules_path, "--num-steps", NUM_STEPS, "--num-abstractions", "1", "--max-arity", "2", "--output", out_str])
        .status()
        .unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed for {input_path} + {rules_path}");

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
                for k in ["num_steps_run", "num_expansions", "best_iteration", "best_history"] {
                    obj.remove(k);
                }
            }
        }
    }
    v
}

/// Blesses (`BLESS=1`) or checks the run output against the frozen fixture.
fn bless_or_check(path: &str, value: &Value) {
    let value = common::sorted(value);
    if std::env::var("BLESS").is_ok() {
        if let Some(parent) = std::path::Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
        }
        let mut text = serde_json::to_string_pretty(&value).expect("serialize expected");
        text.push('\n');
        fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let mut expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        common::sort_keys(&mut expected);
        assert_eq!(value, expected, "fixture mismatch for {path} (run with BLESS=1 to update)");
    }
}

/// Runs `<input>` + `<rules>` and bless/checks the snapshot at `<input>__<rules>`.
fn check(input: &str, rules: &str) {
    let v = run_bfs(input, rules);
    bless_or_check(&expected_path(&format!("{input}__{rules}")), &v);
}

/// `4 == (+ 2 2)` unifies the two corpus shapes; `(+ ?x 0)` is the self-loop.
#[test]
fn arith_unify() {
    check("arith_unify", "arith");
}

/// Direct-child self-loop `(repeat ?x 1 ?m) => ?x` over a compressive corpus.
#[test]
fn converge_tower() {
    check("converge_tower", "converge_tower");
}

/// Grandchild self-loop `(f (g ?x)) => ?x` over a compressive corpus.
#[test]
fn nested_loop_tower() {
    check("nested_loop_tower", "nested_loop_tower");
}

/// `if` evaluation (`if true`/`if false`) plus the `if_nest`/`if_same` self-loop.
#[test]
fn if_branch_unify_full() {
    check("if_branch_unify", "if_branch");
}

/// Same corpus with only the `if true`/`if false` evaluation rules (no self-loop).
#[test]
fn if_branch_unify_eval_only() {
    check("if_branch_unify", "if_eval_only");
}

/// `if_nest`/`if_same` self-loop over an `(if p _ _)` corpus.
#[test]
fn if_unify() {
    check("if_unify", "if");
}
