//! Canonical-pattern pruning for best-first search.
//!
//! Given a candidate pattern P (a `RecExpr` in `F::Apply<OpWithVar<O>>`), build a
//! fresh egraph in that language, insert P, saturate with the user's rewrite
//! rules, and extract a *canonical* min-cost representative of the root eclass.
//! If the extraction equals P, P is canonical; otherwise some equivalent pattern
//! has been (or will be) explored elsewhere and we prune.
//!
//! "Canonical" here means: among min-cost enodes of each eclass, pick the
//! lex-smallest by recursive serialization. Both the egraph's representative
//! and P itself are serialized through the same scheme so the comparison is
//! syntactic on identical strings.
//!
//! No rule file → no equivalences → every pattern is canonical → the check is a
//! cheap no-op.

use crate::lang::{LanguageFamily, OpWithVar, StitchAnalysis, StitchDisc, StitchLanguage, StitchOp, Weights};
use egg::{EGraph, Id, RecExpr, Rewrite, Runner};
use rustc_hash::FxHashMap;

/// Pattern-side language: program ops extended with metavariable leaves.
pub type PatternLang<F, O> = <F as LanguageFamily>::Apply<OpWithVar<O>>;
pub type PatternRules<F, O> = Vec<Rewrite<PatternLang<F, O>, StitchAnalysis>>;

/// Holds the rewrite rules (parsed against the pattern language) and a memo
/// keyed by pattern serialization, so repeated check on the same pattern is O(1).
pub struct CanonicalChecker<F: LanguageFamily, O: StitchOp> {
    rules: PatternRules<F, O>,
    weights: Weights,
    memo: FxHashMap<String, bool>,
    /// Number of times the check returned false (i.e. a candidate was pruned).
    pub pruned: usize,
    /// Memo hits (recomputed avoidance count).
    pub memo_hits: usize,
}

impl<F: LanguageFamily, O: StitchOp> CanonicalChecker<F, O> {
    pub fn new(rules: PatternRules<F, O>, weights: Weights) -> Self {
        Self { rules, weights, memo: FxHashMap::default(), pruned: 0, memo_hits: 0 }
    }

    /// True when the user supplied no rule file: every pattern is trivially canonical.
    pub fn trivial(&self) -> bool {
        self.rules.is_empty()
    }

    /// True iff `pattern` equals the canonical extraction of its root eclass
    /// after rule saturation.
    pub fn is_canonical(&mut self, pattern: &RecExpr<PatternLang<F, O>>) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let key = serialize_recexpr::<PatternLang<F, O>>(pattern);
        if let Some(&v) = self.memo.get(&key) {
            self.memo_hits += 1;
            return v;
        }
        let canonical = canonical_string::<F, O>(pattern, &self.rules, self.weights);
        let result = canonical == key;
        if !result {
            self.pruned += 1;
        }
        self.memo.insert(key, result);
        result
    }
}

/// Build a fresh egraph, add `pattern`, saturate `rules`, and return the
/// lex-min canonical serialization of the resulting root eclass.
fn canonical_string<F: LanguageFamily, O: StitchOp>(pattern: &RecExpr<PatternLang<F, O>>, rules: &PatternRules<F, O>, weights: Weights) -> String {
    let mut egraph: EGraph<PatternLang<F, O>, StitchAnalysis> = EGraph::new(StitchAnalysis::new(weights));
    let root = egraph.add_expr(pattern);
    egraph.rebuild();
    let mut runner: Runner<PatternLang<F, O>, StitchAnalysis> = Runner::new(StitchAnalysis::new(weights)).with_egraph(egraph).with_iter_limit(10);
    runner = runner.run(rules);
    runner.egraph.rebuild();
    let root = runner.egraph.find(root);
    let cost = compute_min_costs::<PatternLang<F, O>>(&runner.egraph, &weights);
    let strings = build_class_strings::<PatternLang<F, O>>(&runner.egraph, &weights, &cost);
    strings.get(&root).expect("root eclass has no canonical string").clone()
}

/// Tree-walk a `RecExpr` from its last (root) node and produce the same
/// "(disc child1 child2 ...)" serialization used by `build_class_strings`. The
/// vec layout is irrelevant — we always walk children via id, so DAG-shared
/// subtrees are unfolded into the resulting tree string just as the egraph
/// extraction would.
fn serialize_recexpr<L: StitchLanguage>(expr: &RecExpr<L>) -> String {
    let nodes = expr.as_ref();
    let root = Id::from(nodes.len() - 1);
    fn walk<L: StitchLanguage>(nodes: &[L], id: Id, out: &mut String) {
        let n = &nodes[usize::from(id)];
        if n.children().is_empty() {
            out.push_str(&format!("{}", n.discriminant()));
        } else {
            out.push('(');
            out.push_str(&format!("{}", n.discriminant()));
            for &c in n.children() {
                out.push(' ');
                walk(nodes, c, out);
            }
            out.push(')');
        }
    }
    let mut s = String::new();
    walk::<L>(nodes, root, &mut s);
    s
}

/// Standard fixed-point cost DP: per-eclass min weighted size over enodes whose
/// children all have costs. Eclasses reachable only via cycles get no entry.
fn compute_min_costs<L: StitchLanguage>(egraph: &EGraph<L, StitchAnalysis>, weights: &Weights) -> FxHashMap<Id, u64> {
    let mut cost: FxHashMap<Id, u64> = FxHashMap::default();
    loop {
        let mut changed = false;
        for ec in egraph.classes() {
            let id = egraph.find(ec.id);
            let best = ec.nodes.iter().filter_map(|n| node_cost::<L>(n, weights, &cost, egraph)).min();
            if let Some(b) = best {
                match cost.get(&id) {
                    Some(&prev) if prev <= b => {}
                    _ => {
                        cost.insert(id, b);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    cost
}

fn node_cost<L: StitchLanguage>(n: &L, weights: &Weights, cost: &FxHashMap<Id, u64>, egraph: &EGraph<L, StitchAnalysis>) -> Option<u64> {
    let mut total = n.discriminant().intrinsic_size(weights) as u64;
    for &c in n.children() {
        let c = egraph.find(c);
        total = total.checked_add(*cost.get(&c)?)?;
    }
    Some(total)
}

/// For each eclass that has a min cost, compute its lex-smallest serialization
/// "(disc child1-str child2-str ...)" among min-cost enodes whose children are
/// all serialized. Fixed-point: a parent's serialization may improve once a
/// child eclass first gets a string in a cyclic egraph.
fn build_class_strings<L: StitchLanguage>(egraph: &EGraph<L, StitchAnalysis>, weights: &Weights, cost: &FxHashMap<Id, u64>) -> FxHashMap<Id, String> {
    let mut memo: FxHashMap<Id, String> = FxHashMap::default();
    loop {
        let mut changed = false;
        for ec in egraph.classes() {
            let id = egraph.find(ec.id);
            let Some(&target) = cost.get(&id) else { continue };
            let mut best: Option<String> = None;
            for n in &ec.nodes {
                if node_cost::<L>(n, weights, cost, egraph) != Some(target) {
                    continue;
                }
                let mut child_strs: Vec<&String> = Vec::with_capacity(n.children().len());
                let mut ok = true;
                for &c in n.children() {
                    let c = egraph.find(c);
                    match memo.get(&c) {
                        Some(s) => child_strs.push(s),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let s = if n.children().is_empty() {
                    format!("{}", n.discriminant())
                } else {
                    let mut s = String::with_capacity(16);
                    s.push('(');
                    s.push_str(&format!("{}", n.discriminant()));
                    for p in child_strs {
                        s.push(' ');
                        s.push_str(p);
                    }
                    s.push(')');
                    s
                };
                match &best {
                    None => best = Some(s),
                    Some(cur) if &s < cur => best = Some(s),
                    _ => {}
                }
            }
            if let Some(s) = best
                && memo.get(&id) != Some(&s)
            {
                memo.insert(id, s);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    memo
}

