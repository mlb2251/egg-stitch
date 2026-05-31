use crate::lang::{LanguageFamily, StitchAnalysis, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};
use crate::shared::SharedData;
use anyhow::anyhow;
use egg::{Analysis, ENodeOrVar, Id, Pattern, RecExpr, Rewrite, Var};
use rustc_hash::FxHashSet;
use std::{fs, path::Path};

/// Loads a JSON file containing s-expressions and builds an egraph from them.
/// All programs are combined into a single term (programs A B C ...).
/// Returns the egraph, the root e-class Id of the programs node, the
/// minimum AST cost of that root *before* any rewrites were applied, and
/// the original program strings as parsed from the input file.
pub fn load_egraph<F: LanguageFamily, O: StitchOp>(filename: &str, rule_file: Option<&str>, weights: Weights) -> (SharedData<F, O>, usize, Vec<String>) {
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
    runner = runner.with_egraph(egraph_before_rules).with_iter_limit(10).run(&rules);
    runner.egraph.rebuild();
    println!("Weight of root node after rules:  {}", extract_root_size(&runner.egraph, root));
    println!("Egraph size: {}", runner.egraph.classes().len());
    (SharedData::new(runner.egraph, root), cost_before_rewrites, exprs)
}

/// Builds a fresh egraph from program strings, applies rewrite rules, and returns it with its root.
///
/// Used between abstractions: the rewritten programs are extracted as strings and fed into a
/// clean egraph, discarding all prior equivalences.
pub fn egraph_from_programs<F: LanguageFamily, O: StitchOp>(programs: &[String], rule_file: Option<&str>, weights: Weights) -> SharedData<F, O> {
    let (egraph, root) = programs_to_egraph::<F::Apply<O>>(programs, weights);
    let rules: Vec<egg::Rewrite<F::Apply<O>, StitchAnalysis>> = match rule_file {
        Some(f) => from_file(f).expect("Failed to parse rules file"),
        None => vec![],
    };
    let mut runner: egg::Runner<F::Apply<O>, StitchAnalysis> = egg::Runner::new(StitchAnalysis::new(weights));
    runner = runner.with_egraph(egraph).with_iter_limit(10).run(&rules);
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
        if let Some(reason) = rule_fv_condition_violation::<L>(&lhs.ast, &rhs.ast) {
            panic!("rule `{name}` violates the min-term free-variable conditions: {reason}");
        }
        if bidirectional {
            rewrites.push(Rewrite::new(format!("{name}-rev"), rhs.clone(), lhs.clone()).map_err(|e| anyhow!("{}", e))?);
        }
        rewrites.push(Rewrite::new(name, lhs, rhs).map_err(|e| anyhow!("{}", e))?);
    }
    Ok(rewrites)
}

/// Structural facts about one side of a rule, used by
/// [`rule_fv_condition_violation`]: its metavariables, whether it contains a
/// *free* de Bruijn leaf, whether any metavariable sits beneath a binder, and its
/// node count (a metavariable counts as one node — its minimal instantiation).
fn rule_side_facts<L: StitchLanguage>(ast: &RecExpr<ENodeOrVar<L>>) -> (FxHashSet<Var>, bool, bool, usize) {
    let nodes = ast.as_ref();
    let mut vars = FxHashSet::default();
    let (mut free_db, mut mv_under_binder, mut count) = (false, false, 0usize);

    fn go<L: StitchLanguage>(nodes: &[ENodeOrVar<L>], id: Id, depth: u32, vars: &mut FxHashSet<Var>, free_db: &mut bool, mv: &mut bool, count: &mut usize) {
        *count += 1;
        match &nodes[usize::from(id)] {
            ENodeOrVar::Var(v) => {
                vars.insert(*v);
                if depth > 0 {
                    *mv = true;
                }
            }
            ENodeOrVar::ENode(e) => {
                let disc = e.discriminant();
                if let Some(idx) = disc.de_bruijn_index()
                    && idx >= depth as i32
                {
                    *free_db = true;
                }
                for (j, &c) in e.children().iter().enumerate() {
                    let child_depth = depth + u32::from(disc.binds_child(j));
                    go::<L>(nodes, c, child_depth, vars, free_db, mv, count);
                }
            }
        }
    }
    go::<L>(nodes, Id::from(nodes.len() - 1), 0, &mut vars, &mut free_db, &mut mv_under_binder, &mut count);
    (vars, free_db, mv_under_binder, count)
}

/// Checks the *structural* conditions under which `fv(c) = fv(MinTerm(c))` is
/// guaranteed — the invariant the extraction-time `shift_free_egraph` assertion
/// relies on. Returns `Some(reason)` for the first violated condition, or `None`
/// if the rule passes. Treating every rule as a bidirectional union, we require:
///   * no rule side has a *free* de Bruijn leaf,
///   * no metavariable occurs beneath a binder,
///   * every metavariable occurring on only one side makes that side strictly
///     larger (so no rewrite introduces a variable on its smaller side).
///
/// These are sufficient, not necessary, conditions, and confluence (the remaining
/// hypothesis) is not checked here; the extraction-time assertion is the backstop.
/// `parse` panics on a `Some` result so a non-conforming rule set fails at load.
pub fn rule_fv_condition_violation<L: StitchLanguage>(lhs: &RecExpr<ENodeOrVar<L>>, rhs: &RecExpr<ENodeOrVar<L>>) -> Option<String> {
    let (lv, lfree, lbind, lcount) = rule_side_facts::<L>(lhs);
    let (rv, rfree, rbind, rcount) = rule_side_facts::<L>(rhs);
    if lfree || rfree {
        return Some("a rule side contains a free de Bruijn variable".to_string());
    }
    if lbind || rbind {
        return Some("a metavariable occurs beneath a binder".to_string());
    }
    if lv.difference(&rv).next().is_some() && lcount <= rcount {
        return Some("a metavariable occurs only on the LHS, which is not strictly larger".to_string());
    }
    if rv.difference(&lv).next().is_some() && rcount <= lcount {
        return Some("a metavariable occurs only on the RHS, which is not strictly larger".to_string());
    }
    None
}
