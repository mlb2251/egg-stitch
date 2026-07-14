//! Feature-invariant assertions that ride on top of the snapshot suite.
//!
//! The snapshots in `tests/snapshots.toml` pin the *exact* search output; these
//! tests assert a *structural property* that must hold regardless of any future
//! re-bless (a metavar reused N times, no free DB var in a body, one config
//! strictly beating another, …). They reuse the shared runner in
//! `tests/common/mod.rs` — either re-running a config (`common::run`) or reading
//! the blessed fixture (`common::read_fixture`) — so there is still exactly one
//! run recipe. The EPFL `.aig` corpus-regeneration checks live here too, as they
//! assert byte-identity against a (gitignored) source rather than a fixture.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

mod common;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// egg prints metavars as `?#0`; normalize to the cleaner `#0` form.
fn egg_to_stitch(s: &str) -> String {
    s.replace("?#", "#")
}

/// The abstraction bodies (with the `fn_N: ` prefix stripped) in a run's library.
fn abstraction_bodies(run: &Value) -> Vec<String> {
    run.get("library")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    let p = e.get("pattern").and_then(|p| p.as_str()).expect("pattern string");
                    egg_to_stitch(p.split_once(": ").expect("pattern prefixed with fn_N:").1)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Largest number of times any single metavar `#k` (k in 0..8) is reused in a
/// stitch-notation body.
fn max_var_reuse(body: &str) -> usize {
    (0..8).map(|k| body.matches(&format!("#{k}")).count()).max().unwrap_or(0)
}

/// Collapse an s-expression to a sorted multiset of its atoms, discarding
/// structure — used where associativity/commutativity make several abstraction
/// shapes equally valid.
fn all_symbols_hack(x: &str) -> Vec<String> {
    let x = x.replace(['(', ')'], " ");
    let mut symbols: Vec<_> = x.split_whitespace().map(|s| s.to_string()).collect();
    symbols.sort();
    symbols
}

/// True under `BLESS=1`. Assertions that read a committed fixture are skipped
/// then: the snapshot suite is concurrently rewriting those fixtures, so a
/// mid-bless read is racy and meaningless — they re-run in the next check pass.
fn blessing() -> bool {
    std::env::var("BLESS").is_ok()
}

/// The run's rewritten corpus, falling back to `original` when the field is absent.
fn rewritten_corpus(run: &Value, original: &[String]) -> Vec<String> {
    if let Some(arr) = run.get("rewritten_programs").and_then(|p| p.as_array()) {
        return arr.iter().filter_map(|s| s.as_str().map(String::from)).collect();
    }
    original.to_vec()
}

/// One library's single abstraction body, asserting there is exactly one.
fn sole_body(run: &Value, label: &str) -> String {
    let bodies = abstraction_bodies(run);
    assert_eq!(bodies.len(), 1, "{label}: expected exactly one abstraction, got {bodies:?}");
    bodies.into_iter().next().unwrap()
}

// ─── useless-var gate ────────────────────────────────────────────────────────

/// `--allow-useless-vars` lifts the default gate that rejects the reused-useless
/// arity-1 abstraction, so best-first returns it (deterministic; pinned exactly)
/// and SMC returns some optimum that reuses a var 4×.
#[test]
fn cex_allow_useless_vars() {
    let input = "data/domains/stitch/cex.json";
    let flag = &["--allow-useless-vars"];
    let bf = common::run("best-first", input, flag);
    assert_eq!(abstraction_bodies(&bf), vec!["(a b c d e f g h #0 #0 #0 #0)".to_string()], "best-first should return the useless-var abstraction the default gate forbids");
    let smc = common::run("smc", input, flag);
    for r in [&bf, &smc] {
        let body = sole_body(r, "cex_allow_useless_vars");
        assert!(max_var_reuse(&body) >= 4, "expected a useless var reused 4×, got body: {body}");
    }
}

/// Shifted-variant reuse must collapse both occurrences of the shared
/// `(+ $0 3 4 (lam (+ $1 6 7)))` into a single arity-1 abstraction — the whole
/// point of the variables-at-multiple-depths branch. (Snapshot: `reuse_at_different_depths`.)
#[test]
fn reuse_at_different_depths_is_arity_1() {
    for search in ["best-first", "smc"] {
        let v = common::run(search, "data/domains/stitch/reuse-at-different-depths.json", &["--language", "lambda-calc"]);
        let library = v.get("library").and_then(|l| l.as_array()).unwrap_or_else(|| panic!("{search}: missing library"));
        assert_eq!(library.len(), 1, "{search}: expected exactly one abstraction, got {library:#?}");
        let arity = library[0].get("arity").and_then(|a| a.as_u64()).unwrap_or_else(|| panic!("{search}: arity missing"));
        assert_eq!(arity, 1, "{search}: shifted-variant reuse must collapse to a single metavar (arity 1), got {arity}");
    }
}

// ─── associative/commutative abstraction shapes ──────────────────────────────

/// With bidirectional `+` rules several abstraction shapes are equally valid, so
/// this compares atom multisets rather than exact trees, and checks which
/// programs are rewritten.
#[test]
fn arith_rewrites() {
    let input = "data/domains/basic-apps/multi-arg-assoc.json";
    let extra_args = &["-r", "data/domains/basic-apps/app-arith.rewrites", "--language", "lambda-calc", "--max-arity", "0", "--seed", "0"];
    let bf = common::run("best-first", input, extra_args);
    let smc = common::run("smc", input, extra_args);
    let original: Vec<String> = serde_json::from_str(&std::fs::read_to_string(input).unwrap_or_else(|e| panic!("read {input}: {e}"))).unwrap_or_else(|e| panic!("parse {input}: {e}"));
    for r in &[bf, smc] {
        let bodies = abstraction_bodies(r);
        assert!(bodies.len() == 1, "expected exactly one abstraction");
        let abstr = all_symbols_hack(&bodies[0]);
        if abstr != ["+", "a", "b", "c", "d"] && abstr != ["+", "+", "a", "b", "c", "d"] && abstr != ["+", "+", "+", "a", "b", "c", "d"] {
            panic!("bad abstr: {abstr:?}");
        }
        let rewr = rewritten_corpus(r, &original).iter().map(|x| all_symbols_hack(x)).collect::<Vec<_>>();
        let rewr = rewr.iter().map(|x| x.iter().filter(|x| **x != <&str as Into<String>>::into("+")).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(rewr, vec![vec!["fn_0", "g"], vec!["f", "fn_0"], vec!["e", "fn_0"], vec!["*", "a", "b", "c", "d", "e"]]);
    }
}

// ─── cross-depth reuse invariants ────────────────────────────────────────────

/// `--allow-useless-vars` forces `opt_useless_inline` off; best-first must still
/// settle on the sound arity-1 mod-pattern (no cross-depth collapse).
#[test]
fn cross_depth_useless_inline_allow_useless_vars() {
    let bf = common::run("best-first", "data/domains/ho-bugs/cross_depth_useless_inline.json", &["--language", "lambda-calc", "--allow-useless-vars"]);
    assert_eq!(abstraction_bodies(&bf), vec!["(lam (map #0 $0))".to_string()], "inline off should still yield the sound arity-1 pattern");
}

/// The 3-way reuse (one metavar occurring three times) must stay reachable in
/// canonical merge order.
#[test]
fn dials_reroll_three_way_reuse() {
    let body = sole_body(
        &common::run("best-first", "data/test/dials_reroll_3way.json", &["--rules", "data/test/dials_reroll_3way.rewrites", "--max-forced-expansion", "3", "--max-arity", "2"]),
        "dials_reroll_3way",
    );
    assert!(max_var_reuse(&body) >= 3, "expected a metavar reused 3×, got max {} in body: {body}", max_var_reuse(&body));
}

/// The 3-way reuse must stay reachable even when dominance/expand order forces a
/// non-canonical merge.
#[test]
fn nuts_bolts_dominance_three_way_reuse() {
    let body = sole_body(&common::run("best-first", "data/test/nuts_bolts_3way_reuse.json", &["--rules", "data/test/nuts_bolts_3way_reuse.rewrites", "--max-arity", "2"]), "nuts_bolts_3way");
    assert!(max_var_reuse(&body) >= 3, "expected a metavar reused 3×, got max {} in body: {body}", max_var_reuse(&body));
}

/// Same, with `--allow-useless-vars` (dominance-reuse off): the 3-way survives
/// via the late post-expand merge path.
#[test]
fn nuts_bolts_dominance_three_way_reuse_allow_useless_vars() {
    let body = sole_body(
        &common::run("best-first", "data/test/nuts_bolts_3way_reuse.json", &["--rules", "data/test/nuts_bolts_3way_reuse.rewrites", "--max-arity", "2", "--allow-useless-vars"]),
        "nuts_bolts_3way (dominance off)",
    );
    assert!(max_var_reuse(&body) >= 3, "3-way reuse should survive dominance-off, got max {} in body: {body}", max_var_reuse(&body));
}

// ─── op-children-db free-var ban ─────────────────────────────────────────────

/// `op-children-db` bans free DB vars from an abstraction body, so no `$`
/// appears. (Snapshot: `op_children_db_free_var`.)
#[test]
fn op_children_db_bans_free_var_from_body() {
    if blessing() {
        return;
    }
    let v = common::read_fixture("data/expected_outputs/test/op_children_db_free_var.out.json");
    for body in abstraction_bodies(&v) {
        assert!(!body.contains('$'), "op-children-db must keep free DB vars out of the body, got `{body}`");
    }
}

/// Contrast: plain `op-children` parses `$0` as an ordinary leaf, so it stays
/// baked into the body. (Snapshot: `op_children_plain_free_var`.)
#[test]
fn op_children_plain_keeps_free_var_in_body() {
    if blessing() {
        return;
    }
    let v = common::read_fixture("data/expected_outputs/test/op_children_db_free_var.plain.out.json");
    let bodies = abstraction_bodies(&v);
    assert!(bodies.iter().any(|b| b.contains("$0")), "plain op-children should keep `$0` baked into the body, got {bodies:?}");
}

// ─── roll-over ───────────────────────────────────────────────────────────────

/// `--roll-over` reaches a strictly lower final cost than the default rebuild at
/// 2 abstractions. (Snapshots: `roll_over_glycol_default` / `_roll`.)
#[test]
fn roll_over_finds_cheaper_stack() {
    if blessing() {
        return;
    }
    let default_out = common::read_fixture("data/expected_outputs/test/roll_over_glycol.default.out.json");
    let roll_out = common::read_fixture("data/expected_outputs/test/roll_over_glycol.roll.out.json");
    let dc = default_out["final_cost"].as_u64().expect("final_cost present");
    let rc = roll_out["final_cost"].as_u64().expect("final_cost present");
    assert!(rc < dc, "expected --roll-over to beat the default rebuild, got roll={rc} default={dc}");
}

// ─── EPFL corpus regeneration ────────────────────────────────────────────────

/// Regenerate `<circuit>.json` from `<circuit>.aig` and assert byte-identity.
/// The `.aig` is gitignored (run `scripts/epfl-circuits/fetch_aigs.py`); locally
/// it's skipped when absent, but under CI (`$CI` set) a missing `.aig` is a hard
/// failure — CI's fetch step must have run, so absence means lost coverage.
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
    let regen = std::fs::read(&tmp).unwrap_or_else(|e| panic!("read {}: {e}", tmp.display()));
    let _ = std::fs::remove_file(&tmp);
    let corpus = format!("data/domains/epfl-circuits/{circuit}.json");
    let committed = std::fs::read(&corpus).unwrap_or_else(|e| panic!("read {corpus}: {e}"));
    assert_eq!(regen, committed, "{corpus} no longer regenerates byte-identically from {circuit}.aig (regenerate with: python3 scripts/epfl-circuits/aig_to_egg.py {circuit})");
}

#[test]
fn hyp_corpus_regenerates() {
    check_regen("hyp");
}
#[test]
fn log2_corpus_regenerates() {
    check_regen("log2");
}
#[test]
fn multiplier_corpus_regenerates() {
    check_regen("multiplier");
}
#[test]
fn square_corpus_regenerates() {
    check_regen("square");
}
#[test]
fn voter_corpus_regenerates() {
    check_regen("voter");
}
