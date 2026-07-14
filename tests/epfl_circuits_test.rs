//! Snapshot tests for abstraction search on the EPFL `epfl-circuits/*.json`
//! corpora. Per circuit: a fixed 4-abstraction SMC rollout (seed 1) in three
//! configs — no-rules `baseline`, factoring `live`, factoring `at-start` —
//! snapshotted, plus `corpus_regenerates` (regenerate the corpus from the
//! gitignored `<circuit>.aig`).
//!
//! Re-bless: `BLESS=1 cargo test --release --test epfl_circuits_test -- --test-threads=1`.

use serde_json::Value;
use std::{fs, path::Path, process::Command};

mod common;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// The best ruleset: De Morgan + size-reducing distributivity-factoring.
const FAC: &str = "data/domains/epfl-circuits/and_or_demorgan_factor.rewrites";

const NUM_ABSTRACTIONS: &str = "4";

/// Lower than table7's 5000 to keep CI fast (~7x); lands on the same final cost.
/// This suite is a regression guard, not the headline measurement (that's table7).
///
/// Caveat: `hyp.factoring.live` sits near an SMC basin boundary at 500 particles,
/// so its fixture reflects a fragile low-particle trajectory (final_cost ~11077).
/// At table7's 5000 particles it lands at ~8716 like every other seed.
const NUM_PARTICLES: &str = "500";

fn corpus_path(circuit: &str) -> String {
    format!("data/domains/epfl-circuits/{circuit}.json")
}

/// Frozen-fixture path for a circuit + config tag.
fn expected_path(circuit: &str, tag: &str) -> String {
    format!("data/expected_outputs/epfl-circuits/{circuit}.{tag}.out.json")
}

/// Runs the rollout and returns its output JSON with the non-deterministic and
/// per-step bookkeeping fields stripped, so it's a stable snapshot.
fn run_smc(circuit: &str, rules: Option<&str>, at_start: bool, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-{}-{}-{}.json", std::process::id(), circuit, tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let corpus = corpus_path(circuit);
    let mut cmd = Command::new(BIN);
    cmd.args([
        "-i",
        &corpus,
        "--output",
        out_str,
        "--search",
        "smc",
        "--language",
        "op-children-db",
        "--max-arity",
        "4",
        "--num-abstractions",
        NUM_ABSTRACTIONS,
        "--num-particles",
        NUM_PARTICLES,
        "--num-steps",
        "100",
        "--temperature",
        "100",
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
    assert!(status.success(), "SMC run failed for {circuit}/{tag}");

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
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir {}: {e}", parent.display()));
        }
        fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let mut expected: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        common::sort_keys(&mut expected);
        assert_eq!(value, expected, "fixture mismatch for {path} (run with BLESS=1 to update)");
    }
}

/// One rollout config: run it and bless/check against the circuit's fixture.
fn check_config(circuit: &str, rules: Option<&str>, at_start: bool, tag: &str) {
    let v = run_smc(circuit, rules, at_start, tag);
    bless_or_check(&expected_path(circuit, tag), &v);
}

/// Regenerate `<circuit>.json` from `<circuit>.aig` and assert byte-identity.
/// The `.aig` is gitignored (run `fetch_aigs.py`). Locally it's skipped when
/// absent, but under CI (`$CI` set) a missing `.aig` is a hard failure — CI's
/// fetch step must have run, so absence there means the suite silently lost
/// coverage rather than a dev simply not fetching.
fn check_regen(circuit: &str) {
    let aig = format!("scripts/epfl-circuits/{circuit}.aig");
    if !Path::new(&aig).exists() {
        assert!(std::env::var_os("CI").is_none(), "{aig} absent under CI (scripts/epfl-circuits/fetch_aigs.py must run first)");
        eprintln!("skipping {circuit} regen: {aig} absent (run scripts/epfl-circuits/fetch_aigs.py)");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("egg-stitch-regen-{}-{}.json", std::process::id(), circuit));
    let tmp_str = tmp.to_str().expect("utf-8 temp path");
    let status = Command::new("python3").args(["scripts/epfl-circuits/aig_to_egg.py", circuit, "6", tmp_str]).status().unwrap_or_else(|e| panic!("spawn aig_to_egg.py: {e}"));
    assert!(status.success(), "aig_to_egg.py failed for {circuit}");
    let regen = fs::read(&tmp).unwrap_or_else(|e| panic!("read {}: {e}", tmp.display()));
    let _ = fs::remove_file(&tmp);
    let corpus = corpus_path(circuit);
    let committed = fs::read(&corpus).unwrap_or_else(|e| panic!("read {corpus}: {e}"));
    assert_eq!(
        regen, committed,
        "{corpus} no longer regenerates byte-identically from {circuit}.aig \
         (regenerate with: python3 scripts/epfl-circuits/aig_to_egg.py {circuit})"
    );
}

/// One snapshot suite per circuit. The whole binary is its own CI job (selected
/// by `binary(epfl_circuits_test)`), heavier than the parallel `test` job's cases.
macro_rules! circuit_suite {
    ($circuit:literal, $module:ident) => {
        mod $module {
            use super::*;

            #[test]
            fn baseline() {
                check_config($circuit, None, false, "baseline");
            }

            #[test]
            fn factoring_live() {
                check_config($circuit, Some(FAC), false, "factoring.live");
            }

            #[test]
            fn factoring_at_start() {
                check_config($circuit, Some(FAC), true, "factoring.at-start");
            }

            #[test]
            fn corpus_regenerates() {
                check_regen($circuit);
            }
        }
    };
}

circuit_suite!("hyp", hyp);
circuit_suite!("log2", log2);
circuit_suite!("multiplier", multiplier);
circuit_suite!("square", square);
circuit_suite!("voter", voter);
