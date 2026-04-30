use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use crate::revexpr::RevExpr;
use egg::{Id, Language, RecExpr};
use rustc_hash::FxHashMap;

/// A partially-built pattern, parameterized by a language family `F` (the
/// type-level constructor `L<_>`) and a leaf-Op `O` for the program side.
///
/// Storage is `RecExpr<F::Apply<OpWithVar<O>>>` — i.e. exactly the program
/// language `F::Apply<O>` reinstantiated with `OpWithVar<O>` as its leaf-Op,
/// so a pattern is just "the same Language as programs, with pattern variables
/// added to the Op slot."
///
/// Canonical-form invariant: for every `k`, every `Id` in `vars[k]` holds a
/// node whose op is `OpWithVar::Var(egg::Var::from(k as u32))` — so the tree's
/// var names match their DFS first-appearance order. `expand` and `reuse`
/// preserve this by rewriting affected var leaves, so `pattern.to_string()`
/// is canonical: alpha-equivalent patterns render identically.
/// The storage type backing a `Pattern<F, O>`: the program language
/// `F::Apply<O>` with `OpWithVar<O>` swapped in as its leaf-Op.
pub type PatternRecExpr<F, O> = RevExpr<<F as LanguageFamily>::Apply<OpWithVar<O>>>;

#[derive(Debug, Clone)]
pub struct Pattern<F: LanguageFamily, O: StitchOp> {
    pub pattern: PatternRecExpr<F, O>,
    pub vars: Vec<Vec<Id>>,  // vars[k] = all RecExpr ids holding Var(k)
    pub var_depth: Vec<u32>, // var_depth[k] = pattern-internal binders enclosing ?#k
}

fn var_node<F: LanguageFamily, O: StitchOp>(idx: u32) -> F::Apply<OpWithVar<O>> {
    F::make_var(egg::Var::from(idx))
}

impl<F: LanguageFamily, O: StitchOp> Pattern<F, O> {
    /// Creates the initial `?#0` pattern: a single variable at depth 0.
    pub fn single_var() -> Self {
        Pattern {
            pattern: RevExpr::new(vec![var_node::<F, O>(0)]),
            vars: vec![vec![0.into()]],
            var_depth: vec![0],
        }
    }

    /// Expands the variable at `var_idx` with `target`. New children are inserted
    /// at list positions `var_idx..var_idx+k`; any vars that previously followed
    /// `var_idx` shift right and get their in-tree `Var(n)` leaves rewritten to
    /// match their new position, so the canonical-form invariant is preserved.
    ///
    /// Each new child meta-var inherits the parent's binder depth, plus one if
    /// `target.discriminant().binds_child(j)` is true for that slot — i.e., a
    /// `Lam` body bumps the depth of the meta-var that lands inside it.
    pub fn expand(&mut self, var_idx: usize, target: &F::Apply<O>) {
        let var_positions = self.vars.remove(var_idx);
        let parent_depth = self.var_depth.remove(var_idx);
        assert!(self.pattern[var_positions[0]].discriminant().as_var().is_some(), "Attempting to expand a non-var");
        let num_children = target.len();
        let target_disc = target.discriminant();

        // Shift names of trailing vars: a var currently at post-removal index p
        // will end up at post-insertion index p + num_children, so rename its leaves.
        // (Skip the no-op case num_children == 1 where indices don't move.)
        if num_children != 1 {
            for p in var_idx..self.vars.len() {
                let shifted = var_node::<F, O>((p + num_children) as u32);
                for &id in &self.vars[p] {
                    self.pattern[id] = shifted.clone();
                }
            }
        }

        // Build the new enode with freshly-named Var children at positions var_idx..var_idx+k.
        let mut new_children = Vec::with_capacity(num_children);
        for j in 0..num_children {
            self.pattern.nodes.push(var_node::<F, O>((var_idx + j) as u32));
            let new_id = Id::from(self.pattern.nodes.len() - 1);
            new_children.push(new_id);
            self.vars.insert(var_idx + j, vec![new_id]);
            let child_depth = parent_depth + if target_disc.binds_child(j) { 1 } else { 0 };
            self.var_depth.insert(var_idx + j, child_depth);
        }
        let new_node = F::make(F::map_discriminant(target_disc, OpWithVar::Node), new_children);

        // Replace each position of the expanded var with the new enode. If the var
        // had multiple positions (from a prior reuse), all parents share the same
        // children via the RecExpr DAG.
        for var_id in var_positions {
            self.pattern[var_id] = new_node.clone();
        }
    }

    /// Unifies two variables. The lower-indexed one is kept; the higher one is
    /// removed and its positions are rewritten to the kept var's name. Trailing
    /// vars shift left by one and have their leaves renamed accordingly. Args may
    /// be passed in either order.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        assert_ne!(var_idx, second_var_idx, "reuse requires two distinct vars");
        let (keep_idx, drop_idx) = if var_idx < second_var_idx { (var_idx, second_var_idx) } else { (second_var_idx, var_idx) };

        // Scope invariant: only meta-vars at the same binder depth may be unified.
        // Mixing depths would mean one occurrence sits under more pattern-internal
        // binders than the other, so substitution would need a binder-shift —
        // outside the canonical-form contract.
        assert_eq!(self.var_depth[keep_idx], self.var_depth[drop_idx], "reuse across differing binder depths is not allowed (depths {} vs {})", self.var_depth[keep_idx], self.var_depth[drop_idx]);

        let keep_name = var_node::<F, O>(keep_idx as u32);
        for var_id in &self.vars[drop_idx] {
            self.pattern[*var_id] = keep_name.clone();
        }
        let drop_ids = self.vars[drop_idx].clone();
        self.vars[keep_idx].extend(drop_ids);
        self.vars.remove(drop_idx);
        self.var_depth.remove(drop_idx);

        // Shift names of trailing vars down by one.
        for p in drop_idx..self.vars.len() {
            let shifted = var_node::<F, O>(p as u32);
            for &id in &self.vars[p] {
                self.pattern[id] = shifted.clone();
            }
        }
    }

    /// Build the abstraction body in stitch λ-form: every `?#k` leaf is
    /// replaced by `(?#k $h_0 … $h_{m-1} $(d_k-1) … $0)` — `?#k` curry-applied
    /// to each hoisted-index var (in `hoists[k]` order, smallest to largest)
    /// followed by the pattern-internal binder vars from outermost to
    /// innermost.
    ///
    /// Pattern-internal binder refs use the local depth at the leaf position
    /// in the pattern AST, which equals `var_depth[k]` for `?#k` (every
    /// occurrence sits under exactly that many lams). Hoisted-index refs use
    /// post-pattern-wrap-frame indices shifted to the local frame: a hoist of
    /// `h_post` becomes `$(h_post + var_depth[k])` at the leaf position.
    ///
    /// `hoists[k]` is sorted ascending. The result is a fresh `RecExpr` with
    /// no aliasing back into `self.pattern`.
    pub fn body_with_hoists(&self, hoists: &[Vec<u32>]) -> RecExpr<F::Apply<OpWithVar<O>>> {
        assert_eq!(hoists.len(), self.var_depth.len(), "hoists length must match metavar count");
        let src: RecExpr<F::Apply<OpWithVar<O>>> = self.pattern.clone().into();
        let src_root = (src.as_ref().len() - 1).into();
        let mut out: RecExpr<F::Apply<OpWithVar<O>>> = RecExpr::default();
        let mut memo: FxHashMap<Id, Id> = FxHashMap::default();
        self.walk_body_with_hoists(&src, src_root, &mut out, &mut memo, hoists);
        out
    }

    fn walk_body_with_hoists(&self, src: &RecExpr<F::Apply<OpWithVar<O>>>, id: Id, out: &mut RecExpr<F::Apply<OpWithVar<O>>>, memo: &mut FxHashMap<Id, Id>, hoists: &[Vec<u32>]) -> Id {
        if let Some(&hit) = memo.get(&id) {
            return hit;
        }
        let node = &src.as_ref()[usize::from(id)];
        let new_id = if let Some(v) = node.discriminant().as_var() {
            // Recover k from `?#k`. Pattern's canonical-form invariant: `vars[k]`
            // contains every Id holding `Var(k)`, which is what `as_var()`
            // returns here.
            let k = parse_meta_var_index(&v);
            let d_k = self.var_depth[k];
            let head_id = out.add(F::make_var::<O>(v));
            // Build the curry-app argument list: hoist indices first
            // (each shifted by d_k to land in the local frame), then
            // pattern-internal binder vars `$(d_k-1) … $0`.
            let mut arg_indices: Vec<u32> = hoists[k].iter().map(|&h| h + d_k).collect();
            for j in (0..d_k).rev() {
                arg_indices.push(j);
            }
            F::apply_to_db_vars::<O>(out, head_id, &arg_indices)
        } else {
            let new_kids: Vec<Id> = node.children().iter().map(|&c| self.walk_body_with_hoists(src, c, out, memo, hoists)).collect();
            let mut new_node = node.clone();
            for (slot, kid) in new_node.children_mut().iter_mut().zip(new_kids.iter()) {
                *slot = *kid;
            }
            out.add(new_node)
        };
        memo.insert(id, new_id);
        new_id
    }
}

/// Recover `k` from a meta-var built by `single_var` / `expand` (which use
/// `egg::Var::from(k as u32)`, displayed as `?#k`).
fn parse_meta_var_index(v: &egg::Var) -> usize {
    let s = v.to_string();
    s.strip_prefix("?#").and_then(|t| t.parse().ok()).unwrap_or_else(|| panic!("meta-var should be `?#k`, got {s:?}"))
}

impl<F: LanguageFamily, O: StitchOp> std::fmt::Display for Pattern<F, O> {
    /// Routes through `StitchLanguage::display_recexpr` so language-specific
    /// pretty-printers (e.g. unappify) take effect on Pattern displays.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let recexpr: egg::RecExpr<F::Apply<OpWithVar<O>>> = self.pattern.clone().into();
        write!(f, "{}", <F::Apply<OpWithVar<O>> as crate::lang::StitchLanguage>::display_recexpr(&recexpr))
    }
}

#[cfg(test)]
mod tests {
    use crate::lang::{Op, OpChildren, OpChildrenLanguage};

    use super::*;
    use egg::Symbol;

    /// Build an enode with `arity` placeholder children. `expand` overwrites the
    /// children, so the dummy Ids here are never read.
    fn op(name: &str, arity: usize) -> OpChildrenLanguage {
        OpChildrenLanguage {
            op: Op::Sym(Symbol::from(name)),
            children: vec![Id::from(0); arity],
        }
    }

    /// Asserts the canonical-form invariant: every id in `vars[k]` holds `Var(k)`,
    /// and nothing in `vars` is non-Var.
    fn assert_vars_canonical(p: &Pattern<OpChildren, Op>) {
        for (k, ids) in p.vars.iter().enumerate() {
            let expected = egg::Var::from(k as u32);
            for id in ids {
                match p.pattern[*id].discriminant().as_var() {
                    Some(v) => assert_eq!(v, expected, "vars[{}] = {:?}: expected {:?}, got {:?}", k, ids, expected, v),
                    None => panic!("vars[{}] contains non-Var: {:?}", k, p.pattern[*id].discriminant()),
                }
            }
        }
    }

    #[test]
    fn single_var_is_canonical() {
        let p: Pattern<OpChildren, Op> = Pattern::single_var();
        assert_eq!(p.vars.len(), 1);
        assert_eq!(p.to_string(), "?#0");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_fresh_var_binary() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2));
        assert_eq!(p.vars.len(), 2);
        assert_eq!(p.to_string(), "(+ ?#0 ?#1)");
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_nested_left_first() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(0, &op("-", 2)); // (+ (- ?#0 ?#1) ?#2)
        assert_eq!(p.to_string(), "(+ (- ?#0 ?#1) ?#2)");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_right_keeps_earlier_vars_first() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#2))");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_ternary() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("f", 3));
        assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#2)");
        assert_eq!(p.vars.len(), 3);
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_adjacent() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.reuse(0, 1); // (+ ?#0 ?#0)
        assert_eq!(p.to_string(), "(+ ?#0 ?#0)");
        assert_eq!(p.vars.len(), 1);
        assert_vars_canonical(&p);
    }

    #[test]
    fn reuse_normalizes_reversed_args() {
        let mut p1: Pattern<OpChildren, Op> = Pattern::single_var();
        p1.expand(0, &op("+", 2));
        p1.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        p1.reuse(0, 2);

        let mut p2: Pattern<OpChildren, Op> = Pattern::single_var();
        p2.expand(0, &op("+", 2));
        p2.expand(1, &op("*", 2));
        p2.reuse(2, 0); // reversed

        assert_eq!(p1.to_string(), "(+ ?#0 (* ?#1 ?#0))");
        assert_eq!(p1.to_string(), p2.to_string());
        assert_eq!(p1.vars.len(), p2.vars.len());
        assert_vars_canonical(&p1);
        assert_vars_canonical(&p2);

        // Downstream expansion should agree: "var 0" must mean the same thing in both.
        p1.expand(0, &op("h", 1));
        p2.expand(0, &op("h", 1));
        assert_eq!(p1.to_string(), p2.to_string());
        assert_vars_canonical(&p1);
        assert_vars_canonical(&p2);
    }

    #[test]
    fn reuse_with_intervening_var() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("f", 3)); // (f ?#0 ?#1 ?#2)
        p.reuse(0, 2); // (f ?#0 ?#1 ?#0)
        assert_eq!(p.to_string(), "(f ?#0 ?#1 ?#0)");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
    }

    #[test]
    fn expand_reused_var_preserves_dag_sharing() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.reuse(0, 1); // (+ ?#0 ?#0)
        assert_eq!(p.vars.len(), 1);
        p.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))
        assert_eq!(p.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);

        // The two new vars must each have a single RecExpr slot (DAG sharing),
        // not one per tree occurrence.
        assert_eq!(p.vars[0].len(), 1);
        assert_eq!(p.vars[1].len(), 1);
    }

    #[test]
    fn expand_then_reuse_across_structure() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.expand(1, &op("*", 2)); // (+ ?#0 (* ?#1 ?#2))
        p.reuse(1, 2); // (+ ?#0 (* ?#1 ?#1))
        assert_eq!(p.to_string(), "(+ ?#0 (* ?#1 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);
    }

    #[test]
    fn to_string_distinguishes_non_equivalent_shapes() {
        let mut a: Pattern<OpChildren, Op> = Pattern::single_var();
        a.expand(0, &op("+", 2));
        a.reuse(0, 1); // (+ ?#0 ?#0)
        a.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))

        let mut b: Pattern<OpChildren, Op> = Pattern::single_var();
        b.expand(0, &op("+", 2));
        b.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) ?#2)
        b.expand(2, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#2 ?#3))

        assert_ne!(a.to_string(), b.to_string());
        assert_eq!(a.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(b.to_string(), "(+ (* ?#0 ?#1) (* ?#2 ?#3))");
        assert_vars_canonical(&a);
        assert_vars_canonical(&b);
    }
}
