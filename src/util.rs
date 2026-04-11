use crate::{lang::{StitchAnalysis, StitchEgraph, StitchLang}, rewrites::from_file};
use egg::{FromOp, Rewrite};

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
        println!("Loaded expression: {:?}", expr);
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

fn extract_root_size(egraph: &StitchEgraph, root: egg::Id) -> usize {
    let extractor = egg::Extractor::new(egraph, egg::AstSize);
    let (expr, _) = extractor.find_best(root);
    expr
}

// fn read_rules(rule_file: &str) -> Vec<egg::Rewrite<StitchLang, StitchAnalysis>> {
//     // let rules = std::fs::read_to_string(rule_file).expect("Failed to read rules file");
//     // let mut rules_vec = Vec::new();
//     // for line in rules.lines() {
//     //     let rule: egg::Rewrite<StitchLang, StitchAnalysis> = line.parse().expect("Failed to parse rule");
//     //     rules_vec.push(rule);
//     // }
//     // rules_vec
//     // vec![
//     //     egg::rewrite!("commute_add"; "(+ ?a ?b)" => "(+ ?b ?a)"),
//     //     egg::rewrite!("assoc_add"; "(+ ?a (+ ?b ?c))" => "(+ (+ ?a ?b) ?c)"),
//     //     egg::rewrite!("identity_add"; "(+ ?a 0)" => "?a"),
//     // ];
//     // rules file looks like this:
//     // 
//     // // reroll new
//     // r3roll_2_x: (C (C ?a (T (T ?s (M 1 0 (- 0 ?x) ?y1)) (M 1 0 0 ?y2))) (T (T ?s (M 1 0 ?x ?y1)) (M 1 0 0 ?y2))) => (repeat (T ?s (M 1 0 (- 0 ?x) ?y1)) 2 (M 1 0 (* 2 ?x) ?y2))
//     // r3roll_3_x: (C (C (C ?a (T (T ?s (M 1 0 (- 0 ?x) ?y1)) (M 1 0 0 ?y2))) (T (T ?s (M 1 0 ?x ?y1)) (M 1 0 0 ?y2))) (T (T ?s (M 1 0 ?x ?y1)) (M 1 0 0 ?y2))) => (repeat (T ?s (M 1 0 (- 0 ?x) ?y1)) 3 (M 1 0 ?x ?y2))

//     let mut rules_vec: Vec<egg::Rewrite<StitchLang, StitchAnalysis>> = Vec::new();
//     let rules = std::fs::read_to_string(rule_file).expect("Failed to read rules file");
//     for line in rules.lines() {
//         if line.starts_with("//") || line.is_empty() {
//             continue;
//         }
//         let rule: egg::Rewrite<StitchLang, StitchAnalysis> = parse_rule(line).expect("Failed to parse rule");
//         rules_vec.push(rule);
//     }
//     rules_vec
// }

// fn parse_rule(rule_str: &str) -> Result<egg::Rewrite<StitchLang, StitchAnalysis>, String> {
//     let parts: Vec<&str> = rule_str.split(": ").collect();
//     if parts.len() != 2 {
//         return Err(format!("Invalid rule format: {}", rule_str));
//     }
//     let rule_name = parts[0].to_string();
//     let rule_body = parts[1];
//     let parts: Vec<&str> = rule_body.split(" => ").collect();
//     if parts.len() != 2 {
//         return Err(format!("Invalid rule format: {}", rule_str));
//     }
//     let lhs = parts[0].to_string();
//     let rhs = parts[1].to_string();
//     // return Ok(egg:Rewrite::new(rule_name, lhs, rhs));
//     return Ok(egg::rewrite!(rule_name; lhs => rhs));
// }

/// Prints a programs term with each child on a new line.
/// If the term is not a programs node, prints it normally.
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
