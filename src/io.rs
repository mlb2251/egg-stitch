use crate::lang::{StitchAnalysis, StitchEgraph, StitchLang};
use anyhow::anyhow;
use egg::{Analysis, FromOp, Language, Pattern, Rewrite};
use std::{error::Error, fs, path::Path};

/// Loads a JSON file containing s-expressions and builds an egraph from them.
/// All programs are combined into a single term (programs A B C ...).
/// Returns the egraph, the root e-class Id of the programs node, and the
/// minimum AST cost of that root *before* any rewrites were applied.
pub fn load_egraph(filename: &str, rule_file: Option<&str>) -> (StitchEgraph, egg::Id, usize) {
    let contents = std::fs::read_to_string(filename).expect("Failed to read file");
    let exprs: Vec<String> = serde_json::from_str(&contents).expect("Failed to parse JSON");

    let mut egraph: StitchEgraph = egg::EGraph::default();

    let mut expr_ids = Vec::new();

    for expr_str in &exprs {
        let expr: egg::RecExpr<StitchLang> = expr_str.parse().expect("Failed to parse expression");
        expr_ids.push(egraph.add_expr(&expr));
    }

    let programs_node = StitchLang::from_op("programs", expr_ids.clone()).expect("Failed to create programs node");
    let root = egraph.add(programs_node);
    println!("Loaded {} programs", expr_ids.len());
    println!("Egraph size: {}", egraph.classes().len());

    let cost_before_rewrites = extract_root_size(&egraph, root);
    println!("Weight of root node before rules: {}", cost_before_rewrites);
    let rules: Vec<egg::Rewrite<StitchLang, StitchAnalysis>> = match rule_file {
        Some(rule_file) => from_file(rule_file).expect("Failed to parse rules file"),
        None => vec![],
    };
    println!("loaded {} rules", rules.len());
    egraph.rebuild();
    let mut runner: egg::Runner<StitchLang, StitchAnalysis> = egg::Runner::default();
    runner = runner.with_egraph(egraph).with_iter_limit(10).run(&rules);

    runner.egraph.rebuild();
    println!("Weight of root node after rules:  {}", extract_root_size(&runner.egraph, root));
    println!("Egraph size: {}", runner.egraph.classes().len());
    (runner.egraph, root, cost_before_rewrites)
}

/// Returns the minimum AST size of the expression rooted at `root`.
fn extract_root_size(egraph: &StitchEgraph, root: egg::Id) -> usize {
    let extractor = egg::Extractor::new(egraph, egg::AstSize);
    let (expr, _) = extractor.find_best(root);
    expr
}

/// Prints a programs term with each child on a new line.
/// If the term is not a programs node, prints it normally.
#[allow(dead_code)]
pub fn print_programs(term: &egg::RecExpr<StitchLang>) {
    let root_node = &term.as_ref()[term.as_ref().len() - 1];
    if root_node.op.as_str() == "programs" {
        println!("(programs");
        for &child_id in &root_node.children {
            print!("  ");
            print_expr(term, child_id.into());
            println!();
        }
        println!(")");
    } else {
        println!("{}", term);
    }
}

/// Recursively prints an s-expression starting from the given node id.
#[allow(dead_code)]
fn print_expr(term: &egg::RecExpr<StitchLang>, id: usize) {
    let node = &term.as_ref()[id];
    if node.children.is_empty() {
        print!("{}", node.op);
    } else {
        print!("({}", node.op);
        for &child_id in &node.children {
            print!(" ");
            print_expr(term, child_id.into());
        }
        print!(")");
    }
}

/// Loads rewrite rules from a file in `name: lhs => rhs` format.
pub fn from_file<L, A, P>(path: P) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: Language + FromOp + Sync + Send + 'static,
    A: Analysis<L>,
    P: AsRef<Path>,
    L::Error: Send + Sync + Error,
{
    let contents = fs::read_to_string(path)?;
    parse(&contents)
}

/// Parses rewrite rules from a string in `name: lhs => rhs` format.
pub fn parse<L, A>(file: &str) -> anyhow::Result<Vec<Rewrite<L, A>>>
where
    L: Language + FromOp + Sync + Send + 'static,
    A: Analysis<L>,
    L::Error: Send + Sync + Error,
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
        let (lhs, rhs) = rewrite.split_once("=>").ok_or(anyhow!("missing arrow"))?;
        let name = name.trim();
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        let lhs: Pattern<L> = lhs.parse()?;
        let rhs: Pattern<L> = rhs.parse()?;
        rewrites.push(Rewrite::new(name, lhs, rhs).map_err(|e| anyhow!("{}", e))?);
    }
    Ok(rewrites)
}
