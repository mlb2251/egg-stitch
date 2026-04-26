//! Tests ported from `../Stitch.jl/tests/` and `../stitch/tests/`
//! (the `data/basic/` folders in both repos).
//!
//! Each test runs both best-first (10 000 expansions) and SMC (1 000 particles
//! × 1 000 steps, temperature 1 000) with `--check-slow` enabled and compares
//! against a frozen fixture living next to the input JSON (`foo.json` →
//! `foo.out.json`). The fixture records the single found abstraction (or no
//! abstractions), its match count, and the rewritten corpus. Both backends
//! must agree, and both must match the fixture.
//!
//! Only the smallest, most deterministic cases are included — egg-stitch's SMC
//! is stochastic, so we pick corpora where both backends reliably converge.
//! Most of the remaining `basic/` cases use DeBruijn indices (`$0`, `$1`, ...)
//! or symbol variables (`%1`, `&x:0`). egg-stitch parses these as plain symbol
//! tokens with no scope awareness, so any abstraction it finds over them isn't
//! semantically valid under Stitch.jl's lambda-calculus reading — we skip all
//! such fixtures. A handful (`simple_hof`, `safe_ctx_thread_bug`) also have a
//! list in operator position, which egg's `RecExpr` parser rejects outright.
//! `minimum-matches-seq` depends on Stitch.jl's special `/seq` matching.
//!
//! From `../stitch/data/basic/` we additionally port `simple3`; `simple4`,
//! `simple5`, and `lio_test*` use DeBruijn vars, `symbol_weighting_test_*`
//! depends on per-symbol cost weights, and the rest duplicate Stitch.jl
//! fixtures we already cover.
//!
//! To regenerate all fixtures after a legitimate behavior change, run with
//! `BLESS=1`:
//!
//! ```text
//! BLESS=1 cargo test --release --test stitch_compat_test -- --test-threads=1
//! ```

use clap::Parser;
use egg_stitch::{
    Args, io,
    lang::{Op, OpChildren},
    multiple_step_search,
    results::AbstractionResult,
};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
struct Expected {
    abstractions: Vec<ExpectedAbstraction>,
    rewritten: Vec<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
struct ExpectedAbstraction {
    body: String,
    num_matches: usize,
}

/// egg prints metavars as `?#0`; store the cleaner `#0` form in fixtures.
fn egg_to_stitch(s: &str) -> String {
    s.replace("?#", "#")
}

fn input_programs(input: &str) -> Vec<String> {
    let text = fs::read_to_string(input).unwrap_or_else(|e| panic!("read {input}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {input}: {e}"))
}

fn expected_path(input: &str) -> String {
    // Mirror the `data/domains/<...>/foo.json` layout under `data/expected_outputs/`.
    let relative = input.strip_prefix("data/domains/").expect("expected input under data/domains/");
    let stem = relative.strip_suffix(".json").unwrap_or(relative);
    format!("data/expected_outputs/{stem}.out.json")
}

fn build_expected(library: Vec<AbstractionResult>, input: &str) -> Expected {
    let abstractions: Vec<ExpectedAbstraction> = library
        .iter()
        .map(|r| ExpectedAbstraction {
            body: egg_to_stitch(r.pattern.split_once(": ").expect("pattern prefixed with fn_N:").1),
            num_matches: r.num_matches,
        })
        .collect();
    let rewritten = library.last().map(|r| r.rewritten_programs.clone()).unwrap_or_else(|| input_programs(input));
    Expected { abstractions, rewritten }
}

/// Runs the backend with the given search, input, and extra CLI args.
fn run_backend(search: &str, input: &str, extra_args: &[&str]) -> Expected {
    let mut argv: Vec<&str> = vec!["egg-stitch", "--search", search, "--input", input, "--check-slow", "--num-abstractions", "1"];
    if search == "best-first" {
        argv.extend(["--num-steps", "10000"]);
    } else {
        argv.extend(["--num-particles", "1000", "--num-steps", "1000", "--temperature", "1000"]);
    }
    argv.extend(extra_args);
    let args = Args::parse_from(argv);
    let (egraph, root, _) = io::load_egraph(&args.input, args.rules.as_deref());
    let (library, _, _) = multiple_step_search::<OpChildren, Op>(egraph, root, &args);
    build_expected(library, input)
}

/// Checks the fixture for the given input and extra CLI args (e.g., ["-r", path]).
fn check_fixture(input: &str, extra_args: &[&str]) {
    let bf = run_backend("best-first", input, extra_args);
    let smc = run_backend("smc", input, extra_args);

    // Best-first is deterministic and is the canonical fixture. SMC is stochastic
    // and — once rewrite rules introduce non-trivial e-class equivalences like
    // commutativity — can pick a different representative of the same semantic
    // abstraction (e.g. `(+ a (+ b #0))` vs `(+ #0 (+ a b))`). We still require
    // the number of matches and the rewritten corpus to match, since those are
    // determined by the equivalence class, not the chosen representative.
    let num_matches_of = |e: &Expected| e.abstractions.iter().map(|a| a.num_matches).collect::<Vec<_>>();
    assert_eq!(num_matches_of(&bf), num_matches_of(&smc), "best-first and smc disagree on num_matches for {input}");
    assert_eq!(bf.rewritten, smc.rewritten, "best-first and smc disagree on rewritten corpus for {input}");

    let path = expected_path(input);
    if std::env::var("BLESS").is_ok() {
        let mut text = serde_json::to_string_pretty(&bf).expect("serialize expected");
        text.push('\n');
        fs::write(&path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    } else {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e} (run with BLESS=1 to create)"));
        let expected: Expected = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
        assert_eq!(bf, expected, "fixture mismatch for {input} (run with BLESS=1 to update)");
    }
}

#[test]
fn identical() {
    check_fixture("data/domains/stitch/identical.json", &[]);
}

/// Diverges from Stitch.jl: Stitch.jl finds the arity-0 body
/// `(a b c d e f g h (A B C) (A B C) (A B C) (A B C))`; egg-stitch's e-class
/// equality unifies the four `(A B C)` subterms and picks the arity-1
/// `(a b c d e f g h #0 #0 #0 #0)` instead.
#[test]
fn cex() {
    check_fixture("data/domains/stitch/cex.json", &[]);
}

#[test]
fn minimum_matches() {
    check_fixture("data/domains/stitch/minimum-matches.json", &[]);
}

#[test]
fn simple1() {
    check_fixture("data/domains/stitch/simple1.json", &[]);
}

#[test]
fn simple2() {
    check_fixture("data/domains/stitch/simple2.json", &[]);
}

/// From `../stitch/data/basic/`. Rust stitch finds `(#0 (lam_1 (#0 #0)))` under
/// its per-primitive cost weights; under egg-stitch's unit-cost AST model the
/// compression doesn't pay, so no abstraction is returned.
#[test]
fn simple3() {
    check_fixture("data/domains/stitch/simple3.json", &[]);
}

#[test]
fn tmp_minimal() {
    check_fixture("data/domains/stitch/tmp_minimal.json", &[]);
}

/// Exercises `--rules`: with the bidirectional `(+ 0 ?x) <=> ?x` in play,
/// the `(+ _ (* _ _))` shape aligns across all five programs (the fifth,
/// `(* 7 (* (- v) (- v)))`, gets a `(+ 0 _)`-wrapped representation in its
/// e-class so the inner `(* (- v) (- v))` becomes a match too).
#[test]
fn nested() {
    check_fixture("data/domains/stitch/nested.json", &["-r", "data/domains/stitch/nested.rewrites"]);
}

const ARITH_RULES: &str = "data/domains/simple-arithmetic/arithmetic.rewrites";

#[test]
fn arithmetic_aplusbplusc() {
    check_fixture("data/domains/simple-arithmetic/aplusbplusc.json", &["-r", ARITH_RULES]);
}

#[test]
fn arithmetic_aplusbplus1234() {
    check_fixture("data/domains/simple-arithmetic/aplusbplus1234.json", &["-r", ARITH_RULES]);
}
