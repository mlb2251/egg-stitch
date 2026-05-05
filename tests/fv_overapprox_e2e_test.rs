//! End-to-end check that the intersection-based fv analysis lets all three
//! programs rewrite under the natural arity-1 abstraction, even when the
//! `(* 0 ?x) => 0` rule pollutes one match's capture eclass.
//!
//! Corpus: three programs of shape `(big (chain (of (g X (lam X)))))` with
//! `X` ∈ {`xx`, `yy`, `(* 0 $0)`}. Under the rule every program's body
//! collapses to the same shape, and `(big (chain (of (g ?#0 (lam ?#0)))))`
//! matches all three with ?#0 = `xx`, `yy`, `0`.
//!
//! For the third program, ?#0 captures the eclass `{0, (* 0 $0)}` after the
//! rule fires. Intersection fv reports `{}` (correct semantic fv), so
//! `subst_is_sound` accepts it; AstSize extraction picks `0` for the
//! capture, satisfying `check_fvs_are_as_expected` post-extraction.

use serde_json::Value;
use std::{fs, path::Path, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/domains/fv-overapprox/annihilator.json";
const RULES: &str = "data/domains/fv-overapprox/annihilator.rewrites";

#[test]
fn annihilator_rule_should_unlock_all_three_matches() {
    let out_path = std::env::temp_dir().join(format!("egg-stitch-fv-overapprox-{}.json", std::process::id()));
    let out_str = out_path.to_str().expect("utf-8 temp path");
    let status = Command::new(BIN)
        .args([
            "--search",
            "best-first",
            "--input",
            INPUT,
            "--rules",
            RULES,
            "--language",
            "lambda-calc",
            "--sym-var-cost",
            "100",
            "--max-arity",
            "1",
            "--num-steps",
            "200000",
            "--num-abstractions",
            "1",
            "--check-slow",
            "--output",
            out_str,
        ])
        .status()
        .unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "egg-stitch run failed");

    let text = fs::read_to_string(&out_path).unwrap_or_else(|e| panic!("read {}: {e}", out_path.display()));
    let _ = fs::remove_file(&out_path);
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse output: {e}"));

    let library = v.get("library").and_then(|l| l.as_array()).expect("library array");
    let entry = library.first().expect("at least one abstraction");
    let pattern = entry.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
    assert_eq!(pattern, "fn_0: (big (chain (of (g ?#0 (lam ?#0)))))", "BF should land on the reuse pattern under --max-arity 1");

    // Under correct fv semantics, the polluted eclass denotes 0 (closed) and
    // ?#0 = 0 in P3 is sound at depth 1, so all three programs rewrite.
    let matches = entry.get("num_matches").and_then(|n| n.as_u64()).expect("num_matches");
    assert_eq!(matches, 3, "expected all three programs to be matched; over-approximation drops the polluted one");

    let rewritten = entry.get("rewritten_programs").and_then(|r| r.as_array()).expect("rewritten_programs");
    let expected = ["(fn_0 xx)", "(fn_0 yy)", "(fn_0 0)"];
    let actual: Vec<&str> = rewritten.iter().filter_map(|r| r.as_str()).collect();
    assert_eq!(actual.as_slice(), expected.as_slice(), "every program should rewrite under the abstraction (P3 currently doesn't)");
}

// Compile-time existence check so the test framework finds the fixtures.
#[allow(dead_code)]
fn _fixtures_exist() -> bool {
    Path::new(INPUT).exists() && Path::new(RULES).exists()
}
