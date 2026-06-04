//! End-to-end regression for `strip_dominated_wrappers`: stripping a dominated
//! wrapper must never change a pattern's optimum.
//!
//! This corpus broke the original rule (a) (`redundant_r`), which dropped a
//! self-wrapping subst `σ` at root `r` whenever some sibling subst `σ'`
//! *progressed* at the wrapper node, on the theory that σ' "keeps a genuine cover
//! of `r`". But σ and σ' bind the metavars to different e-classes, so σ' can be a
//! strictly more expensive rewrite of `r`. When the pattern internalizes
//! structure no shallower pattern reproduces — here a metavar reuse straddling
//! the wrapper — there is no cheaper alternative, so dropping σ strictly worsened
//! the optimum (the search scored the *same* abstraction 54 vs 42).
//!
//! The current strip drops only *vacuous* wrappers (no-op in every match); σ's
//! wrapper `P` is genuine at the 8 sites, so it isn't vacuous and σ is kept. (The
//! tower that vac can't reach is bounded separately by `wrap_nesting_depth` /
//! `--max-wrap-nesting`; σ has spin-depth 1, well under the cap.) So σ survives
//! and the optimum is preserved — this test pins exactly that.
//!
//! The corpus:
//!   - 8 genuine sites `(f (k pj) (D (D (D (D (D (P pj qj))))))`, `qj ≠ z`, so
//!     the abstraction `B = (f (k ?a) D⁵(P ?a ?b))` fires with `?a` **reused**
//!     across `(k ?a)` and `(P ?a ?b)`.
//!   - one root `r` written both as the expensive genuine form
//!     `(f (k THREE) D⁵(P THREE NEGTWO))` and the cheap wrap form
//!     `(f (k one) D⁵(P one z))`; rules collapse them into the *same* e-class:
//!       `pid:  (P ?x z) => ?x`            (the no-op self-loop ⇒ `has_cycle`)
//!       `fold: (P THREE NEGTWO) => one`   (genuine `+` folds to the same class)
//!       `keq:  (k THREE) => (k one)`      (`k` is non-injective: both reach `r`)
//!
//! At `r`, `B` matches two ways: `σ` (`?a=one,  ?b=z`, self-wrap at `P`, cheap)
//! and `σ'` (`?a=THREE, ?b=NEGTWO`, progresses at `P`, expensive). The old rule
//! dropped σ on seeing σ' progress; the splice `(f (k ?a) D⁵ ?a)` that "dominates
//! on a sibling branch" cannot match the genuine sites (it forces `k`'s arg to
//! equal the `P`-subtree), so nothing recovered σ's cheap rewrite of `r`. Both
//! modes find the *identical* abstraction `(f (k ?#0) (D⁵ (P ?#0 ?#1)))`; the bug
//! showed only in the cost (54 vs 42).
//!
//! This test pins the soundness invariant: stripping must not raise the cost
//! (`cost_on == cost_off`).

use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/test/strip_redundant_sibling_unsound.json";
const RULES: &str = "data/test/strip_redundant_sibling_unsound.rewrites";

/// Runs best-first on the counterexample corpus and returns the parsed output
/// JSON. `strip = false` passes `--no-opt-strip-wrap` (the no-strip baseline).
fn run(strip: bool, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-redundant-{}-{}.json", std::process::id(), tag));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--language", "op-children", "--input", INPUT, "--rules", RULES, "--num-steps", "2000", "--num-abstractions", "1", "--max-arity", "3", "--output", out_str]);
    if !strip {
        cmd.arg("--no-opt-strip-wrap");
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed (strip={strip})");
    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse output JSON: {e}"))
}

fn cost(v: &Value) -> u64 {
    v["final_cost"].as_u64().expect("final_cost present")
}

fn pattern(v: &Value) -> String {
    v["library"][0]["pattern"].as_str().expect("library[0].pattern present").to_string()
}

/// The cost-guarded rule (a) keeps the cheap wrap subst, so stripping does not
/// change the optimal cost of the discovered abstraction.
#[test]
fn redundant_sibling_strip_preserves_optimal_cost() {
    let off = run(false, "off");
    let on = run(true, "on");

    // Same abstraction is found either way — the only question is the cost
    // assigned to it.
    assert_eq!(pattern(&off), pattern(&on), "both modes should discover the same abstraction");
    assert_eq!(pattern(&on), "fn_0: (f (k ?#0) (D (D (D (D (D (P ?#0 ?#1)))))))", "unexpected abstraction — corpus/heuristics drifted, counterexample may be stale");

    let (c_off, c_on) = (cost(&off), cost(&on));

    // Soundness: stripping a *dominated* subst must not change the optimum. The
    // cost guard on rule (a) keeps the cheap wrap subst at the shared root, so
    // the strip run matches the no-strip baseline. (Before the fix this was
    // 54 > 42.)
    assert_eq!(c_on, c_off, "strip must not change the optimal cost: off={c_off}, on={c_on}");
}
