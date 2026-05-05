//! Bless/check tests for HO capture.
//!
//! Mirrors `stitch_compat_test`'s harness: invokes the cargo-built binary on a
//! frozen JSON corpus, captures `--output` JSON, strips noise fields, and
//! compares against a fixture under `data/expected_outputs/higher-order/`.
//!
//! HO capture is always on — every metavar with `ho_arity[k] > 0` (i.e.
//! captures whose fv reaches into pattern-internal binders) is η-wrapped in
//! the abstraction body and shifted-and-λ-wrapped at each call site.
//!
//! To regenerate the fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --test higher_order_test -- --test-threads=1
//! ```

use serde_json::Value;
use std::{fs, path::Path, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

fn expected_path(input: &str) -> String {
    let relative = input.strip_prefix("data/domains/").expect("expected input under data/domains/");
    let stem = relative.strip_suffix(".json").unwrap_or(relative);
    format!("data/expected_outputs/{stem}.out.json")
}

fn temp_output_path(input: &str) -> std::path::PathBuf {
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    std::env::temp_dir().join(format!("egg-stitch-ho-{}-{}.json", std::process::id(), stem))
}

/// Runs best-first on `input` with the given extra args, returns the
/// `--output` JSON with non-deterministic fields stripped.
fn run_bf(input: &str, extra_args: &[&str]) -> Value {
    let out = temp_output_path(input);
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", input, "--check-slow", "--num-abstractions", "1", "--num-steps", "10000", "--output", out_str]);
    cmd.args(extra_args);
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed for {input}");

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

fn bless_or_check(path: &str, value: &Value, ctx: &str) {
    if std::env::var("BLESS").is_ok() {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir {}: {e}", parent.display()));
        }
        let mut text = serde_json::to_string_pretty(value).expect("serialize expected");
        text.push('\n');
        fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        assert_eq!(value, &expected, "fixture mismatch for {ctx} (run with BLESS=1 to update)");
    }
}

fn check_fixture(input: &str, base_args: &[&str]) {
    let v = run_bf(input, base_args);
    bless_or_check(&expected_path(input), &v, input);
}

const LAMBDA: &[&str] = &["--language", "lambda-calc"];

/// Five programs sharing `(lam (foo (bar _)))` where the trailing slot is a
/// distinct closed-head application of `$0`. Captures use HO arity 1 to lift
/// the open `(@ X $0)` subterms under the surrounding lam.
#[test]
fn shared_lam_uniform_bottom() {
    check_fixture("data/domains/higher-order/uniform-bottom.json", LAMBDA);
}

/// Programs whose bottom shapes vary in *how* they use `$0` (head, middle,
/// trailing, bare). The HO pattern `(lam (foo (bar ?#0)))` covers all
/// variants by η-wrapping each capture.
#[test]
fn shared_lam_varying_bottom() {
    check_fixture("data/domains/higher-order/varying-bottom.json", LAMBDA);
}

/// Minimal: each program is just `(lam (h $0))` for varying head leaf. The
/// only shared structure is `(lam _)`, so any compression must put a `lam`
/// inside the abstraction body — pure HO at `var_depth > 0`.
#[test]
fn minimal_lam_varying_head() {
    check_fixture("data/domains/higher-order/minimal-head.json", LAMBDA);
}

/// Same varying-bottom inner shapes as `varying-bottom.json`, but wrapped in
/// a chunky outer `(+ a b c d e f (lam …))` so there's a lot of shared
/// non-lam structure surrounding the variation. Tests whether outer context
/// shifts the optimum from inside-lam to a deeper abstraction that includes
/// the outer skeleton.
#[test]
fn shared_lam_with_outer_context() {
    check_fixture("data/domains/higher-order/outer-context.json", LAMBDA);
}
