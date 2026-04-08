use std::cmp::{Reverse, min};
use std::collections::BinaryHeap;

use colored::Colorize;

use crate::lang::StitchLang;
use crate::pattern::Pattern;
use crate::revexpr::RevExpr;
use crate::search::{SearchState, SharedSearchData, Subst};
use egg::{Analysis, ENodeOrVar, Id, Language};
use priority_queue::PriorityQueue;
use rand::Rng;
use rustc_hash::{FxHashMap};

#[derive(Clone, Debug, Default)]
pub struct StitchAnalysis;

impl Analysis<StitchLang> for StitchAnalysis {
    type Data = u32;

    fn make(egraph: &mut egg::EGraph<StitchLang, Self>, enode: &StitchLang, _id: egg::Id) -> Self::Data {
        1 + enode.children.iter().map(|&child_id| egraph[child_id].data).sum::<u32>()
    }

    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> egg::DidMerge {
        if from < *to {
            *to = from;
            egg::DidMerge(true, false)
        } else if from == *to {
            egg::DidMerge(false, false)
        } else {
            // from = *to; but we don't do this because types; idk it seems like they don't want us to
            egg::DidMerge(false, true)
        }
    }
}

pub type StitchEgraph = egg::EGraph<StitchLang, StitchAnalysis>;

pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> Option<(usize, SearchState)> {
    let follow_expr: Option<RevExpr<ENodeOrVar<StitchLang>>> = args.follow.as_deref().map(|s|
        s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e))
    );
    let usage_counts = crate::search::compute_usage_counts(&egraph, root);
    let shared = SharedSearchData { egraph, follow: follow_expr, weight_by_usage: args.weight_by_usage, usage_counts, p_reuse: args.p_reuse, check_slow: args.check_slow };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared), shared.check_slow);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;

    // make a bunch of search states
    let mut search_states: Vec<SearchState> = (0..num_particles)
        .map(|i| SearchState::new(&shared))
        .collect();

    for step in 0..num_steps {
        let mut i = 0;
        for search_state in &mut search_states {
            let verb = false;
            if verb {
                println!("Expanding particle {} with pattern: {}", i, search_state.pattern);
            }
            search_state.expand_random(&shared, verb);
            if verb {
                println!("Expanded particle {} to pattern: {}", i, search_state.pattern);
            }
            i += 1;
        }

        let costs: Vec<usize> = search_states
            .iter()
            .map(|search_state| compute_cost(&shared.egraph, root, search_state, shared.check_slow))
            .collect();
        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity
                && best_so_far.as_ref().is_none_or(|best| *cost < best.0)
            {
                println!("{} {} {}",
                    format!("[iteration {}]", step).yellow().bold(),
                    format!("new best: {}", cost).green().bold(),
                    search_states[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }


        let mut weights: Vec<f64> = costs.iter().map(|cost| (-(*cost as f64)/temperature)).collect();
        let max_weight = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mut j = 0;
        for w in &mut weights {
            // println!("Before reweighting particle {} with cost {}: weight={}", j, costs[j], w);
            *w = (*w - max_weight).exp();
            // println!("After reweighting particle {} with cost {}: weight={}", j, costs[j], w);
            j += 1;
        }

        // force no resampling of completed patterns
        for (i, state) in search_states.iter().enumerate() {
            if state.pattern.vars.is_empty() {
                weights[i] = 0.0;
            }
        }

        // filter out particles inconsistent with the follow target
        if let Some(ref follow) = shared.follow {
            let total_weight: f64 = weights.iter().sum();
            let mut found = false;
            for (i, state) in search_states.iter().enumerate() {
                if !state.matches_follow(follow) {
                    // println!("Particle {} with pattern {} does not match follow pattern, killing it", i, state.pattern);
                    weights[i] = 0.0;
                } else {
                    found = true;
                }
            }
            if (found) {
                let matching_weight: f64 = weights.iter().sum();
                let weight_frac = if total_weight > 0.0 { matching_weight / total_weight } else { 0.0 };
                println!("{} {}", "follow:".dimmed(), format!("{} / {} particles match ({:.1}% of weight)", weights.iter().filter(|&&w| w > 0.0).count(), weights.len(), weight_frac * 100.0).blue());
            } else {
                println!("{}", "No particles match the follow pattern".red().bold());
            }
        }

        if weights.iter().sum::<f64>() == 0.0 {
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }

        if best_found_at.is_some_and(|best_found_at| (step as i64) - (best_found_at as i64) > dead_runs as i64) {
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        // resample
        normalize_and_accumulate(&mut weights);

        println!("{}", format!("Step {}: expanded all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| costs[i]);

        search_states = (0..num_particles).map(|_| {
            let idx = weighted_choice(&weights);
            search_states[idx].clone()
        }).collect();

        println!("{}", format!("Step {}: resampled all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| {
            compute_cost(&shared.egraph, root, &search_states[i], shared.check_slow)
        });

    }

    let (cost) = compute_cost(&shared.egraph, root, &best_so_far.as_ref().unwrap().1, shared.check_slow);
    println!("\n{}", "═══ RESULT ═══".green().bold());
    println!("{} {}", "best found at iteration:".dimmed(), best_found_at.unwrap().to_string().yellow());
    println!("{} {}", "pattern:".dimmed(), best_so_far.as_ref().unwrap().1.pattern.to_string().cyan().bold());
    println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
    println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / cost as f64).green().bold());
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

/// Prints summary info for the top particles (up to 5).
fn print_top_particles(
    states: &[SearchState],
    weights: &[f64],
    shared: &SharedSearchData,
    original_size: usize,
    get_cost: impl Fn(usize) -> usize,
) {
    for i in 0..min(5, states.len()) {
        let usage_matches: usize = states[i].matches.iter()
            .map(|m| shared.usage_counts.get(&m.root_eclass).copied().unwrap_or(1))
            .sum();
        let pat_size = compute_pattern_size(&states[i].pattern);
        let appx_cost = original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
        let cost_i = get_cost(i);
        let ratio = original_size as f64 / cost_i as f64;
        println!("  {} {}", format!("p{}:", i).dimmed(), states[i].pattern.to_string().cyan());
        println!("      cost={} ratio={:.2}x weight={:.4} matches={} usage_matches={} pat_size={} appx_cost={}", cost_i, ratio, weights[i], states[i].matches.len(), usage_matches, pat_size, appx_cost);
    }
}

pub fn compute_cost(
    egraph: &StitchEgraph,
    root: egg::Id,
    search_state: &SearchState,
    check_slow: bool,
) -> usize {
    let cost = compute_size(egraph, root, search_state, check_slow);
    let pattern_size = compute_pattern_size(&search_state.pattern);
    cost + pattern_size
}

pub fn compute_pattern_size(pattern: &Pattern) -> usize {
    1 + pattern.pattern.nodes.iter().map(|node| node.children().len()).sum::<usize>()
}

fn compute_size(
    egraph: &StitchEgraph,
    root: egg::Id,
    search_state: &SearchState,
    check_slow: bool,
) -> usize {
    let mut size_under_rewrite = FxHashMap::<Id, i64>::default();
    let mut work_queue = BinaryHeap::new();
    let mut eclass_to_matches = FxHashMap::<Id, &Vec<Subst>>::default();

    let get_size = |eclass: Id, s_u_r: &FxHashMap<Id, i64>| -> i64 {
        s_u_r.get(&eclass).cloned().unwrap_or(egraph[eclass].data as i64)
    };

    for m in &search_state.matches {
        work_queue.push(Reverse(m.root_eclass));
        eclass_to_matches.insert(m.root_eclass, &m.substs);
    }
    while let Some(Reverse(eclass)) = work_queue.pop() {
        // we assume that small numbers are children of large numbers, so when we pop we have already computed children
        if(size_under_rewrite.contains_key(&eclass)) {
            continue;
        }
        let size_current = get_size(eclass, &size_under_rewrite);
        let mut best = size_current;
        // trying a rewrite; (fn_i arg0 ...)
        if let Some(substs) = eclass_to_matches.get(&eclass) {
            for subst in *substs {
                let mut size_new: i64 = 1;
                for &var in &subst.vars {
                    size_new += get_size(var, &size_under_rewrite);
                }
                if size_new < best {
                    best = size_new;
                }
            }
        }
        // not doing a rewrite (just try all the enocdes)
        if let Some(enode) = egraph[eclass].nodes.first() {
            let mut size_no_rewrite: i64 = 1;
            for &child in &enode.children {
                size_no_rewrite += get_size(child, &size_under_rewrite);
            }
            if size_no_rewrite < best {
                best = size_no_rewrite;
            }
        }
        if best < size_current {
            for parent in egraph[eclass].parents() {
                work_queue.push(Reverse(parent));
            }
            size_under_rewrite.insert(eclass, best);
        }
    }
    let final_size = size_under_rewrite.get(&root).cloned().unwrap_or(egraph[root].data as i64);
    if check_slow {
        let slow_size = crate::rewrite_slow::rewrite_slow(egraph, root, search_state) as i64;
        assert_eq!(final_size, slow_size, "Fast rewrite size {} != slow rewrite size {}", final_size, slow_size);
    }
    final_size as usize
}

