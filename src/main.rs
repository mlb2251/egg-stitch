mod lang;
mod util;
mod pattern;
mod search;
mod revexpr;
mod smc;
mod rewrites;

use lang::StitchLang;
use pattern::Pattern;
use search::{SharedSearchData, SearchState};
use smc::compute_cost;

fn main() {
    let rules = "../babble/harness/data/benchmark-dsrs/list.rewrites";
    let (egraph, root) = util::load_egraph("data/domains/list/list_hard_test_ellisk_2019-02-15T11.26.41__bench000_it0.json", Some(rules));

    smc::smc(egraph, root, None);

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



