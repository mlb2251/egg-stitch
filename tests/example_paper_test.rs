//! See data/domains/examples-paper/README.md for context.

use egg::Language;
use egg_stitch::{
    io,
    lang::{Op, OpChildren, StitchLanguage, Weights},
};
use serde_json::Value;
use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const DIR: &str = "data/domains/examples-paper";

/// `--iter-limit` for rule runs. `add_zero`/`neg_zero` are size-increasing in one
/// direction and never saturate, so this is a cap; a handful of iterations is
/// enough to merge the orientations and expand the bare square, and small keeps
/// the e-graph from ballooning.
const ITER_LIMIT: &str = "4";

/// Reads a corpus JSON file (an array of s-expression strings).
fn load_corpus(name: &str) -> Vec<String> {
    let text = fs::read_to_string(format!("{DIR}/{name}")).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

/// Weighted size (= node count under unit weights) of an s-expression string.
fn tree_size(sexpr: &str) -> usize {
    sexpr.replace(['(', ')'], " ").split_whitespace().count()
}

/// Total weighted size of a corpus.
fn total_size(programs: &[String]) -> usize {
    programs.iter().map(|p| tree_size(p)).sum()
}

/// Saturates `programs` under the real ruleset (bounded iter-limit) and returns
/// each program's size-minimal extraction.
fn minimal_terms(programs: &[String]) -> Vec<String> {
    let rules = format!("{DIR}/rules.rewrites");
    let data = io::egraph_from_programs::<OpChildren, Op>(programs, Some(&rules), Weights::default(), 4, 200_000);
    io::extract_programs(&data.egraph, data.root)
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
/// found. `found_f0` is true when the abstraction is the full
/// `(+ (- ?0) (* ?1 ?1))` (it contains a `+`); the consolation `(* ?1 ?1)`
/// squaring abstraction does not.
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
fn a_and_b_are_equivalent_under_the_rewrites() {
    let a = load_corpus("corpus_a.json");
    let b = load_corpus("corpus_b.json");
    assert_eq!(a.len(), b.len(), "corpora must align program-for-program");
    let rules = format!("{DIR}/rules.rewrites");
    for (ai, bi) in a.iter().zip(&b) {
        let data = io::egraph_from_programs::<OpChildren, Op>(&[ai.clone(), bi.clone()], Some(&rules), Weights::default(), 4, 200_000);
        let eg = &data.egraph;
        let programs = eg[data.root].nodes.iter().find(|n| n.is_programs_node()).expect("root has a programs node");
        let (ca, cb) = (eg.find(programs.children()[0]), eg.find(programs.children()[1]));
        assert_eq!(ca, cb, "A and B programs should be equivalent under the rewrites:\n  A = {ai}\n  B = {bi}");
    }
}

#[test]
fn b_is_minimal_but_a_is_expanded() {
    // B is already size-minimal, so saturating + extracting shrinks nothing.
    // A is the deliberately *expanded* corpus: its last program carries a
    // removable `(+ (- 0) ..)`, so its minimal term is strictly smaller.
    let a = load_corpus("corpus_a.json");
    let b = load_corpus("corpus_b.json");
    assert_eq!(total_size(&minimal_terms(&b)), total_size(&b), "B should already be size-minimal");
    assert!(total_size(&minimal_terms(&a)) < total_size(&a), "A should be non-minimal — its (+ (- 0) ..) strips away");
}

#[test]
fn syntactic_search_compresses_a_but_not_b() {
    let a = run("corpus_a.json", Mode::RuleFree);
    let b = run("corpus_b.json", Mode::RuleFree);
    assert!(a.found_f0, "rule-free search should find the full f0 on aligned A");
    assert!(!b.found_f0, "rule-free search should only manage bare squaring on scrambled B");
    assert!(a.ratio > b.ratio, "A should compress better than B syntactically (A={}, B={})", a.ratio, b.ratio);
}

/// The headline: live rewriting re-aligns B's `+` nodes and expands the bare
/// square to recover the full `f0`, but `--only-use-dsrs-at-start` searches the
/// minimal corpus and is stuck with bare squaring — the abstraction-exposing
/// form does not fall out of canonicalization.
#[test]
fn live_beats_only_use_dsrs_at_start_on_b() {
    let live = run("corpus_b.json", Mode::Live);
    let at_start = run("corpus_b.json", Mode::AtStart);
    assert!(live.found_f0, "live rewriting should recover the full f0 on B");
    assert!(!at_start.found_f0, "at-start should be stuck with bare squaring on the scrambled minimal term");
    assert!(live.ratio > at_start.ratio, "live should beat at-start (live={}, at-start={})", live.ratio, at_start.ratio);
}
