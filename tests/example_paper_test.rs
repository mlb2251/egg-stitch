//! Worked example for the paper (e-stitch figure-1 shape): a commutative
//! ordering that does *not* fall out of canonicalization, so live rewriting
//! beats `--only-use-dsrs-at-start`.
//!
//! The rewrites (`data/domains/examples-paper/rules.rewrites`) are commutativity
//! of `+` plus the additive identity:
//! ```text
//! plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
//! add_zero:  ?x       <=> (+ 0 ?x)
//! ```
//!
//! The shared abstraction is `f0 = (+ ?x (* ?y ?y))` (i.e. `x + y²`, arity 2).
//! Its slots are filled with varied per-program subterms — bare symbols, larger
//! `(* ..)` / `(/ ..)` terms — and some programs are wrapped in an outer function
//! (`sqrt`, `f1`). Six programs are of `f0` shape; the seventh is the bare square
//! `(* (/ x 2) (/ x 2))`, which has no `+` and only fits `f0` after an `add_zero`
//! expansion `(+ 0 (* (/ x 2) (/ x 2))) = (f0 0 (/ x 2))`.
//!
//! * `corpus_a.json` writes every `f0` program with the square *second*, so a
//!   plain *syntactic* (rule-free) search finds `f0`.
//! * `corpus_b.json` is the same programs, commutatively scrambled (half put the
//!   square first). Because *both* operands of each `+` are per-program subterms
//!   (no shared anchor leaf), the left one is parsed first and gets the smaller
//!   e-class id, so egg's min-term extractor keeps each `+` in its written
//!   orientation — it does *not* re-align them. With a balanced split no single
//!   orientation wins, so a search over the minimal corpus falls back to the
//!   weaker `(* ?y ?y)` squaring. Live rewriting re-aligns every `+` and uses
//!   `add_zero` on the bare square, recovering the full `f0` across all programs.
//!
//! Measured (best-first, `--max-arity 2`, seven programs):
//! | corpus | rule-free         | at-start | live             |
//! |--------|-------------------|----------|------------------|
//! | A      | ~1.19x (full f0)  | —        | —                |
//! | B      | ~1.16x (squaring) | ~1.16x   | ~1.26x (full f0) |
//!
//! Live both re-aligns the scrambled `+`s and `add_zero`-expands the bare square,
//! so it finds the full `(+ ?x (* ?y ?y))`; the minimal corpus only yields bare
//! `(* ?y ?y)`.

use egg::Language;
use egg_stitch::{
    io,
    lang::{Op, OpChildren, StitchLanguage, Weights},
};
use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const DIR: &str = "data/domains/examples-paper";

/// Commutativity terminates as an e-graph saturation (each `+` class gains its
/// swapped node and no more), so a small iter cap is plenty.
const ITER_LIMIT: &str = "6";

/// Reads a corpus JSON file (an array of s-expression strings).
fn load_corpus(name: &str) -> Vec<String> {
    let text = fs::read_to_string(format!("{DIR}/{name}")).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

/// Writes a commutativity-only ruleset to a temp file and returns its path.
/// The equivalence and minimal-term checks only need commutativity: `add_zero`
/// (`?x => (+ 0 ?x)`) only ever *grows* terms, so it changes neither the e-class
/// merges nor the minimal size — and saturating it with comm blows the e-graph
/// up. Comm alone saturates cheaply.
fn comm_rules_path() -> String {
    let p = std::env::temp_dir().join(format!("egg-stitch-examples-paper-comm-{}.rewrites", std::process::id()));
    fs::write(&p, "plus_comm: (+ ?x ?y) <=> (+ ?y ?x)\n").unwrap_or_else(|e| panic!("write {}: {e}", p.display()));
    p.to_str().expect("utf-8 temp path").to_string()
}

/// Weighted size (= node count under unit weights) of an s-expression string.
fn tree_size(sexpr: &str) -> usize {
    sexpr.replace('(', " ").replace(')', " ").split_whitespace().count()
}

/// What rewrite regime a compression run uses.
#[derive(Clone, Copy)]
enum Mode {
    /// No rewrites — pure syntactic library learning.
    RuleFree,
    /// Rules applied live during search.
    Live,
    /// Rules used only to extract the minimal term up front, then dropped.
    AtStart,
}

/// Result of one best-first run: its compression ratio and the abstraction it
/// found. `found_f0` is true when the abstraction is the full `(+ ?x (* ?y ?y))`
/// (it contains a `+`); the consolation `(* ?y ?y)` squaring abstraction does
/// not.
struct Run {
    ratio: f64,
    found_f0: bool,
}

/// Runs best-first on a corpus under one rewrite regime.
fn run(corpus: &str, mode: Mode) -> Run {
    let input = format!("{DIR}/{corpus}");
    let out = std::env::temp_dir().join(format!("egg-stitch-examples-paper-{}-{}-{}.json", std::process::id(), corpus, mode as u8));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", &input, "--max-arity", "2", "--num-steps", "200000", "--output", out_str]);
    if !matches!(mode, Mode::RuleFree) {
        cmd.args(["--rules", &format!("{DIR}/rules.rewrites"), "--iter-limit", ITER_LIMIT]);
    }
    if matches!(mode, Mode::AtStart) {
        cmd.arg("--only-use-dsrs-at-start");
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "best-first run failed for {input}");
    let text = fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = fs::remove_file(&out);
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", out.display()));
    let ratio = v["compression_ratio"].as_f64().expect("compression_ratio present");
    // The full f0 contains a `+`; the bare-squaring fallback does not.
    let found_f0 = v["library"].get(0).and_then(|e| e["pattern"].as_str()).is_some_and(|p| p.contains('+'));
    Run { ratio, found_f0 }
}

#[test]
fn a_and_b_are_equivalent_under_commutativity() {
    let a = load_corpus("corpus_a.json");
    let b = load_corpus("corpus_b.json");
    assert_eq!(a.len(), b.len(), "corpora must align program-for-program");
    let rules = comm_rules_path();
    for (ai, bi) in a.iter().zip(&b) {
        let data = io::egraph_from_programs::<OpChildren, Op>(&[ai.clone(), bi.clone()], Some(&rules), Weights::default(), 10, 1_000_000);
        let eg = &data.egraph;
        let programs = eg[data.root].nodes.iter().find(|n| n.is_programs_node()).expect("root has a programs node");
        let (ca, cb) = (eg.find(programs.children()[0]), eg.find(programs.children()[1]));
        assert_eq!(ca, cb, "A and B programs should be equivalent under commutativity:\n  A = {ai}\n  B = {bi}");
    }
}

#[test]
fn b_is_size_minimal() {
    // Under commutativity (size-neutral) B has no reducible structure: its
    // min-term extraction is the same size as B itself.
    let b = load_corpus("corpus_b.json");
    let rules = comm_rules_path();
    let data = io::egraph_from_programs::<OpChildren, Op>(&b, Some(&rules), Weights::default(), 10, 1_000_000);
    for (bi, mi) in b.iter().zip(io::extract_programs(&data.egraph, data.root)) {
        assert_eq!(tree_size(&mi), tree_size(bi), "B should be size-minimal, but extraction changed its size: {bi} -> {mi}");
    }
}

#[test]
fn syntactic_search_compresses_a_but_not_b() {
    let a = run("corpus_a.json", Mode::RuleFree);
    let b = run("corpus_b.json", Mode::RuleFree);
    assert!(a.found_f0, "rule-free search should find the full f0 on aligned A");
    assert!(!b.found_f0, "rule-free search should only manage bare squaring on scrambled B");
    assert!(a.ratio > b.ratio, "A should compress better than B syntactically (A={}, B={})", a.ratio, b.ratio);
}

/// The headline: live commutativity re-aligns B's `+` nodes and recovers the
/// full `f0`, but `--only-use-dsrs-at-start` searches the minimal (scrambled)
/// corpus and is stuck with bare squaring — the compressive ordering does not
/// fall out of canonicalization.
#[test]
fn live_beats_only_use_dsrs_at_start_on_b() {
    let live = run("corpus_b.json", Mode::Live);
    let at_start = run("corpus_b.json", Mode::AtStart);
    assert!(live.found_f0, "live rewriting should recover the full f0 on B");
    assert!(!at_start.found_f0, "at-start should be stuck with bare squaring on the scrambled minimal term");
    assert!(live.ratio > at_start.ratio, "live should beat at-start (live={}, at-start={})", live.ratio, at_start.ratio);
}
