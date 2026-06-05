//! The wrap-nesting frontier gate (`--max-wrap-nesting`,
//! `SearchState::max_var_wrap_nesting_depth`) is a *per-variable* bound:
//! `∀v ∃(r,σ): spin-depth(v) ≤ cap` — every variable must be shallow in *some*
//! match, possibly a different one for each variable. That's the maximin
//! `max_v min_σ`, strictly weaker than the older minimax `min_σ max_v`
//! ("∃σ shallow for *every* v at once").
//!
//! The distinction is load-bearing on *crossed-spin* corpora, where the spin
//! alternates across variables and matches so that no single match is shallow
//! everywhere. Here the optimum `(g (da³(P ?#0 ?#1)) (db³(Q ?#0 ?#2)))` reuses
//! `?#0` across two wrappers that collapse on disjoint match families — `P` at
//! the `r1` roots (`(P x z) ⇒ x`), `Q` at the `r2` roots (`(Q y z) ⇒ y`). Every
//! match has exactly one wrapper spinning, so the *minimax* gate value is 1 and
//! the old gate would prune the whole abstraction at cap 0, settling for a
//! worse one (cost 60). But each of `?#0`/`?#1`/`?#2` is shallow in *some* match
//! (`?#1` at the r2 roots, `?#2` at r1, `?#0` via its `Q`-occurrence), so the
//! *maximin* gate value is 0 — it keeps the optimum (cost 54) even at cap 0.
//!
//! Termination still holds (the heap drains): wrapping the pattern one level
//! deeper buries some variable past the cap in *every* match, so the re-wrap
//! tower is finite.

use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/test/crossed_wrap_collapse.json";
const RULES: &str = "data/test/crossed_wrap_collapse.rewrites";

/// Runs best-first at the given `--max-wrap-nesting` and returns the output JSON.
fn run(cap: &str) -> Value {
    let out = std::env::temp_dir().join(format!("egg-stitch-wrapcap-{}-{}.json", std::process::id(), cap));
    let out_str = out.to_str().expect("utf-8 temp path");
    let status = Command::new(BIN)
        .args(["--search", "best-first", "--language", "op-children", "--input", INPUT, "--rules", RULES, "--num-steps", "60000", "--num-abstractions", "1", "--max-arity", "3", "--max-wrap-nesting", cap, "--output", out_str])
        .status()
        .unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed (cap={cap})");
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

/// At the most aggressive cap (0) the maximin gate still keeps the crossed-spin
/// optimum — pinning that the gate is `max_v min_σ`, not the minimax `min_σ max_v`
/// (which would prune it here and report cost 60). The search still terminates.
#[test]
fn maximin_gate_keeps_crossed_spin_optimum_at_cap_zero() {
    let v = run("0");

    // Termination: the re-wrap tower stays finite, so the heap drains.
    assert_eq!(v["heap_sizes_at_end"][0].as_u64(), Some(0), "search must terminate (heap drains to 0)");

    // The depth-1 crossed optimum survives cap 0 (each variable shallow somewhere).
    assert_eq!(pattern(&v), "fn_0: (g (da1 (da2 (da3 (P ?#0 ?#1)))) (db1 (db2 (db3 (Q ?#0 ?#2)))))", "maximin gate must keep the crossed optimum at cap 0 (minimax would prune it for a cost-60 abstraction)");
    assert_eq!(cost(&v), 54, "cost drifted (corpus/heuristics changed)");

    // Cap-invariant: a looser cap finds the same optimum — the gate never drops it.
    assert_eq!(cost(&run("5")), 54, "a looser cap must agree (the gate doesn't move this optimum)");
}
