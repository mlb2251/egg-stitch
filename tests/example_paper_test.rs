//! Worked example for the paper: commutative ordering that does *not* fall out
//! of canonicalization, so live rewriting beats `--only-use-dsrs-at-start`.
//!
//! The only rewrite is commutativity of `+`
//! (`data/domains/examples-paper/rules.rewrites`):
//! ```text
//! plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
//! ```
//!
//! Each program (after a shared `preamble` program at index 0) is a 3-level sum
//! of *blocks*; block `k` pairs a shared anchor `k{1,2,3}` with a per-program
//! value.
//!
//! * `corpus_a.json` writes every block anchor-first, so all programs are
//!   instances of one skeleton `(+ (+ k1 ?) (+ (+ k2 ?) (+ k3 ?)))`. A plain
//!   *syntactic* (rule-free) search finds it.
//! * `corpus_b.json` is the size-minimal canonical form: each block is in the
//!   orientation egg's min-term extractor would pick (smaller child e-class id
//!   first). The corpus is built so that — by introducing some values *before*
//!   the anchors (smaller ids) and some *after* (larger ids) — that canonical
//!   orientation is *inconsistent* across programs. So the shared skeleton is
//!   invisible to a syntactic search, and crucially extracting the min-term does
//!   not re-align it.
//!
//! The payoff is that the *minimal term of A is B* (`extract_programs(A) == B`),
//! and `B` is its own minimal term. Hence `--only-use-dsrs-at-start`, which
//! searches the extracted minimal corpus, is stuck with the scrambled B and
//! compresses poorly, while live commutativity re-aligns the blocks and recovers
//! A's compression.
//!
//! Measured (best-first, `--max-arity 3`):
//! | corpus | rule-free | at-start | live |
//! |--------|-----------|----------|------|
//! | A      | ~1.67x    | ~1.11x   | ~1.67x |
//! | B      | ~1.11x    | ~1.11x   | ~1.67x |

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

/// Runs best-first on a corpus under one rewrite regime and returns its
/// `compression_ratio` (corpus size before / after one abstraction).
fn compression_ratio(corpus: &str, mode: Mode) -> f64 {
    let input = format!("{DIR}/{corpus}");
    let out = std::env::temp_dir().join(format!("egg-stitch-examples-paper-{}-{}-{}.json", std::process::id(), corpus, mode as u8));
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", "best-first", "--input", &input, "--max-arity", "3", "--num-steps", "200000", "--output", out_str]);
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
    v["compression_ratio"].as_f64().expect("compression_ratio present")
}

/// Saturates `programs` under commutativity and returns each program's
/// size-minimal extraction.
fn minimal_terms(programs: &[String]) -> Vec<String> {
    let rules = format!("{DIR}/rules.rewrites");
    let data = io::egraph_from_programs::<OpChildren, Op>(programs, Some(&rules), Weights::default(), 15, 1_000_000);
    io::extract_programs(&data.egraph, data.root)
}

#[test]
fn a_and_b_are_equivalent_under_commutativity() {
    let a = load_corpus("corpus_a.json");
    let b = load_corpus("corpus_b.json");
    assert_eq!(a.len(), b.len(), "corpora must align program-for-program");
    let rules = format!("{DIR}/rules.rewrites");
    for (ai, bi) in a.iter().zip(&b) {
        let data = io::egraph_from_programs::<OpChildren, Op>(&[ai.clone(), bi.clone()], Some(&rules), Weights::default(), 15, 1_000_000);
        let eg = &data.egraph;
        let programs = eg[data.root].nodes.iter().find(|n| n.is_programs_node()).expect("root has a programs node");
        let (ca, cb) = (eg.find(programs.children()[0]), eg.find(programs.children()[1]));
        assert_eq!(ca, cb, "A and B programs should be equivalent under commutativity:\n  A = {ai}\n  B = {bi}");
    }
}

#[test]
fn b_is_the_minimal_term_and_does_not_unscramble() {
    let a = load_corpus("corpus_a.json");
    let b = load_corpus("corpus_b.json");
    // The minimal term of the aligned corpus A is exactly the scrambled B...
    assert_eq!(minimal_terms(&a), b, "min-term(A) should equal B");
    // ...and B is already its own minimal term (extraction leaves it scrambled).
    assert_eq!(minimal_terms(&b), b, "B should be its own minimal term (extraction must not unscramble it)");
}

#[test]
fn syntactic_search_compresses_a_but_not_b() {
    let a = compression_ratio("corpus_a.json", Mode::RuleFree);
    let b = compression_ratio("corpus_b.json", Mode::RuleFree);
    assert!(a >= 1.5, "rule-free search should compress aligned A well (got {a})");
    assert!(b <= 1.3, "rule-free search should barely compress scrambled B (got {b})");
}

/// The headline: live commutativity re-aligns the blocks and compresses, but
/// `--only-use-dsrs-at-start` searches the minimal (scrambled) corpus and can't.
/// Holds for both corpora, since A's minimal term is B.
#[test]
fn live_beats_only_use_dsrs_at_start() {
    for corpus in ["corpus_a.json", "corpus_b.json"] {
        let live = compression_ratio(corpus, Mode::Live);
        let at_start = compression_ratio(corpus, Mode::AtStart);
        assert!(live >= 1.5, "{corpus}: live rewriting should compress well (got {live})");
        assert!(at_start <= 1.3, "{corpus}: at-start should be stuck on the scrambled minimal term (got {at_start})");
        assert!(live >= 1.4 * at_start, "{corpus}: live should clearly beat at-start (live={live}, at-start={at_start})");
    }
}
