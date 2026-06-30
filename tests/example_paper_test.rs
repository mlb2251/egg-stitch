//! Worked example for the paper: a commutative ordering that does *not* fall out
//! of canonicalization, so live rewriting beats `--only-use-dsrs-at-start`.
//!
//! The only rewrite is commutativity of `+`
//! (`data/domains/examples-paper/rules.rewrites`):
//! ```text
//! plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
//! ```
//!
//! Each program is a 3-level sum of *blocks*; block `k` pairs a shared anchor
//! `k{1,2,3}` with a per-program value.
//!
//! * `corpus_a.json` writes every block anchor-first, so all programs are
//!   instances of one skeleton `(+ (+ k1 ?) (+ (+ k2 ?) (+ k3 ?)))` and a plain
//!   *syntactic* (rule-free) search finds it.
//! * `corpus_b.json` is a size-minimal canonical form whose block orientations
//!   are deliberately *inconsistent* across programs. egg's min-term extractor
//!   breaks the `(+ P Q)` vs `(+ Q P)` tie by smaller child e-class id first, and
//!   ids follow parse order, so a block `(+ k v)` extracts value-first iff `v`
//!   was introduced before the anchor `k`. The first program is the
//!   all-value-first one, so it introduces the shared values `e1,e2,e3` *before*
//!   the anchors; later programs reuse `e_j` for a value-first block or a fresh
//!   "late" value for an anchor-first block. The eight programs realize the eight
//!   distinct orientation patterns, so no consistent skeleton survives — and
//!   crucially extracting the minimal term does not re-align them.
//!
//! Hence `--only-use-dsrs-at-start`, which abstracts over the extracted minimal
//! corpus, is stuck with the scrambled B, while live commutativity re-aligns the
//! blocks and recovers A's compression.
//!
//! Measured (best-first, `--max-arity 3`):
//! | corpus | rule-free | at-start | live |
//! |--------|-----------|----------|------|
//! | A      | ~2.0x     | ~1.4x    | ~2.0x |
//! | B      | ~1.1x     | ~1.1x    | ~2.0x |

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
fn b_is_size_minimal() {
    // Under commutativity (size-neutral) B has no reducible structure: its
    // min-term extraction is the same size as B itself.
    let b = load_corpus("corpus_b.json");
    let rules = format!("{DIR}/rules.rewrites");
    let data = io::egraph_from_programs::<OpChildren, Op>(&b, Some(&rules), Weights::default(), 15, 1_000_000);
    for (bi, mi) in b.iter().zip(io::extract_programs(&data.egraph, data.root)) {
        assert_eq!(tree_size(&mi), tree_size(bi), "B should be size-minimal, but extraction changed its size: {bi} -> {mi}");
    }
}

#[test]
fn syntactic_search_compresses_a_but_not_b() {
    let a = compression_ratio("corpus_a.json", Mode::RuleFree);
    let b = compression_ratio("corpus_b.json", Mode::RuleFree);
    assert!(a >= 1.8, "rule-free search should compress aligned A well (got {a})");
    assert!(b <= 1.3, "rule-free search should barely compress scrambled B (got {b})");
}

/// The headline: live commutativity re-aligns B's blocks and recovers A's
/// compression, but `--only-use-dsrs-at-start` searches the minimal (scrambled)
/// corpus and can't — the compressive ordering does not fall out of
/// canonicalization.
#[test]
fn live_beats_only_use_dsrs_at_start_on_b() {
    let live = compression_ratio("corpus_b.json", Mode::Live);
    let at_start = compression_ratio("corpus_b.json", Mode::AtStart);
    assert!(live >= 1.8, "live rewriting should recover B's compression (got {live})");
    assert!(at_start <= 1.3, "at-start should be stuck on the scrambled minimal term (got {at_start})");
    assert!(live >= 1.5 * at_start, "live should clearly beat at-start (live={live}, at-start={at_start})");
}
