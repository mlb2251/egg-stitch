use crate::lang::{LanguageFamily, StitchAnalysis, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::shared::SharedData;
use anyhow::anyhow;
use egg::{Analysis, Pattern, Rewrite};
use std::{fs, path::Path};

/// Loads a JSON file containing s-expressions and builds an egraph from them.
/// All programs are combined into a single term (programs A B C ...).
/// Returns the egraph, the root e-class Id of the programs node, the
/// minimum AST cost of that root *before* any rewrites were applied, and
/// the original program strings as parsed from the input file.
/// `iter_limit` and `node_limit` cap the e-saturation iterations and egraph
/// node count when applying rules.
pub fn load_egraph<F: LanguageFamily, O: StitchOp>(filename: &str, rule_file: Option<&str>, only_use_dsrs_at_start: bool, weights: Weights, iter_limit: usize, node_limit: usize) -> (SharedData<F, O>, usize, Vec<String>) {
    let contents = std::fs::read_to_string(filename).expect("Failed to read file");
    let exprs: Vec<String> = serde_json::from_str(&contents).expect("Failed to parse JSON");
    println!("Loaded {} programs", exprs.len());

    let (egraph_before_rules, root) = programs_to_egraph::<F::Apply<O>>(&exprs, weights);
    println!("Egraph size: {}", egraph_before_rules.classes().len());

    let cost_before_rewrites = extract_root_size(&egraph_before_rules, root);
    println!("Weight of root node before rules: {}", cost_before_rewrites);

    let rules: Vec<egg::Rewrite<F::Apply<O>, StitchAnalysis>> = match rule_file {
        Some(rule_file) => from_file(rule_file).expect("Failed to parse rules file"),
        None => vec![],
    };
    println!("loaded {} rules", rules.len());

    let mut runner: egg::Runner<F::Apply<O>, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(weights));
    runner = runner.with_egraph(egraph_before_rules).with_iter_limit(iter_limit).with_node_limit(node_limit).run(&rules);
    runner.egraph.rebuild();
    println!("Weight of root node after rules:  {}", extract_root_size(&runner.egraph, root));
    println!("Egraph size: {}", runner.egraph.classes().len());

    // With `--only-use-dsrs-at-start`, the rules only serve to find a better
    // initial representation: extract the normalized min-term and rebuild a
    // fresh rule-free egraph for the search to run over.
    if only_use_dsrs_at_start {
        let programs = extract_programs::<F::Apply<O>>(&runner.egraph, root);
        let data = egraph_from_programs::<F, O>(&programs, None, weights, iter_limit, node_limit);
        println!("Egraph size after dropping rules: {}", data.egraph.classes().len());
        return (data, cost_before_rewrites, exprs);
    }
    (SharedData::new(runner.egraph, root), cost_before_rewrites, exprs)
}

/// Extracts the size-minimal program string for each child of the `programs`
/// root e-class, using `WeightedSize` extraction.
pub fn extract_programs<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: egg::Id) -> Vec<String> {
    let extractor = egg::Extractor::new(egraph, crate::cost::WeightedSize { weights: egraph.analysis.weights });
    let programs_node = egraph[root].nodes.iter().find(|n| n.is_programs_node()).expect("root e-class should contain a `programs` enode");
    programs_node.children().iter().map(|&child| L::display_recexpr(&extractor.find_best(child).1)).collect()
}

/// Builds a fresh egraph from program strings, applies rewrite rules, and returns it with its root.
///
/// Used between abstractions: the rewritten programs are extracted as strings and fed into a
/// clean egraph, discarding all prior equivalences. `iter_limit` and `node_limit`
/// cap the e-saturation iterations and egraph node count when applying rules.
pub fn egraph_from_programs<F: LanguageFamily, O: StitchOp>(programs: &[String], rule_file: Option<&str>, weights: Weights, iter_limit: usize, node_limit: usize) -> SharedData<F, O> {
    let (egraph, root) = programs_to_egraph::<F::Apply<O>>(programs, weights);
    let rules: Vec<egg::Rewrite<F::Apply<O>, StitchAnalysis>> = match rule_file {
        Some(f) => from_file(f).expect("Failed to parse rules file"),
        None => vec![],
    };
    let mut runner: egg::Runner<F::Apply<O>, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(weights));
    runner = runner.with_egraph(egraph).with_iter_limit(iter_limit).with_node_limit(node_limit).run(&rules);
    runner.egraph.rebuild();
    SharedData::new(runner.egraph, root)
}

/// Parses a list of s-expression strings into a fresh egraph wrapped in a `(programs ...)` root.
fn programs_to_egraph<L: StitchLanguage>(programs: &[String], weights: Weights) -> (StitchEgraph<L>, egg::Id) {
    let mut egraph: StitchEgraph<L> = egg::EGraph::new(StitchAnalysis::new(weights));
    let expr_ids: Vec<egg::Id> = programs
        .iter()
        .map(|s| {
            let expr = L::parse_program(s).unwrap_or_else(|e| panic!("Failed to parse expression: {s}: {e}"));
            egraph.add_expr(&expr)
        })
        .collect();
    let programs_node = L::from_op("programs", expr_ids).expect("Failed to create programs node");
    let root = egraph.add(programs_node);
    egraph.rebuild();
    (egraph, root)
}

/// Returns the minimum weighted size of the expression rooted at `root`,
/// using `WeightedSize` so the result matches the egraph's analysis-recorded
/// `data.size` (rather than diverging under non-uniform `Weights`).
fn extract_root_size<L: StitchLanguage>(egraph: &StitchEgraph<L>, root: egg::Id) -> usize {
    let extractor = egg::Extractor::new(egraph, crate::cost::WeightedSize { weights: egraph.analysis.weights });
    let (cost, _) = extractor.find_best(root);
    cost as usize
}

/// Loads rewrite rules from a file in `name: lhs => rhs` format.
pub fn from_file<L, A, P>(path: P) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: StitchLanguage,
    A: Analysis<L>,
    P: AsRef<Path>,
{
    let contents = fs::read_to_string(path)?;
    parse(&contents)
}

/// Parses rewrite rules from a string in `name: lhs => rhs` format. A rule may
/// use `<=>` instead of `=>` to declare a bidirectional equivalence; it expands
/// to the forward rule plus a `<name>-rev` rule with `lhs`/`rhs` swapped.
///
/// A `constant_folding: !<kind>` directive line adds built-in numeric rewrites:
/// `!numbers` folds `+ - * /` over literal leaves, and `!successors` expands an
/// integer literal `n` into `(+ 1 (n-1))` (see [`crate::constant_folding`]). The
/// fold-mode kinds (`!integers`, `!floats`, `!integersarefloats`, `!numbers`)
/// may carry a `(params (ops …))` block, e.g.
/// `constant_folding: !integersarefloats (params (ops (+ * / sin cos pi)))`
/// (with no params they default to `+ - * /`); `!round` rounds every numeric
/// literal to `(params (places N))` decimals (default 6).
///
/// The `fv(c) = fv(MinTerm(c))` invariant that rules must preserve is not
/// checked here; it is verified directly on the built egraph by
/// [`crate::cost::assert_fv_matches_min_term`].
pub fn parse<L, A>(file: &str) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: StitchLanguage,
    A: Analysis<L>,
{
    let mut rewrites = Vec::new();
    for line in file
        .lines()
        .map(|line| {
            let line = line.split_once("//").map_or(line, |(line, _comment)| line);
            line.trim()
        })
        .filter(|line| !line.is_empty())
    {
        let (name, rewrite) = line.split_once(':').ok_or(anyhow!("missing colon"))?;
        // A `constant_folding: !<kind>` directive expands to a built-in family of
        // folding rewrites rather than a single `lhs => rhs` rule.
        if name.trim() == "constant_folding" {
            use crate::constant_folding::{FoldMode, FoldingParams, folding_rewrites, round_rewrite, successor_expansion_rewrite};
            // An optional `(params …)` block may follow the kind, e.g.
            // `!integersarefloats (params (ops (+ * / sin cos pi)))` or
            // `!round (params (places 6))`.
            let directive = rewrite.trim();
            let (kind, rest) = directive.split_once(char::is_whitespace).unwrap_or((directive, ""));
            let rest = rest.trim();
            // Only these kinds take params; the standalone appliers don't.
            let params_allowed = matches!(kind, "!integers" | "!floats" | "!integersarefloats" | "!numbers" | "!round");
            if !rest.is_empty() && !params_allowed {
                return Err(anyhow!("constant_folding: {kind} does not take parameters"));
            }
            let params = FoldingParams::parse(rest)?;
            match kind {
                "!integers" => rewrites.extend(folding_rewrites::<L, A>(FoldMode::Integers, &params.ops)?),
                "!floats" => rewrites.extend(folding_rewrites::<L, A>(FoldMode::Floats, &params.ops)?),
                "!integersarefloats" => rewrites.extend(folding_rewrites::<L, A>(FoldMode::IntegersAreFloats, &params.ops)?),
                // `!numbers` is `!integers` and `!floats` combined (the original behaviour).
                "!numbers" => {
                    rewrites.extend(folding_rewrites::<L, A>(FoldMode::Integers, &params.ops)?);
                    rewrites.extend(folding_rewrites::<L, A>(FoldMode::Floats, &params.ops)?);
                }
                "!successors" => rewrites.push(successor_expansion_rewrite::<L, A>(1)?),
                // `!round` snaps numeric literals to `places` decimals (default 6),
                // killing float noise and unifying near-equal values.
                "!round" => rewrites.push(round_rewrite::<L, A>(params.places.unwrap_or(6))?),
                other => return Err(anyhow!("unknown constant_folding kind {other:?} (supported: !integers, !floats, !integersarefloats, !numbers, !successors, !round)")),
            }
            continue;
        }
        // `=>` is a substring of `<=>`, so check the bidirectional arrow first.
        let (lhs, rhs, bidirectional) = match rewrite.split_once("<=>") {
            Some((lhs, rhs)) => (lhs, rhs, true),
            None => {
                let (lhs, rhs) = rewrite.split_once("=>").ok_or(anyhow!("missing arrow"))?;
                (lhs, rhs, false)
            }
        };
        let name = name.trim();
        let lhs: Pattern<L> = L::parse_pattern_ast(lhs.trim())?.into();
        let rhs: Pattern<L> = L::parse_pattern_ast(rhs.trim())?.into();
        if bidirectional {
            rewrites.push(Rewrite::new(format!("{name}-rev"), rhs.clone(), lhs.clone()).map_err(|e| anyhow!("{}", e))?);
        }
        rewrites.push(Rewrite::new(name, lhs, rhs).map_err(|e| anyhow!("{}", e))?);
    }
    Ok(rewrites)
}
