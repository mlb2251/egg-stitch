//! `--max-wrap-nesting` is sound-as-*incomplete*: at a low enough cap it can
//! drop the true optimum. This test is the counterexample — the same corpus
//! yields a strictly better abstraction at cap 1 than at cap 0.
//!
//! Why a counterexample exists at all (the splice argument has a hole). A spin
//! node is a *no-op* wrapper — its e-class equals a descendant's — so it can
//! always be spliced out, and the spliced (spin-0) pattern matches the same root
//! through the collapsed e-node. That makes any *single* spin node redundant.
//! But the frontier gate (`min_wrap_nesting_depth`) prunes a search state when
//! *every* subst has spin-depth ≥ cap+1, and different substs can spin at
//! *different* nodes. When the spin alternates between two wrapper positions and
//! a reused metavar straddles both, no single splice covers every root and the
//! all-holes generalization can't reproduce the reuse — so the structured
//! abstraction is uniquely optimal yet every one of its substs is spinning.
//!
//! The corpus:
//!   - Two big shared wrappers `(da1 (da2 (da3 …)))` and `(db1 (db2 (db3 …)))`,
//!     each ending in a conditional collapse: `(P ?x z) => ?x`, `(Q ?y z) => ?y`.
//!   - `r1` roots `(g (da³(P aₖ z)) (db³(Q aₖ w)))` — `P` collapses (spin at the
//!     `P` node), `Q` is genuine (`w ≠ z`).
//!   - `r2` roots `(g (da³(P bₖ w)) (db³(Q bₖ z)))` — mirror image: `Q` collapses,
//!     `P` is genuine.
//! The optimum `(g (da³(P ?#0 ?#1)) (db³(Q ?#0 ?#2)))` reuses `?#0` across both
//! wrappers (each root binds it to one atom passed once). Every subst spins —
//! `P` at the `r1` roots, `Q` at the `r2` roots — so its `min_wrap_nesting_depth`
//! is 1. At cap 0 the gate prunes it and a wrapper-less, less compressive
//! abstraction wins; at cap 1 it survives and wins.

use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/test/crossed_wrap_collapse.json";
const RULES: &str = "data/test/crossed_wrap_collapse.rewrites";

/// Runs best-first at `cap` on `input`+`rules` and returns the output JSON.
fn run_on(input: &str, rules: &str, cap: &str) -> Value {
    // Disambiguate by input stem too: the two tests run in parallel and would
    // otherwise collide on a `(pid, cap)`-only temp path.
    let stem = std::path::Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("x");
    let out = std::env::temp_dir().join(format!("egg-stitch-wrapcap-{}-{}-{}.json", std::process::id(), stem, cap));
    let out_str = out.to_str().expect("utf-8 temp path");
    let status = Command::new(BIN)
        .args(["--search", "best-first", "--language", "op-children", "--input", input, "--rules", rules, "--num-steps", "60000", "--num-abstractions", "1", "--max-arity", "3", "--max-wrap-nesting", cap, "--output", out_str])
        .status()
        .unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed (cap={cap})");
    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse output JSON: {e}"))
}

/// Runs best-first at `cap` on the crossed-collapse counterexample corpus.
fn run(cap: &str) -> Value {
    run_on(INPUT, RULES, cap)
}

fn cost(v: &Value) -> u64 {
    v["final_cost"].as_u64().expect("final_cost present")
}

fn pattern(v: &Value) -> String {
    v["library"][0]["pattern"].as_str().expect("library[0].pattern present").to_string()
}

/// At cap 0 the spin-1 optimum is gate-pruned, so a wrapper-less abstraction
/// wins; at cap 1 it survives and compresses strictly better. The two caps must
/// therefore disagree — pinning that the bound really can move the optimum.
#[test]
fn low_cap_drops_the_crossed_spin_optimum() {
    let (c0, c1) = (run("0"), run("1"));

    // cap 0: the wrappers are pruned away — every subst of the structured
    // abstraction spins, so the gate never lets it onto the frontier.
    assert_eq!(pattern(&c0), "fn_0: (g (da1 (da2 (da3 ?#0))) (db1 (db2 (db3 ?#1))))", "cap-0 abstraction drifted (corpus/heuristics changed)");
    // cap 1: the crossed-spin optimum survives (its depth is exactly 1).
    assert_eq!(pattern(&c1), "fn_0: (g (da1 (da2 (da3 (P ?#0 ?#1)))) (db1 (db2 (db3 (Q ?#0 ?#2)))))", "cap-1 abstraction drifted (corpus/heuristics changed)");

    // The cap-1 optimum is strictly cheaper — cap 0 genuinely lost compression.
    assert!(cost(&c1) < cost(&c0), "cap 1 must beat cap 0: c0={}, c1={}", cost(&c0), cost(&c1));
    assert_eq!((cost(&c0), cost(&c1)), (60, 54), "costs drifted (corpus/heuristics changed)");
}

/// The cap also bounds which subst the cost optimiser may *select* (use) — a
/// binder-independent restriction on the cost-selected substs, enforced in
/// `compute_cost_and_select`, distinct from the frontier gate.
///
/// The `strip_redundant_sibling_unsound` corpus discovers the *same* abstraction
/// `(f (k ?#0) (D⁵(P ?#0 ?#1)))` at every cap — its 8 genuine sites are spin-0,
/// so `min_wrap_nesting_depth` is 0 and the gate never prunes it. But at the
/// shared root `r` the only cheap rewrite binds via a *spin-1* subst `σ`
/// (`(P one z) ≡ one`). At cap ≥ 1 the optimiser may select `σ`, compressing `r`
/// to cost 42; at cap 0 the restriction forbids `σ`, so `r` falls back to a
/// pricier rewrite and the cost rises to 54. Same pattern, different cost —
/// pinning that the restriction acts on the *used* subst, not just the pattern.
#[test]
fn low_cap_forbids_a_gate_surviving_spin_subst() {
    const SR_INPUT: &str = "data/test/strip_redundant_sibling_unsound.json";
    const SR_RULES: &str = "data/test/strip_redundant_sibling_unsound.rewrites";
    let (c0, c1) = (run_on(SR_INPUT, SR_RULES, "0"), run_on(SR_INPUT, SR_RULES, "1"));

    // Same abstraction either way — the gate keeps it (genuine sites are spin-0).
    let pat = "fn_0: (f (k ?#0) (D (D (D (D (D (P ?#0 ?#1)))))))";
    assert_eq!(pattern(&c0), pat, "abstraction drifted (corpus/heuristics changed)");
    assert_eq!(pattern(&c1), pat, "abstraction drifted (corpus/heuristics changed)");

    // Only the *used* subst — hence the cost — differs: cap 0 forbids σ at r.
    assert!(cost(&c0) > cost(&c1), "cap 0 must forbid the spin subst and cost more: c0={}, c1={}", cost(&c0), cost(&c1));
    assert_eq!((cost(&c0), cost(&c1)), (54, 42), "costs drifted (corpus/heuristics changed)");
}
