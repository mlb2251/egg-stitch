//! Bless/check tests for `--higher-order` capture.
//!
//! Mirrors `stitch_compat_test`'s harness: invokes the cargo-built binary on a
//! frozen JSON corpus, captures `--output` JSON, strips noise fields, and
//! compares against a fixture under `data/expected_outputs/higher-order/`.
//! Each corpus is run twice — once with `--higher-order` enabled and once
//! without — so the fixtures make it easy to read off what HO actually
//! changes (if anything) versus the plain baseline.
//!
//! To regenerate the fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --test higher_order_test -- --test-threads=1
//! ```
//!
//! Note on cost-model interactions (also captured in the fixtures): under
//! default weights, plain capture at `var_depth = 0` is always available as an
//! alternative to HO at `var_depth > 0`, because `subst_is_sound` permits any
//! fv index `≥ 0`. So search will typically prefer a plain depth-0 pattern
//! whenever both compress equivalently, since the HO body pays an extra
//! `(app + sym_var) * ho_arity` per metavar use. The fixtures pin this — if a
//! future cost-model change makes HO actually win, the bless diff will show
//! it.

use serde_json::Value;
use std::{fs, path::Path, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

fn expected_path(input: &str, suffix: &str) -> String {
    let relative = input.strip_prefix("data/domains/").expect("expected input under data/domains/");
    let stem = relative.strip_suffix(".json").unwrap_or(relative);
    format!("data/expected_outputs/{stem}.{suffix}.out.json")
}

fn temp_output_path(input: &str, tag: &str) -> std::path::PathBuf {
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    std::env::temp_dir().join(format!("egg-stitch-ho-{}-{}-{}.json", std::process::id(), stem, tag))
}

/// Runs best-first on `input` with the given extra args, returns the
/// `--output` JSON with non-deterministic fields stripped.
fn run_bf(input: &str, extra_args: &[&str], tag: &str) -> Value {
    let out = temp_output_path(input, tag);
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", input, "--check-slow", "--num-abstractions", "1", "--num-steps", "10000", "--output", out_str]);
    cmd.args(extra_args);
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed for {input} ({tag})");

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

/// Runs the corpus twice — without and with `--higher-order` — and pins each
/// run separately. Two fixtures (`<stem>.plain.out.json` and
/// `<stem>.ho.out.json`) make the diff between the two runs immediately
/// visible.
fn check_ho_pair(input: &str, base_args: &[&str]) {
    let plain = run_bf(input, base_args, "plain");
    let mut ho_args: Vec<&str> = base_args.to_vec();
    ho_args.push("--higher-order");
    let ho = run_bf(input, &ho_args, "ho");
    bless_or_check(&expected_path(input, "plain"), &plain, &format!("{input} (plain)"));
    bless_or_check(&expected_path(input, "ho"), &ho, &format!("{input} (ho)"));
}

const LAMBDA: &[&str] = &["--language", "lambda-calc"];

/// Five programs sharing `(lam (foo (bar _)))` where the trailing slot is a
/// distinct closed-head application of `$0`. Plain depth-0 capture
/// `(foo (bar (?#0 $0)))` works (the leaf head is closed). With HO, the
/// search additionally considers the same shape lifted under the lam:
/// `(lam (foo (bar ?#0)))` where `?#0` captures `(@ X $0)` open subterms.
/// The two should compress equivalently — both fixtures pin which one
/// best-first picks.
#[test]
fn shared_lam_uniform_bottom() {
    check_ho_pair("data/domains/higher-order/uniform-bottom.json", LAMBDA);
}

/// Programs whose bottom shapes vary in *how* they use `$0` (head, middle,
/// trailing, bare). No single arity-1 plain pattern covers all of them; the
/// HO pattern `(lam (foo (bar ?#0)))` does. Pins what each setting finds.
#[test]
fn shared_lam_varying_bottom() {
    check_ho_pair("data/domains/higher-order/varying-bottom.json", LAMBDA);
}

/// Minimal: each program is just `(lam (h $0))` for varying head leaf. There
/// is no shared outer structure besides `(lam _)` itself, so any compression
/// must put a `lam` inside the abstraction body — i.e. is HO-shaped at
/// `var_depth > 0`.
#[test]
fn minimal_lam_varying_head() {
    check_ho_pair("data/domains/higher-order/minimal-head.json", LAMBDA);
}

/// Same varying-bottom inner shapes as `varying-bottom.json`, but wrapped in
/// a chunky outer `(+ a b c d e f (lam …))` so there's a lot of shared
/// non-lam structure surrounding the variation. Tests whether outer context
/// shifts the optimum from plain-depth-0-inside-lam to a deeper abstraction
/// that includes the outer skeleton.
#[test]
fn shared_lam_with_outer_context() {
    check_ho_pair("data/domains/higher-order/outer-context.json", LAMBDA);
}
