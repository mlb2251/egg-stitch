mod io;
mod lang;
mod matching;
mod pattern;
mod revexpr;
mod search;
mod smc;

fn main() {
    let rules = "../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites";
    let (egraph, root) = io::load_egraph("data/domains/cogsci/dials.json", Some(rules));

    smc::smc(egraph, root);

    // let extractor = egg::Extractor::new(&egraph, egg::AstSize);
    // let (_, term) = extractor.find_best(root);
    // util::print_programs(&term);

    // let mut pattern = Pattern::single_var();
    // println!("before expansion: {:?}", pattern.pattern.nodes);
    // println!("before expansion: {}", pattern.pattern);

    // pattern.expand(0, &StitchLang{op: "+".into(), children: vec![2.into(), 3.into()]});
    // println!("after expansion: {:?}", pattern.pattern.nodes);
    // println!("after expansion: {}", pattern.pattern);

    // let shared = SharedSearchData { egraph };
    // let mut search_state = SearchState::new(&shared);
    // println!("search state: {}", search_state);

    // println!("****************************");

    // while search_state.pattern.vars.len() > 0 {
    //     search_state.expand_random(&shared);
    //     println!("cost: {}", compute_cost(&shared.egraph, root, &search_state));
    // }


}



