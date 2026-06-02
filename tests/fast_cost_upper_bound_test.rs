//! Snapshot (bless/check) regression for the fast-cost upper-bound bug.
//!
//! The analytic ("fast") cost path used to score a captured argument by the
//! un-shifted `sizes.get(v)`. For cross-depth / higher-order captures
//! `wrap_subst_args` / `shift_free_egraph` re-index the arg's free De Bruijn
//! leaves onto a *different* e-class — one on which the abstraction's own
//! rewrite may no longer fire — so the fast cost could fall *below* the exact
//! ("slow") rewritten size, violating `LanguageFamily::check_fast_vs_slow`'s
//! `fast >= slow` contract.
//!
//! Run with `--check-slow`, best-first on these dreamcoder benchmarks
//! deterministically reaches such a candidate (e.g. the pattern `(lam (?#0 $0))`):
//! before the fix the run aborts mid-search; after it the run completes and its
//! output is stable, so we pin it. `--num-steps` is large so the search runs to
//! convergence and a future change to search order still exercises the path.
//! Pinned under both uniform and non-uniform (`--sym-var-cost 100`) weights,
//! since the undercount shows up under each.
//!
//! To regenerate the fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test fast_cost_upper_bound_test -- --test-threads=1
//! ```

use serde_json::Value;
use std::{fs, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// Runs best-first with `--check-slow` on `input` (plus any extra args), reads
/// the `--output` JSON back, and strips the non-deterministic / bookkeeping
/// fields so the result is a stable snapshot. The `--check-slow` flag makes the
/// binary abort if any scored candidate's fast cost dips below the exact
/// rewritten size, so a regression of the underlying bug fails the run before
/// the snapshot is even compared.
fn run(input: &str, extra: &[&str], tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-fast-ub-{}-{}.json", std::process::id(), tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut args = vec!["--search", "best-first", "--input", input, "--language", "lambda-calc", "--num-abstractions", "1", "--num-steps", "100000", "--check-slow", "--seed", "0", "--output", out_str];
    args.extend_from_slice(extra);
    let status = Command::new(BIN).args(&args).status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(
        status.success(),
        "best-first --check-slow aborted on {input} (extra {extra:?}): the fast cost path undercounts the exact rewritten size for a cross-depth capture — `check_fast_vs_slow` `fast >= slow` violated"
    );

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

#[test]
fn uniform_weights() {
    let v = run("data/domains/towers/tower_batch_50_3600_ellisk_2019-03-26T10.51.16__bench002_it3.json", &[], "uniform");
    bless_or_check("data/expected_outputs/fast_cost_upper_bound/towers.out.json", &v);
}

#[test]
fn nonuniform_weights() {
    let v = run("data/domains/logo/logo_batch_50_1h_ellisk_2019-03-23T14.05.43__bench000_it0.json", &["--sym-var-cost", "100"], "nonuniform");
    bless_or_check("data/expected_outputs/fast_cost_upper_bound/logo.out.json", &v);
}
