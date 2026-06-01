//! Convergence regression for `SearchState::strip_dominated_wrappers`.
//!
//! `data/domains/converge/tower.json` is a deliberately *compressive* corpus
//! (many distinct, reusable shapes) whose `rep_1` DSR (`(repeat ?x 1 ?m) => ?x`)
//! turns every `(repeat shape 1 (M 1 0 0 0))` wrapper into a no-op self-loop.
//! Those self-loops spawn an unbounded tower of equivalent re-wrapped patterns.
//! Because the corpus is compressive, the lower bound stays well below the best
//! cost for many of those tower patterns, so the pattern-size term never
//! dominates the bound: without the dominated-wrapper strip best-first can't
//! prune the tower and the heap never drains — it grinds to the `num_steps` cap.
//!
//! With the strip, the no-op wrapper substs are removed, the tower collapses, and
//! the frontier is exhausted (heap drains to 0) while the real abstraction is
//! still found. These two tests pin both directions.

use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/domains/converge/tower.json";
const RULES: &str = "data/domains/converge/tower.rewrites";
// Comfortably above the ~200 expansions the strip needs to converge, but far
// below where the un-stripped tower would (it never does) — keeps the run fast.
const CAP: &str = "2000";

/// Runs best-first on the tower corpus and returns the parsed `--output` JSON.
/// `strip = false` passes `--no-opt-strip-wrap`, reproducing pre-strip behavior.
fn run(strip: bool, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-converge-{}-{}.json", std::process::id(), tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", INPUT, "--rules", RULES, "--num-steps", CAP, "--num-abstractions", "1", "--max-arity", "2", "--output", out_str]);
    if !strip {
        cmd.arg("--no-opt-strip-wrap");
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed (strip={strip})");
    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse output JSON: {e}"))
}

/// With the strip, the self-loop DSR converges: the heap drains to 0 and a real,
/// reused abstraction is still found (so this isn't a degenerate corpus).
#[test]
fn strip_wrap_drains_heap() {
    let v = run(true, "on");
    let heap = v["heap_size_at_end"].as_u64().expect("heap_size_at_end present");
    assert_eq!(heap, 0, "heap should drain to 0 with the strip (search converged)");
    let ratio = v["compression_ratio"].as_f64().expect("compression_ratio present");
    assert!(ratio > 1.2, "expected a real compressive abstraction, got {ratio}x");
    let matches = v["library"][0]["num_matches"].as_u64().expect("library[0].num_matches");
    assert!(matches >= 2, "the abstraction should be reused, got {matches} matches");
}

/// Without the strip, the same corpus never converges: the unbounded wrapper
/// tower keeps the frontier non-empty until the `num_steps` cap. This is the
/// `main`/pre-strip behavior — it pins that the strip is what makes it terminate.
#[test]
fn without_strip_heap_does_not_drain() {
    let v = run(false, "off");
    let heap = v["heap_size_at_end"].as_u64().expect("heap_size_at_end present");
    assert!(heap > 0, "without the strip the heap must NOT drain (tower never pruned), got {heap}");
}
