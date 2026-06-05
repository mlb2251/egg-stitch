//! Bless/check (snapshot) tests for best-first (BFS) abstraction search on the
//! dreamcoder `list` and `physics` benchmark domains, run both with and without
//! their domain-specific rewrites (DSRs) and pinning *multiple* sequential
//! abstractions (`--num-abstractions 3`, so each round stacks on the previous
//! round's rewritten corpus). These are the lambda-calc analogue of the cogsci
//! drawing-domain suite in `cogsci_bfs_test.rs`.
//!
//! Unlike cogsci (one corpus file per domain), the dreamcoder domains ship as
//! many per-benchmark files under `data/domains/<domain>/*.json`. Each file is
//! searched independently; a domain+variant fixture aggregates the per-file
//! results into a single JSON object keyed by file name, so there is still just
//! one fixture per (domain, variant).
//!
//! Only best-first is pinned: it enumerates canonical patterns deterministically
//! (SMC is stochastic). Each run writes its `--output` JSON to a temp file;
//! non-deterministic and input-dependent fields (`timestamp`, `elapsed_secs`,
//! `input_file`, `rules_file`, `search`) and per-step bookkeeping on every
//! library entry (`num_steps_run`, `num_expansions`, `best_iteration`,
//! `best_history`) are stripped before comparison.
//!
//! Both variants run in CI: the no-DSR fixtures use the in-repo
//! `data/domains/<domain>/*.json` inputs, and the DSR fixtures use the in-repo
//! `data/domains/<domain>/<domain>.rewrites` rule files (copied from babble's
//! `benchmark-dsrs/<domain>.rewrites`).
//!
//! To regenerate all fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test dreamcoder_bfs_test -- --test-threads=1
//! ```

use serde_json::{Map, Value};
use std::{fs, path::PathBuf, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// Number of sequential abstractions each run pins. Three is enough to exercise
/// the stack: rounds 2 and 3 search the corpus rewritten by the prior round(s).
const NUM_ABSTRACTIONS: &str = "3";

/// Fixture path for a domain + variant tag (`nodsr` / `dsr`), mirroring the
/// `data/expected_outputs/<...>` layout used by the other snapshot suites.
fn expected_path(domain: &str, tag: &str) -> String {
    format!("data/expected_outputs/{domain}/{domain}.{tag}.out.json")
}

/// All `*.json` corpus files for a domain, sorted by file name so re-runs are
/// deterministic. The in-repo `<domain>.rewrites` DSR file is not a `.json`, so
/// it is naturally excluded.
fn input_files(domain: &str) -> Vec<PathBuf> {
    let dir = format!("data/domains/{domain}");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {dir}: {e}")).filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    assert!(!files.is_empty(), "no *.json inputs under {dir}");
    files
}

/// Runs best-first on a single lambda-calc corpus file (optionally with DSR
/// rules), writes its `--output` JSON to a unique temp file, reads it back, and
/// strips the non-deterministic / bookkeeping fields so the result is a stable
/// snapshot.
fn run_bfs_file(input: &str, rules: Option<&str>, tag: &str, idx: usize) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-dreamcoder-{}-{}-{}.json", std::process::id(), tag, idx));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", input, "--language", "lambda-calc", "--num-steps", "50000", "--num-abstractions", NUM_ABSTRACTIONS, "--max-arity", "2", "--output", out_str]);
    if let Some(r) = rules {
        cmd.args(["--rules", r]);
    }
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
                for k in ["num_steps_run", "num_expansions", "best_iteration", "best_history"] {
                    obj.remove(k);
                }
            }
        }
    }
    v
}

/// Searches every corpus file of `domain` and aggregates the per-file snapshots
/// into one object keyed by file name.
fn run_domain(domain: &str, rules: Option<&str>, tag: &str) -> Value {
    let mut map = Map::new();
    for (idx, path) in input_files(domain).into_iter().enumerate() {
        let name = path.file_name().and_then(|n| n.to_str()).expect("utf-8 file name").to_string();
        let input = path.to_str().expect("utf-8 input path");
        map.insert(name, run_bfs_file(input, rules, tag, idx));
    }
    Value::Object(map)
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

/// No-DSR variant: the inputs live in-repo, so this always runs.
fn check_nodsr(domain: &str) {
    let v = run_domain(domain, None, "nodsr");
    bless_or_check(&expected_path(domain, "nodsr"), &v);
}

/// DSR variant: runs best-first with the in-repo per-domain rewrite rules.
fn check_dsr(domain: &str) {
    let rules = format!("data/domains/{domain}/{domain}.rewrites");
    let v = run_domain(domain, Some(&rules), "dsr");
    bless_or_check(&expected_path(domain, "dsr"), &v);
}

#[test]
fn list_nodsr() {
    check_nodsr("list");
}

#[test]
fn list_dsr() {
    check_dsr("list");
}

#[test]
fn physics_nodsr() {
    check_nodsr("physics");
}

#[test]
fn physics_dsr() {
    check_dsr("physics");
}
