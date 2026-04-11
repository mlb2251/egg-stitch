use std::cmp::min;

use crate::cost::{compute_cost, compute_size};
use crate::lang::{StitchEgraph, StitchLang};
use crate::search::{SearchState, SharedSearchData};
use rand::Rng;

pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> Option<(usize, SearchState)> {
    let shared = SharedSearchData { egraph, p_reuse: args.p_reuse };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared));
    println!("original size of egraph: {}", original_size);

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs as i64;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;

    // make a bunch of search states
    let mut search_states: Vec<SearchState> = (0..num_particles).map(|_| SearchState::new(&shared)).collect();

    for step in 0..num_steps {
        for search_state in &mut search_states {
            search_state.expand_random(&shared);
        }

        let costs: Vec<usize> = search_states
            .iter()
            .map(|search_state| compute_cost(&shared.egraph, root, search_state))
            .collect();

        for (i, cost) in costs.iter().enumerate() {
            if best_so_far.as_ref().is_none_or(|best| *cost < best.0) {
                println!("[iteration {}] new best: {} {}", step, cost, search_states[i].pattern);
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }


        let mut weights: Vec<f64> = costs.iter().map(|cost| -(*cost as f64) / temperature).collect();
        let max_weight = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        for w in &mut weights {
            *w = (*w - max_weight).exp();
        }

        // force no resampling of completed patterns
        for (i, state) in search_states.iter().enumerate() {
            if state.pattern.vars.is_empty() {
                weights[i] = 0.0;
            }
        }

        if weights.iter().sum::<f64>() == 0.0 {
            println!("all particles died, stopping");
            break;
        }

        if best_found_at.is_some_and(|best_found_at| (step as i64) - (best_found_at as i64) > dead_runs) {
            println!("no progress in 100 steps, stopping at {}", step);
            break;
        }

        // resample
        normalize_and_accumulate(&mut weights);

        println!("Step {}: expanded all particles", step);
        for i in 0..min(5, search_states.len()) {
            println!("Sample particle {}: {}; cost={} weight={}", i, search_states[i].pattern, costs[i], weights[i]);
        }

        search_states = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights);
                search_states[idx].clone()
            })
            .collect();
    }

    let cost = compute_cost(&shared.egraph, root, &best_so_far.as_ref().unwrap().1);
    println!("best found at iteration {}: {}", best_found_at.unwrap(), cost);
    println!("program: {}", best_so_far.as_ref().unwrap().1.pattern);
    println!("best: {}", cost);
    println!("Compression ratio: {}", original_size as f64 / cost as f64);
    // crate::util::print_programs(&term);

    best_so_far
}

pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    // println!("Choosing from weights: {:?}", cum_weights);
    let r: f64 = rand::rng().random_range(0.0..1.0);
    // println!("r: {:?}", r);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) => idx,
        Err(idx) => idx, // it could be inserted at idx, which means it's <= cum_weights[idx]
    }
}

pub fn normalize_and_accumulate(weights: &mut Vec<f64>) {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum == 0.0 {
        let len = weights.len();
        weights.fill(1.0 / len as f64);
    } else {
        weights.iter_mut().for_each(|w| *w /= weight_sum);
    }
    let mut accum = 0.0;
    for w in weights {
        accum += *w;
        *w = accum;
    }
}

#[allow(dead_code)]
pub fn rewrite_slow(egraph: &StitchEgraph, root: egg::Id, search_state: &SearchState) -> usize {
    let mut egraph = egraph.clone(); // todo be smarter

    // println!("search state: {}", search_state);
    // let mut nodes = vec![];
    for m in &search_state.matches {
        // println!("match at eclass {}: {:?}", m.root_eclass, m.substs);
        for subst in &m.substs {
            let node: StitchLang = StitchLang {
                op: "inv_0".into(),
                children: subst.vars.clone(),
            };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
            // nodes.push(x);
        }
    }
    egraph.rebuild();
    // let extractor = egg::Extractor::new(&egraph, egg::AstSize);
    // let (cost, term) = extractor.find_best(root);
    // for n in nodes {
    //     egraph
    // }
    // assert_eq!(egraph[root].data as usize, cost);
    // println!("cost from extractor: {}", cost);
    // println!("cost from egraph: {}", egraph[root].data);
    egraph[root].data as usize
    // return (cost, term);
}
