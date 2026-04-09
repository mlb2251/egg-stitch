use crate::{lang::StitchLang, rewrites::from_file, smc::{StitchAnalysis, StitchEgraph}};
use egg::FromOp;

/// Loads a JSON file containing s-expressions and builds an egraph from them.
/// All programs are combined into a single term (programs A B C ...).
/// Returns the egraph and the root e-class Id of the programs node.
pub fn load_egraph(filename: &str, rule_file: Option<&str>) -> (StitchEgraph, egg::Id) {
    let contents = std::fs::read_to_string(filename).expect("Failed to read file");
    let exprs: Vec<String> = serde_json::from_str(&contents).expect("Failed to parse JSON");

    let mut egraph: StitchEgraph = egg::EGraph::default();

    let mut expr_ids = Vec::new();

    for expr_str in &exprs {
        let expr: egg::RecExpr<StitchLang> = expr_str.parse().expect("Failed to parse expression");
        // println!("Loaded expression: {:?}", expr);
        expr_ids.push(egraph.add_expr(&expr));
    }

    let programs_node = StitchLang::from_op("programs", expr_ids.clone()).expect("Failed to create programs node");
    let root = egraph.add(programs_node);
    println!("Loaded {} programs", expr_ids.len());
    println!("Egraph size: {}", egraph.classes().len());

    println!("Weight of root node before rules: {}", extract_root_size(&egraph, root));
    let rules: Vec<egg::Rewrite<StitchLang, StitchAnalysis>> = match rule_file {
        Some(rule_file) => from_file(rule_file).expect("Failed to parse rules file"),
        None => vec![],
    };
    println!("{:#?}", rules);
        //  from_file(rule_file).expect("Failed to parse rules file");
    egraph.rebuild(); // might be unnecessary
    let mut runner: egg::Runner<StitchLang, StitchAnalysis> = egg::Runner::default();
    runner = runner.with_egraph(egraph)
        .with_iter_limit(10)
        .run(&rules);

    runner.egraph.rebuild(); // might be unnecessary
    println!("Weight of root node after rules:  {}", extract_root_size(&runner.egraph, root));
    println!("Egraph size: {}", runner.egraph.classes().len());
    (runner.egraph, root)
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
