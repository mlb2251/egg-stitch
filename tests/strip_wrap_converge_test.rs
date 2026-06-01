//! Convergence regression for `SearchState::strip_dominated_wrappers`.
//!
//! Both corpora here are deliberately *compressive* (many distinct, reusable
//! shapes) and wrap those shapes in an identity that an `=>` rule collapses to a
//! no-op self-loop. Those self-loops spawn an unbounded tower of equivalent
//! re-wrapped patterns; because the corpus is compressive, the lower bound stays
//! well below the best cost for many tower patterns, so the pattern-size term
//! never dominates the bound. Without the dominated-wrapper strip best-first
//! can't prune the tower and the heap never drains — it grinds to the `num_steps`
//! cap. With the strip, the no-op substs are removed, the tower collapses, the
//! frontier is exhausted (heap drains to 0), and a real abstraction is still found.
//!
//! - `converge_tower`: the wrapper is a single node `(repeat shape 1 (M 1 0 0 0))`
//!   collapsed by `rep_1` — a *direct-child* self-loop.
//! - `nested_loop_tower`: the wrapper is two nodes `(f (g shape))` collapsed by
//!   `(f (g ?x)) => ?x` — the self-loop is at a *grandchild*, which the
//!   direct-child strip (and its 1-cycle gate) misses entirely.

use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
// Comfortably above the ~200 expansions the strip needs to converge, but far
// below where the un-stripped tower would (it never does) — keeps the run fast.
const CAP: &str = "2000";

/// Runs best-first on `input`+`rules` and returns the parsed `--output` JSON.
/// `strip = false` passes `--no-opt-strip-wrap`, reproducing pre-strip behavior.
fn run(input: &str, rules: &str, strip: bool, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-converge-{}-{}.json", std::process::id(), tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", input, "--rules", rules, "--num-steps", CAP, "--num-abstractions", "1", "--max-arity", "2", "--output", out_str]);
    if !strip {
        cmd.arg("--no-opt-strip-wrap");
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed (strip={strip})");
    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse output JSON: {e}"))
}

/// Asserts the strip converges this corpus: heap drains to 0 *and* a real, reused
/// abstraction is still found (so the corpus isn't degenerate).
fn assert_converges_with_strip(input: &str, rules: &str, tag: &str) {
    let v = run(input, rules, true, tag);
    let heap = v["heap_sizes_at_end"][0].as_u64().expect("heap_sizes_at_end[0] present");
    assert_eq!(heap, 0, "heap should drain to 0 with the strip (search converged)");
    let ratio = v["compression_ratio"].as_f64().expect("compression_ratio present");
    assert!(ratio > 1.2, "expected a real compressive abstraction, got {ratio}x");
    let matches = v["library"][0]["num_matches"].as_u64().expect("library[0].num_matches");
    assert!(matches >= 2, "the abstraction should be reused, got {matches} matches");
}

/// Asserts that *without* the strip the corpus never converges at the cap — i.e.
/// the tower risk is real (this is the `main`/pre-strip behavior).
fn assert_no_drain_without_strip(input: &str, rules: &str, tag: &str) {
    let v = run(input, rules, false, tag);
    let heap = v["heap_sizes_at_end"][0].as_u64().expect("heap_sizes_at_end[0] present");
    assert!(heap > 0, "without the strip the heap must NOT drain (tower never pruned), got {heap}");
}

const TOWER_INPUT: &str = "data/test/converge_tower.json";
const TOWER_RULES: &str = "data/test/converge_tower.rewrites";
// `(f (g ?x)) => ?x`: the self-loop is two levels down, not at a direct child.
const NESTED_INPUT: &str = "data/test/nested_loop_tower.json";
const NESTED_RULES: &str = "data/test/nested_loop_tower.rewrites";

/// Direct-child self-loop (`repeat _ 1 _`) converges with the strip.
#[test]
fn direct_child_loop_drains_heap() {
    assert_converges_with_strip(TOWER_INPUT, TOWER_RULES, "tower-on");
}

/// Direct-child loop never drains without the strip — the tower risk is real.
#[test]
fn direct_child_loop_no_drain_without_strip() {
    assert_no_drain_without_strip(TOWER_INPUT, TOWER_RULES, "tower-off");
}

/// Grandchild self-loop (`(f (g ?x)) => ?x`) converges with the strip. This is
/// the regression for the non-direct-child case: it fails if the strip only
/// looks at direct children (or its gate only detects 1-cycles).
#[test]
fn nested_loop_drains_heap() {
    assert_converges_with_strip(NESTED_INPUT, NESTED_RULES, "nested-on");
}

/// Grandchild loop never drains without the strip — the tower risk is real.
#[test]
fn nested_loop_no_drain_without_strip() {
    assert_no_drain_without_strip(NESTED_INPUT, NESTED_RULES, "nested-off");
}
