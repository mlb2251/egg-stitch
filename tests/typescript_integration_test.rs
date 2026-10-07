//! End-to-end tests for `--language typescript`: de Bruijn handling across
//! multi-slot binders (`lam{n}`, `define`) and higher-order capture. Corpora
//! live in `data/test/ts/`. The outputs are checked two ways:
//!
//! 1. **Scope invariants**, computed by the tiny s-expression walker at the
//!    bottom of this file. The invariants are:
//!    - Every library `lambda` must be closed.
//!    - Every rewritten program must have the same free variables as its original.
//!    - Every call site must be one flat `(app fn_k …)` carrying exactly `arity_k`
//!      arguments.
//! 2. Asserts on expected outputs for small corpora.

use std::collections::BTreeSet;

use serde_json::Value;

mod common;

const DIR: &str = "data/test/ts";

// ─── running ────────────────────────────────────────────────────────────────

/// One `--language typescript` run under the stitch convention
/// (`--check-slow`, so any fast/slow cost mismatch panics the binary).
fn run_ts(search: &str, corpus: &str, extra: &[&str]) -> Value {
    run_ts_n(search, corpus, "1", extra)
}

/// Like [`run_ts`] but with `--num-abstractions n`, for stacked libraries.
fn run_ts_n(search: &str, corpus: &str, n: &str, extra: &[&str]) -> Value {
    let input = format!("{DIR}/{corpus}.json");
    let mut args = vec!["--language", "typescript"];
    args.extend_from_slice(extra);
    common::run_with_abstractions(search, &input, n, &args)
}

// ─── accessors ──────────────────────────────────────────────────────────────

/// The run's `library` array (empty when no abstraction was found).
fn library(run: &Value) -> Vec<Value> {
    run.get("library").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// A string-array field such as `original_programs` / `rewritten_programs`.
fn programs(run: &Value, key: &str) -> Vec<String> {
    run[key].as_array().unwrap_or_else(|| panic!("missing {key}")).iter().map(|p| p.as_str().expect("program string").to_string()).collect()
}

/// The `lambda` field of library entry `i`.
fn lambda(run: &Value, i: usize) -> String {
    library(run)[i]["lambda"].as_str().expect("lambda string").to_string()
}

/// The `arity` field of library entry `i`.
fn arity(run: &Value, i: usize) -> usize {
    library(run)[i]["arity"].as_u64().expect("arity") as usize
}

/// Runs one corpus under both backends and checks every invariant on each.
/// Returns `(best_first, smc)`.
fn run_both_checked(corpus: &str) -> (Value, Value) {
    let bf = run_ts("best-first", corpus, &[]);
    let smc = run_ts("smc", corpus, &[]);
    assert_all_invariants(&bf, &format!("{corpus}[best-first]"));
    assert_all_invariants(&smc, &format!("{corpus}[smc]"));
    (bf, smc)
}

// ─── invariants ─────────────────────────────────────────────────────────────

/// Everything that must hold for any abstraction the search picks.
fn assert_all_invariants(run: &Value, label: &str) {
    assert!(!library(run).is_empty(), "{label}: no abstraction found; every corpus here is built to compress, and an empty library would make the oracle pass vacuously");
    assert_library_closed(run, label);
    assert_scope_preserved(run, label);
    assert_call_sites_flat(run, label);
    assert_binder_groups(run, label);
}

/// Every library `lambda` is a closed term: all `?#k` became bound DB indices
/// and no free variable from the matched context leaked into a body.
fn assert_library_closed(run: &Value, label: &str) {
    for (i, e) in library(run).iter().enumerate() {
        let l = e["lambda"].as_str().expect("lambda");
        let fv = free_vars(&parse(l));
        assert!(fv.is_empty(), "{label}: fn_{i} lambda has free vars {fv:?}: {l}");
    }
}

/// Rewriting never captures or frees a variable: each rewritten program has
/// exactly the free variables of its original.
fn assert_scope_preserved(run: &Value, label: &str) {
    let orig = programs(run, "original_programs");
    let rewr = programs(run, "rewritten_programs");
    assert_eq!(orig.len(), rewr.len(), "{label}: program count changed");
    for (i, (o, r)) in orig.iter().zip(&rewr).enumerate() {
        assert_eq!(free_vars(&parse(o)), free_vars(&parse(r)), "{label}: program {i} changed its free variables\n  original : {o}\n  rewritten: {r}");
    }
}

/// Every use of `fn_k` is the callee of one flat `app` with exactly `arity_k` arguments.
fn assert_call_sites_flat(run: &Value, label: &str) {
    let arities: Vec<usize> = (0..library(run).len()).map(|i| arity(run, i)).collect();
    let mut terms = programs(run, "rewritten_programs");
    terms.extend((0..arities.len()).map(|i| lambda(run, i)));
    for t in &terms {
        let mut uses = Vec::new();
        fn_uses(&parse(t), &mut uses);
        for u in uses {
            match u {
                FnUse::Call(k, argc) => assert_eq!(argc, arities[k], "{label}: fn_{k} called with {argc} args, arity is {}: {t}", arities[k]),
                FnUse::Misplaced(k) => panic!("{label}: fn_{k} appears outside a flat `(app fn_{k} …)`: {t}"),
            }
        }
    }
}

/// Every library lambda with `arity > 0` opens with exactly one `lam{arity}`,
/// and neither the library nor the rewritten programs contain a `lam0` the
/// corpus didn't have.
fn assert_binder_groups(run: &Value, label: &str) {
    let orig = programs(run, "original_programs");
    let corpus_has_lam0 = orig.iter().any(|p| p.contains("(lam0 "));
    for (i, (o, r)) in orig.iter().zip(programs(run, "rewritten_programs")).enumerate() {
        let (no, nr) = (o.matches("(lam0 ").count(), r.matches("(lam0 ").count());
        assert!(nr <= no, "{label}: rewritten program {i} has {nr} lam0s, its original only {no}\n  original : {o}\n  rewritten: {r}");
    }
    for i in 0..library(run).len() {
        let l = lambda(run, i);
        assert!(!l.starts_with("(lam0 "), "{label}: fn_{i} is wrapped in a lam0: {l}");
        assert!(corpus_has_lam0 || !l.contains("lam0"), "{label}: fn_{i} contains a lam0 the corpus never had: {l}");
        let a = arity(run, i);
        if a > 0 {
            assert!(l.starts_with(&format!("(lam{a} ")), "{label}: fn_{i} (arity {a}) should open with one lam{a}: {l}");
        }
    }
}

// ─── A. de Bruijn indices across multi-slot binders ─────────────────────────

/// A metavar that lands *inside* a `lam2` sits two binders deep, so its head
/// index is `$0 + 2 = $2`.
#[test]
fn metavar_under_lam2_is_shifted_by_two() {
    let (bf, _) = run_both_checked("lam2_metavar_depth");
    assert_eq!(lambda(&bf, 0), "(lam1 (app reduce xs z (lam2 (app max (app $2 $1) $0))))");
}

/// Metamorphic sibling of the test above: prepend unused parameters to every
/// corpus lambda (`lam2` → `lam3`, `lam10`, `lam100`). Tests the following:
/// 1. Under the rightmost convention the body's own `$1`/`$0` don't move,
///    only the abstraction's slot does (`$2` → `$n`).
/// 2. `lams_cost` is flat in n, so the compression must be identical.
#[test]
fn widening_a_binder_group_changes_only_the_slot_index() {
    let (l2, _) = run_both_checked("lam2_metavar_depth");
    for n in [3, 10, 100] {
        let (ln, _) = run_both_checked(&format!("lam{n}_metavar_depth"));
        assert_eq!(lambda(&ln, 0), format!("(lam1 (app reduce xs z (lam{n} (app max (app ${n} $1) $0))))"));
        assert_eq!(l2["final_cost"], ln["final_cost"], "a lam{n} must cost what a lam2 costs");
        assert_eq!(library(&l2)[0]["num_matches"], library(&ln)[0]["num_matches"]);
    }
}

/// Variables bound inside the abstracted window stay in the body, while those
/// bound outside it are abstracted exactly like symbols, including at the
/// boundary (the largest bound index vs. the smallest free one).
#[test]
fn barely_free_var_is_passed_like_a_symbol() {
    let (bound, _) = run_both_checked("free_var_barely_bound");
    assert_eq!(lambda(&bound, 0), "(lam1 (app reduce xs z (lam2 (app max (app $2 $1) $0))))");

    let (free, _) = run_both_checked("free_var_barely_free");
    let (sym, _) = run_both_checked("free_var_as_symbol");
    assert_eq!(lambda(&free, 0), "(lam2 (app reduce xs z (lam2 (app max (app $3 $2) $0))))");
    assert_eq!(lambda(&free, 0), lambda(&sym, 0), "a free var must abstract exactly like a symbol");
    assert_eq!(free["final_cost"], sym["final_cost"]);
    assert_eq!(library(&free)[0]["num_matches"], library(&sym)[0]["num_matches"]);
    for (i, (f, s)) in programs(&free, "rewritten_programs").iter().zip(programs(&sym, "rewritten_programs")).enumerate() {
        assert_eq!(*f, s.replace(&format!("y{}", i + 1), "$0"), "program {i}: the free var should sit where the symbol does");
    }
}

/// `() => x` extracts as `(lam0 x)`. The thunk's `lam0` is ordinary corpus
/// structure: it must survive into the body verbatim. The oracle can't check
/// this one (it desugars `(lam0 X)` to `X`, so dropping the `lam0` still
/// β-checks), which is why the string is pinned.
#[test]
fn thunk_lam0_is_kept_as_structure() {
    let (bf, _) = run_both_checked("lam0_thunk");
    assert_eq!(lambda(&bf, 0), "(lam1 (app setTimeout (lam0 (app $0 x)) ms))");
}

/// A de Bruijn variable within `define`'s first child isn't shifted by the binder
/// `define` introduces.
#[test]
fn define_value_slot_is_outside_the_let() {
    let (bf, _) = run_both_checked("define_value_slot");
    assert_eq!(lambda(&bf, 0), "(lam1 (define (app load $0) (app use $0 k)))");
}

/// A de Bruijn variable within `define`'s second child is shifted by the binder
/// `define` introduces.
#[test]
fn define_body_slot_is_inside_the_let() {
    let (bf, _) = run_both_checked("define_body_slot");
    assert_eq!(lambda(&bf, 0), "(lam1 (define (app load a) (app use $0 $1)))");
}

/// Each program reuses a variable bound *outside* the repeated window
/// (`(app bar $0 _)` under the program's own `lam1`). A body that baked in that
/// `$0` would be an open term; the closedness invariant rejects it, so the `$0`
/// must travel as an argument instead.
///
/// It is also the backward-ordering check at arity 2: slot 0 (the *first*
/// call-site argument, here the context var) is the *highest* index, `$1`.
/// The oracle alone can't see the convention: flipping both the index names
/// and the call-site argument order still β-checks, so the pair is pinned.
#[test]
fn free_context_var_never_leaks_into_a_body() {
    let (bf, smc) = run_both_checked("free_var_context");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam2 (app bar $1 (app cfg q r s) $0))", "{name}");
        assert_eq!(programs(run, "rewritten_programs")[1], "(lam1 (app baz (app fn_0 $0 d) (app fn_0 $0 e)))", "{name}: first argument fills the highest index");
    }
}

/// Flat version of lambda-calc `ho_arity2_capture`. Without currying, a flat
/// `app` only matches calls with the same number of arguments, so the slot is
/// filled only by closed symbols and stays first-order (no η-wrap), unlike the
/// higher-order slot in the lambda-calc version.
#[test]
fn flat_arity_mismatch_stays_first_order() {
    let run = run_ts("best-first", "flat_arity_mismatch", &[]);
    assert_all_invariants(&run, "flat_arity_mismatch");
    assert_eq!(arity(&run, 0), 1);
    assert_eq!(lambda(&run, 0), "(lam1 (app F e (lam2 (app P $0 (app $2 $1)))))");
    let orig = programs(&run, "original_programs");
    let rewr = programs(&run, "rewritten_programs");
    assert_eq!(rewr[0], "(app fn_0 g)", "closed filler is passed bare, not lam1-wrapped");
    assert_eq!(rewr[1], orig[1], "the 3-child call can't match the 2-child pattern");
    assert_eq!(rewr[2], "(app fn_0 k)");
}

// ─── B. higher-order capture ────────────────────────────────────────────────

/// A higher-order slot that captures both indices of a `lam2` keeps them in
/// source order, including for a filler that uses them swapped.
#[test]
fn two_index_capture_keeps_argument_order() {
    let (bf, smc) = run_both_checked("ho_two_index_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(arity(run, 0), 1, "{name}: one higher-order slot");
        assert_eq!(lambda(run, 0), "(lam1 (app F e (lam2 (app P $0 (app $2 $1 $0)))))", "{name}");
        let rewr = programs(run, "rewritten_programs");
        assert!(rewr.iter().all(|p| p.starts_with("(app fn_0 ")), "{name}: all four programs should use fn_0: {rewr:#?}");
        assert!(rewr[1].contains("(app h $0 $1)"), "{name}: captured args must keep source order: {}", rewr[1]);
    }
}

/// A continuation that uses variables bound by `define`s inside the
/// abstraction becomes one higher-order slot, with both `define`s kept in the
/// body. The exact string isn't pinned, since which indices the slot captures
/// depends on the capture analysis.
#[test]
fn continuation_capture_across_define() {
    let (bf, _) = run_both_checked("cps_define_capture");
    assert_eq!(arity(&bf, 0), 1, "one continuation slot: {}", lambda(&bf, 0));
    assert!(lambda(&bf, 0).contains("(define (app parse $0)"), "the window should keep both defines: {}", lambda(&bf, 0));
}

/// A higher-order slot captures only the binder indices its fillers use,
/// skipping unused ones. Full expansion (capturing all three, passing a
/// `lam3`) still passes the oracle, so the body and `lam2` width are pinned.
#[test]
fn partial_capture_skips_unused_binder_slots() {
    let (bf, smc) = run_both_checked("ho_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (app F e (lam3 (app P $1 (app $3 $2 $0)))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(2); 4], "{name}: every call site should pass a lam2, not a lam3");
        let rewr = programs(run, "rewritten_programs");
        assert_eq!(rewr[1], "(app fn_0 (lam2 (app h $0 (app num_add $1 one))))", "{name}: $2 → $1, $0 stays");
        assert_eq!(rewr[3], "(app fn_0 (lam2 (if (app lt $0 $1) (app m $0) (app m $1))))", "{name}");
    }
}

/// Partial capture works across a wide binder group: using only the first and
/// last of 100 slots still captures just those two.
#[test]
fn partial_capture_under_lam100_spans_the_whole_group() {
    let (bf, _) = run_both_checked("lam100_ho_capture");
    assert_eq!(lambda(&bf, 0), "(lam1 (app F e (lam100 (app P $50 (app $100 $99 $0)))))");
    assert_eq!(call_site_wraps(&bf), vec![Some(2); 4]);
    assert_eq!(programs(&bf, "rewritten_programs")[3], "(app fn_0 (lam2 (app m $0 (app n $1))))");
}

/// A higher-order slot in a `define`'s value captures the enclosing let, not
/// the one that `define` introduces.
#[test]
fn define_value_slot_captures_the_outer_let() {
    let (bf, smc) = run_both_checked("define_value_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app $1 $0) (app use $0))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(1); 4], "{name}");
    }
}

/// A continuation slot captures only the `define`-bound variables its fillers
/// use.
#[test]
fn define_continuation_captures_only_the_used_let() {
    let (bf, smc) = run_both_checked("define_partial_capture");
    for (name, run) in [("best-first", &bf), ("smc", &smc)] {
        assert_eq!(lambda(run, 0), "(lam1 (define (app fetch u) (define (app parse $0) (app $2 $1))))", "{name}");
        assert_eq!(call_site_wraps(run), vec![Some(1); 4], "{name}");
        assert_eq!(programs(run, "rewritten_programs")[3], "(app fn_0 (lam1 (if (app ok $0) (app send $0) done)))", "{name}");
    }
}

/// A higher-order slot used at two binder depths shifts its deeper captures by
/// the inner binder's width, so a `lam1` or `lam2` inner binder compresses the
/// same.
#[test]
fn cross_depth_capture_shifts_by_binder_width() {
    let (l1, _) = run_both_checked("cross_depth_lam1");
    let (l2, _) = run_both_checked("cross_depth_lam2");
    assert_eq!(l1["final_cost"], l2["final_cost"], "lam1 vs lam2 inner binder must compress identically");
    assert_eq!(arity(&l1, 0), arity(&l2, 0));
    assert_eq!(library(&l1)[0]["num_matches"], library(&l2)[0]["num_matches"]);
}

// ─── C. pipeline-level ──────────────────────────────────────────────────────

/// A run that learns several abstractions keeps every invariant
/// for each library entry and the programs rewritten with it.
#[test]
fn multiple_abstractions_stay_sound() {
    for search in ["best-first", "smc"] {
        let run = run_ts_n(search, "mixed", "3", &[]);
        assert!(library(&run).len() >= 2, "{search}: expected a stacked library on the mixed corpus");
        assert_all_invariants(&run, &format!("mixed[{search}]"));
    }
}

/// Lower-bound pruning never discards the optimum: best-first finds the same
/// cost with it on or off.
#[test]
fn lower_bound_pruning_preserves_the_optimum() {
    for corpus in ["lam2_metavar_depth", "define_body_slot", "ho_two_index_capture", "cps_define_capture", "cross_depth_lam2", "ho_partial_capture", "define_value_capture", "free_var_barely_free"] {
        let on = run_ts("best-first", corpus, &["--lower-bound", "on"]);
        let off = run_ts("best-first", corpus, &["--lower-bound", "off"]);
        assert_eq!(on["final_cost"], off["final_cost"], "{corpus}: pruning changed the optimum");
    }
}

/// When best-first exhausts its frontier on a tiny corpus, its answer is
/// optimal, so SMC can match it but never beat it.
#[test]
fn smc_never_beats_a_converged_best_first() {
    for corpus in ["lam2_metavar_depth", "define_value_slot", "ho_two_index_capture", "cross_depth_lam1"] {
        let bf = run_ts("best-first", corpus, &[]);
        let converged = bf["heap_sizes_at_end"].as_array().is_some_and(|h| h.iter().all(|x| x == 0));
        if !converged {
            continue;
        }
        let smc = run_ts("smc", corpus, &[]);
        assert!(smc["final_cost"].as_u64() >= bf["final_cost"].as_u64(), "{corpus}: SMC beat an exhausted best-first");
    }
}

/// `--follow` on a flat higher-order pattern: the η-wrapped display
/// `(app ?#0 $1 $0)` must parse through the default flat parser and be
/// collapsed by `unwrap_pattern_db_apps` when matched.
#[test]
fn follow_reaches_two_index_capture() {
    use clap::Parser;
    use egg_stitch::lang::{LanguageFamily, TsOp, TypeScript};
    use egg_stitch::{Args, io, smc};
    use rand::SeedableRng;

    let follow = "(app F e (lam2 (app P $0 (app ?#0 $1 $0))))";
    let input = format!("{DIR}/ho_two_index_capture.json");
    let args = Args::parse_from(["egg-stitch", "--search", "smc", "--input", &input, "--language", "typescript", "--num-steps", "500", "--num-particles", "500", "--temperature", "100", "--follow", follow]);
    let (data, _, _) = io::load_egraph::<TypeScript, TsOp>(&args.input, None, false, args.weights, args.iter_limit, args.node_limit);
    let result = smc::smc(data, &args, &mut rand::rngs::StdRng::seed_from_u64(0));
    let target = TypeScript::parse_follow_pattern::<TsOp>(follow).expect("parse follow");
    let (cost, best) = result.best.as_ref().expect("SMC should reach the follow target");
    assert!(best.state.matches_follow(&target), "best (cost {cost}) {} should match {follow}", best.state.pattern);
}

// ─── independent scope walker ───────────────────────────────────────────────

/// A parsed flat s-expression.
#[derive(Debug)]
enum Sx {
    Atom(String),
    List(Vec<Sx>),
}

/// Parses one s-expression (no quoting or comments, which the dialect doesn't use).
fn parse(s: &str) -> Sx {
    let toks: Vec<String> = s.replace('(', " ( ").replace(')', " ) ").split_whitespace().map(String::from).collect();
    let mut pos = 0;
    let sx = parse_at(&toks, &mut pos);
    assert_eq!(pos, toks.len(), "trailing tokens in {s:?}");
    sx
}

/// Recursive-descent step for [`parse`].
fn parse_at(toks: &[String], pos: &mut usize) -> Sx {
    let tok = &toks[*pos];
    *pos += 1;
    if tok != "(" {
        return Sx::Atom(tok.clone());
    }
    let mut items = Vec::new();
    while toks[*pos] != ")" {
        items.push(parse_at(toks, pos));
    }
    *pos += 1;
    Sx::List(items)
}

/// Slots that head `head` binds over its `j`-th argument (head excluded):
/// `lam{n}` binds n over its body, `define` binds 1 over its second child.
/// Written out here rather than borrowed from `TsOp::binds_child` on purpose.
fn binds(head: &str, j: usize) -> i32 {
    match (head, j) {
        ("define", 1) => 1,
        (h, 0) => h.strip_prefix("lam").and_then(|n| n.parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

/// Free de Bruijn indices of `sx`, relative to its root.
fn free_vars(sx: &Sx) -> BTreeSet<i32> {
    /// Collects free indices of `sx`, which sits under `depth` binders.
    fn go(sx: &Sx, depth: i32, out: &mut BTreeSet<i32>) {
        match sx {
            Sx::Atom(a) => {
                if let Some(n) = a.strip_prefix('$').and_then(|r| r.parse::<i32>().ok())
                    && n >= depth
                {
                    out.insert(n - depth);
                }
            }
            Sx::List(items) => {
                let head = match items.first() {
                    Some(Sx::Atom(h)) => h.as_str(),
                    _ => "",
                };
                for (j, child) in items.iter().skip(1).enumerate() {
                    go(child, depth + binds(head, j), out);
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    go(sx, 0, &mut out);
    out
}

/// One occurrence of a library function name.
enum FnUse {
    /// `(app fn_k a_1 … a_argc)`.
    Call(usize, usize),
    /// `fn_k` anywhere else: as an op head, an argument, a bare leaf.
    Misplaced(usize),
}

/// `Some(k)` iff `atom` is a library name `fn_k`.
fn fn_index(atom: &str) -> Option<usize> {
    atom.strip_prefix("fn_").and_then(|k| k.parse().ok())
}

/// Collects every [`FnUse`] in `sx`.
fn fn_uses(sx: &Sx, out: &mut Vec<FnUse>) {
    match sx {
        Sx::Atom(a) => out.extend(fn_index(a).map(FnUse::Misplaced)),
        Sx::List(items) => {
            let mut rest = &items[..];
            if let [Sx::Atom(app), Sx::Atom(callee), args @ ..] = items.as_slice()
                && app == "app"
                && let Some(k) = fn_index(callee)
            {
                out.push(FnUse::Call(k, args.len()));
                rest = args;
            }
            rest.iter().for_each(|c| fn_uses(c, out));
        }
    }
}

/// For each rewritten program that is a top-level call `(app fn_k X …)`, the
/// binder width `n` if its first argument is an η-wrap `(lam{n} …)`, else
/// `None`. Lets a test assert how much a higher-order slot captured without
/// pinning every filler.
fn call_site_wraps(run: &Value) -> Vec<Option<usize>> {
    programs(run, "rewritten_programs")
        .iter()
        .map(|p| match parse(p) {
            Sx::List(items) => match items.as_slice() {
                [Sx::Atom(app), Sx::Atom(callee), Sx::List(arg), ..] if app == "app" && fn_index(callee).is_some() => match arg.first() {
                    Some(Sx::Atom(h)) => h.strip_prefix("lam").and_then(|n| n.parse().ok()),
                    _ => None,
                },
                _ => None,
            },
            Sx::Atom(_) => None,
        })
        .collect()
}

/// The walker itself needs a test, or a bug in it would weaken every
/// invariant above.
#[test]
fn scope_walker_agrees_with_the_binder_rules() {
    let fv = |s: &str| free_vars(&parse(s)).into_iter().collect::<Vec<_>>();
    assert_eq!(fv("(lam2 (app f $0 $1 $2))"), vec![0], "lam2 binds $0,$1; $2 escapes as $0");
    assert_eq!(fv("(define $0 $0)"), vec![0], "define's value is outside the let");
    assert_eq!(fv("(define v $1)"), vec![0], "define's body is inside it");
    assert_eq!(fv("(app lam2 $0)"), vec![0], "`lam2` as an argument binds nothing");
    assert_eq!(fv("(lam0 $0)"), vec![0], "lam0 binds nothing");
    assert_eq!(fv("(lam100 (app f $99 $100))"), vec![0], "multi-digit widths parse");
    let wraps = call_site_wraps(&serde_json::json!({"rewritten_programs": ["(app fn_0 (lam2 $0))", "(app fn_0 f)", "(lam1 (app fn_0 (lam1 $0)))"]}));
    assert_eq!(wraps, vec![Some(2), None, None], "only a top-level call's first argument counts");
    let mut uses = Vec::new();
    fn_uses(&parse("(app fn_0 (fn_1 a) (app fn_2))"), &mut uses);
    assert!(matches!(uses.as_slice(), [FnUse::Call(0, 2), FnUse::Misplaced(1), FnUse::Call(2, 0)]));
}
