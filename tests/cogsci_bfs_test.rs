//! Bless/check (snapshot) tests for best-first (BFS) abstraction search on the
//! cogsci drawing domains, run both with and without their domain-specific
//! rewrites (DSRs) and pinning *multiple* sequential abstractions
//! (`--num-abstractions 3`, so each round stacks on the previous round's
//! rewritten corpus).
//!
//! Only best-first is pinned: it enumerates canonical patterns deterministically
//! (SMC is stochastic and doesn't converge reliably on these 250-program
//! corpora). Each run writes its `--output` JSON to a temp file; non-deterministic
//! and input-dependent fields (`timestamp`, `elapsed_secs`, `input_file`,
//! `rules_file`, `search`) and per-step bookkeeping on every library entry
//! (`num_steps_run`, `num_expansions`, `best_iteration`, `best_history`) are
//! stripped before comparison.
//!
//! Both variants run in CI: the no-DSR fixtures use the in-repo
//! `data/domains/cogsci/<domain>.json` inputs, and the DSR fixtures use the
//! in-repo `data/domains/cogsci/<domain>.rewrites` rule files (copied from
//! babble's `benchmark-dsrs/drawings.<domain>.rewrites`).
//!
//! A third variant pins the `--max-forced-expansion` prune (DSR + cap; tag
//! `dsr-mfe<cap>`). The cap is chosen so the prune genuinely fires across the
//! stacked rounds: at `cap = 3` it drops forced-expansion abstractions on
//! `wheels` and `furniture` (changing the result vs. the uncapped DSR run) while
//! being a safe no-op on `dials` / `nuts-bolts`. These snapshots are the
//! regression guard for `within_forced_expansion_cap` — breaking the prune (so
//! it never fires, or over-fires) shifts these costs off their frozen values.
//!
//! To regenerate all fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test cogsci_bfs_test -- --test-threads=1
//! ```

use serde_json::Value;
use std::{fs, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// Directory holding the per-domain DSR (`.rewrites`) files, in-repo alongside
/// the `<domain>.json` inputs.
const DSR_DIR: &str = "data/domains/cogsci";

/// Number of sequential abstractions each run pins. Three is enough to exercise
/// the stack: rounds 2 and 3 search the corpus rewritten by the prior round(s).
const NUM_ABSTRACTIONS: &str = "3";

/// `--max-forced-expansion` cap for the `dsr-mfe` variant, in symbols. Tuned so
/// the prune fires on at least one domain over the three stacked rounds; see the
/// module docstring.
const MFE_CAP: &str = "3";

/// Fixture path for a domain + variant tag (`nodsr` / `dsr`), mirroring the
/// `data/expected_outputs/<...>` layout used by the other snapshot suites.
fn expected_path(domain: &str, tag: &str) -> String {
    format!("data/expected_outputs/cogsci/{domain}.{tag}.out.json")
}

/// Runs best-first on a cogsci domain (optionally with DSR rules), writes its
/// `--output` JSON to a unique temp file, reads it back, and strips the
/// non-deterministic / bookkeeping fields so the result is a stable snapshot.
fn run_bfs(domain: &str, rules: Option<&str>, mfe: Option<&str>, tag: &str) -> Value {
    let input = format!("data/domains/cogsci/{domain}.json");
    let out = std::env::temp_dir().join(format!("egg-stitch-cogsci-{}-{}-{}.json", std::process::id(), domain, tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", &input, "--num-steps", "50000", "--num-abstractions", NUM_ABSTRACTIONS, "--max-arity", "2", "--output", out_str]);
    if let Some(r) = rules {
        cmd.args(["--rules", r]);
    }
    if let Some(cap) = mfe {
        cmd.args(["--max-forced-expansion", cap]);
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed for {input}");

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

/// No-DSR variant: the input lives in-repo, so this always runs.
fn check_nodsr(domain: &str) {
    let v = run_bfs(domain, None, None, "nodsr");
    bless_or_check(&expected_path(domain, "nodsr"), &v);
}

/// DSR variant: runs best-first with the in-repo per-domain rewrite rules.
fn check_dsr(domain: &str) {
    let rules = format!("{DSR_DIR}/{domain}.rewrites");
    let v = run_bfs(domain, Some(&rules), None, "dsr");
    bless_or_check(&expected_path(domain, "dsr"), &v);
}

/// Forced-expansion-pruned DSR variant: the DSR run with `--max-forced-expansion
/// MFE_CAP`. Pins that the prune behaves consistently across the stacked rounds.
fn check_dsr_mfe(domain: &str) {
    let rules = format!("{DSR_DIR}/{domain}.rewrites");
    let tag = format!("dsr-mfe{MFE_CAP}");
    let v = run_bfs(domain, Some(&rules), Some(MFE_CAP), &tag);
    bless_or_check(&expected_path(domain, &tag), &v);
}

#[test]
fn dials_nodsr() {
    check_nodsr("dials");
}

#[test]
fn dials_dsr() {
    check_dsr("dials");
}

#[test]
fn wheels_nodsr() {
    check_nodsr("wheels");
}

#[test]
fn wheels_dsr() {
    check_dsr("wheels");
}

#[test]
fn furniture_nodsr() {
    check_nodsr("furniture");
}

#[test]
fn furniture_dsr() {
    check_dsr("furniture");
}

#[test]
fn nuts_bolts_nodsr() {
    check_nodsr("nuts-bolts");
}

#[test]
fn nuts_bolts_dsr() {
    check_dsr("nuts-bolts");
}

#[test]
fn dials_dsr_mfe() {
    check_dsr_mfe("dials");
}

#[test]
fn wheels_dsr_mfe() {
    check_dsr_mfe("wheels");
}

#[test]
fn furniture_dsr_mfe() {
    check_dsr_mfe("furniture");
}

#[test]
fn nuts_bolts_dsr_mfe() {
    check_dsr_mfe("nuts-bolts");
}
