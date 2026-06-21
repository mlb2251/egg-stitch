//! Snapshot tests for the AIG-multiplier abstraction search on the
//! `epfl-circuits/mult.json` corpus. Each config runs a fixed 4-abstraction SMC
//! rollout (`--seed 1`, deterministic) with the factoring ruleset — no-rules
//! `baseline`, `live`, and `at-start` — and snapshots the output JSON with the
//! non-deterministic fields stripped. `corpus_regenerates_from_aig` additionally
//! pins that `mult.json` regenerates byte-for-byte from `multiplier.aig`.
//!
//! Re-bless after an intended change:
//! `BLESS=1 cargo test --release --test aig_mult_test -- --test-threads=1`.

use serde_json::Value;
use std::{fs, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// The committed op-children corpus (input-bounded K=6 cuts of `multiplier.aig`).
const CORPUS: &str = "data/domains/epfl-circuits/mult.json";

/// The best ruleset: De Morgan + size-reducing distributivity-factoring.
const FAC: &str = "data/domains/epfl-circuits/and_or_demorgan_factor.rewrites";

/// Number of sequential abstractions each rollout pins. Four is enough to
/// exercise the live-vs-at-start divergence while staying cheap in CI.
const NUM_ABSTRACTIONS: &str = "4";

/// Fixture path for a config tag, mirroring the `data/expected_outputs/<...>`
/// layout used by the other snapshot suites.
fn expected_path(tag: &str) -> String {
    format!("data/expected_outputs/epfl-circuits/{tag}.out.json")
}

/// Runs the fixed SMC rollout on the multiplier corpus (optionally with a DSR
/// file, optionally DSRs-at-start-only), writes its `--output` JSON to a unique
/// temp file, reads it back, and strips the non-deterministic / bookkeeping
/// fields so the result is a stable snapshot.
fn run_smc(rules: Option<&str>, at_start: bool, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-mult-{}-{}.json", std::process::id(), tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args([
        "-i",
        CORPUS,
        "--output",
        out_str,
        "--search",
        "smc",
        "--language",
        "op-children",
        "--max-arity",
        "4",
        "--num-abstractions",
        NUM_ABSTRACTIONS,
        "--num-particles",
        "5000",
        "--num-steps",
        "100",
        "--temperature",
        "1000",
        "--seed",
        "1",
        "--iter-limit",
        "30",
    ]);
    if let Some(r) = rules {
        cmd.args(["-r", r]);
    }
    if at_start {
        cmd.arg("--only-use-dsrs-at-start");
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "SMC run failed for {tag}");

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

// Test names are prefixed `aig_mult_` so a single nextest filter
// (`test(/aig_mult/)`) routes the whole suite to its own CI job — these SMC
// runs are far heavier (memory + time) than the cogsci best-first snapshots, so
// they run isolated and single-threaded rather than in the parallel `test` job.

#[test]
fn aig_mult_baseline() {
    let v = run_smc(None, false, "baseline");
    bless_or_check(&expected_path("baseline"), &v);
}

#[test]
fn aig_mult_factoring_live() {
    let v = run_smc(Some(FAC), false, "factoring.live");
    bless_or_check(&expected_path("factoring.live"), &v);
}

#[test]
fn aig_mult_factoring_at_start() {
    let v = run_smc(Some(FAC), true, "factoring.at-start");
    bless_or_check(&expected_path("factoring.at-start"), &v);
}

/// Provenance guard: the committed `mult.json` must regenerate byte-for-byte from
/// `multiplier.aig` via `scripts/epfl-circuits/aig_to_egg.py` (input-bounded K=6
/// cuts, deterministic stride-sample). Catches any drift between the .aig and the
/// corpus the rollout fixtures above are blessed against. Regenerates to a temp
/// path so the committed corpus is never touched.
#[test]
fn aig_mult_corpus_regenerates_from_aig() {
    let tmp = std::env::temp_dir().join(format!("egg-stitch-mult-regen-{}.json", std::process::id()));
    let tmp_str = tmp.to_str().expect("utf-8 temp path");
    let status = Command::new("python3")
        .args(["scripts/epfl-circuits/aig_to_egg.py", "scripts/epfl-circuits/multiplier.aig", "6", "mult", tmp_str])
        .status()
        .unwrap_or_else(|e| panic!("spawn aig_to_egg.py: {e}"));
    assert!(status.success(), "aig_to_egg.py failed");
    let regen = fs::read(&tmp).unwrap_or_else(|e| panic!("read {}: {e}", tmp.display()));
    let _ = fs::remove_file(&tmp);
    let committed = fs::read(CORPUS).unwrap_or_else(|e| panic!("read {CORPUS}: {e}"));
    assert_eq!(
        regen, committed,
        "{CORPUS} no longer regenerates byte-identically from multiplier.aig \
         (regenerate with: python3 scripts/epfl-circuits/aig_to_egg.py)"
    );
}
