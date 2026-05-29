use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchLanguage, StitchOp};
use crate::revexpr::RevExpr;
use crate::shift::{shift_db_disc, shift_extraction};
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
    pub var_depth: Vec<u32>, // var_depth[k] = pattern-internal binders enclosing ?#k (= min depth across occurrences after reuse)
    /// Syntactic occurrence count of `?#k`: how many times a walk from the
    /// root visits a node holding `Var(k)`. The pattern is a pure tree (no node
    /// sharing), so this equals `vars[k].len()`; it is still tracked explicitly
    /// because cost accounting reads it on the hot path. Maintained incrementally
    /// by `expand`/`reuse`.
    pub var_occurrences: Vec<usize>,
    /// True iff `?#k` is in the freshest cohort. `expand` flips all
    /// pre-existing vars to false and inserts new children as true; `reuse`
    /// flips `0..drop_idx` to false (including the kept slot). Search skips
    /// `Reuse(i, j)` only when *both* are false — that pair would re-merge
    /// cohorts a prior action already committed to (duplicate canonical).
    pub var_reusable: Vec<bool>,
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
            var_occurrences: vec![1],
            var_reusable: vec![true],
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
    ///
    /// When `?#var_idx` occupies multiple positions (from a prior `reuse`), every
    /// occurrence is expanded *independently*: each gets its own copy of the new
    /// enode and its own freshly-created child nodes. The pattern stays a pure
    /// tree — no node is shared between occurrences — so `vars[var_idx+j]` ends up
    /// with one id per occurrence rather than a single DAG-shared id.
    pub fn expand(&mut self, var_idx: usize, target: &F::Apply<O>) {
        // Per-occurrence structural depths, snapshotted before any mutation.
        let depths = self.occurrence_depths();
        let var_positions = self.vars.remove(var_idx);
        let parent_depth = self.var_depth.remove(var_idx);
        let parent_occ = self.var_occurrences.remove(var_idx);
        self.var_reusable.remove(var_idx);
        // Any expansion flips every *previously existing* var to non-reusable;
        // only the children we insert below start out reusable. See
        // `var_reusable` docs.
        for r in &mut self.var_reusable {
            *r = false;
        }
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

        // Insert the `num_children` new var slots (names var_idx..var_idx+k).
        // Positions are filled in below — one freshly-created node per occurrence
        // of the expanded var, since we never share nodes across occurrences.
        for j in 0..num_children {
            self.vars.insert(var_idx + j, Vec::with_capacity(var_positions.len()));
            let child_depth = parent_depth + if target_disc.binds_child(j) { 1 } else { 0 };
            self.var_depth.insert(var_idx + j, child_depth);
            // The new enode replaces every occurrence of the parent var, so the
            // syntactic walk visits each new child exactly `parent_occ` times.
            self.var_occurrences.insert(var_idx + j, parent_occ);
            self.var_reusable.insert(var_idx + j, true);
        }

        // Expand each occurrence of the var independently: build its own enode
        // with its own freshly-named Var children. No node is shared between
        // occurrences, so a reused var deepens every copy separately and the
        // pattern remains a pure tree. A DB-var leaf is shifted to each
        // occurrence's depth — a no-op unless it's a cross-depth shift-variant.
        for var_id in var_positions {
            let delta = depths[usize::from(var_id)] as i32 - parent_depth as i32;
            let disc = shift_db_disc::<F, O>(target_disc.clone(), delta);
            let mut new_children = Vec::with_capacity(num_children);
            for j in 0..num_children {
                self.pattern.nodes.push(var_node::<F, O>((var_idx + j) as u32));
                let new_id = Id::from(self.pattern.nodes.len() - 1);
                new_children.push(new_id);
                self.vars[var_idx + j].push(new_id);
            }
            let new_node = F::make(F::map_discriminant(disc, OpWithVar::Node), new_children);
            self.pattern[var_id] = new_node;
        }
    }

    /// Unifies two variables. The lower-indexed one is kept; the higher one is
    /// removed and its positions are rewritten to the kept var's name. Trailing
    /// vars shift left by one and have their leaves renamed accordingly. Args may
    /// be passed in either order.
    pub fn reuse(&mut self, var_idx: usize, second_var_idx: usize) {
        assert_ne!(var_idx, second_var_idx, "reuse requires two distinct vars");
        let (keep_idx, drop_idx) = if var_idx < second_var_idx { (var_idx, second_var_idx) } else { (second_var_idx, var_idx) };

        // Merged metavar adopts the *min* depth; deeper occurrences are
        // recovered by shifting concrete content to their depth in
        // `expand`/`concretize`.
        let merged_depth = self.var_depth[keep_idx].min(self.var_depth[drop_idx]);

        let keep_name = var_node::<F, O>(keep_idx as u32);
        for var_id in &self.vars[drop_idx] {
            self.pattern[*var_id] = keep_name.clone();
        }
        let drop_ids = self.vars[drop_idx].clone();
        self.vars[keep_idx].extend(drop_ids);
        self.vars.remove(drop_idx);
        self.var_depth.remove(drop_idx);
        self.var_depth[keep_idx] = merged_depth;
        let dropped_occ = self.var_occurrences.remove(drop_idx);
        self.var_occurrences[keep_idx] += dropped_occ;
        // Reusing (i, j) commits to a canonical order: any var strictly below
        // the *higher* of the two participating indices becomes non-reusable,
        // so future reuses must involve indices ≥ drop_idx (including the
        // kept slot itself, since we've moved past it).
        for r in &mut self.var_reusable[..drop_idx] {
            *r = false;
        }
        self.var_reusable.remove(drop_idx);

        // Shift names of trailing vars down by one.
        for p in drop_idx..self.vars.len() {
            let shifted = var_node::<F, O>(p as u32);
            for &id in &self.vars[p] {
                self.pattern[id] = shifted.clone();
            }
        }
    }

    /// Splices a fully-concrete subtree in for every occurrence of `?#var_idx`
    /// and removes the var slot. The subtree is `extraction`, a postorder node
    /// list with `root` at its last index (children referenced by lower
    /// indices). Soundness wrt enclosing pattern binders — i.e. that every DB
    /// index in `extraction` is bound at the splice site — is the caller's
    /// responsibility; the typical caller obtains `extraction` from a
    /// size-minimal eclass walk gated by a `fv < var_depth[var_idx]` check.
    ///
    /// Multi-position vars (from prior `reuse`) get an independent copy of the
    /// extraction per occurrence (pure tree — no sharing), free DB indices
    /// shifted up to that occurrence's depth (identity for same-depth ones).
    /// Trailing var names shift down by one to keep the canonical-form invariant.
    pub fn concretize(&mut self, var_idx: usize, extraction: &[F::Apply<OpWithVar<O>>], root: Id) {
        let depths = self.occurrence_depths();
        let ref_depth = self.var_depth[var_idx];
        let var_positions = self.vars.remove(var_idx);
        self.var_depth.remove(var_idx);
        self.var_occurrences.remove(var_idx);
        self.var_reusable.remove(var_idx);

        for p in var_idx..self.vars.len() {
            let shifted = var_node::<F, O>(p as u32);
            for &id in &self.vars[p] {
                self.pattern[id] = shifted.clone();
            }
        }

        debug_assert_eq!(usize::from(root), extraction.len() - 1, "concretize: root must be the last extraction node");
        for &var_id in &var_positions {
            let delta = depths[usize::from(var_id)] as i32 - ref_depth as i32;
            let (shifted, shifted_root) = shift_extraction::<F, O>(extraction, root, delta);
            self.splice_extraction_at(var_id, &shifted, shifted_root);
        }
    }

    /// Appends one copy of postorder `extraction` and writes its remapped root
    /// into `var_id` — one (shifted) copy per occurrence (no sharing).
    fn splice_extraction_at(&mut self, var_id: Id, extraction: &[F::Apply<OpWithVar<O>>], root: Id) {
        let n = extraction.len();
        debug_assert_eq!(usize::from(root), n - 1, "splice_extraction_at: root must be the last node");
        let base = self.pattern.nodes.len();
        // `remap` is only invoked when traversing child references; n == 1
        // means a single leaf root with no children, so the closure body
        // (which would underflow `n - 2`) is never reached.
        let remap = |c: Id| {
            let i = usize::from(c);
            debug_assert!(i + 1 < n, "splice_extraction_at: extraction child references must skip the root");
            Id::from(base + n - 2 - i)
        };
        for i in (0..n - 1).rev() {
            let mut clone = extraction[i].clone();
            for c in clone.children_mut() {
                *c = remap(*c);
            }
            self.pattern.nodes.push(clone);
        }
        let mut root_node = extraction[n - 1].clone();
        for c in root_node.children_mut() {
            *c = remap(*c);
        }
        self.pattern[var_id] = root_node;
    }

    /// Per-node structural binder depth: `depth[id]` = enclosing pattern binders.
    /// Parents-before-children walk (a `RevExpr` keeps parents below children);
    /// unambiguous since the pattern is a pure tree (one parent per id).
    fn occurrence_depths(&self) -> Vec<u32> {
        let nodes = &self.pattern.nodes;
        let mut depth = vec![0u32; nodes.len()];
        for i in 0..nodes.len() {
            let d = depth[i];
            let disc = nodes[i].discriminant();
            for (j, &c) in nodes[i].children().iter().enumerate() {
                depth[usize::from(c)] = d + if disc.binds_child(j) { 1 } else { 0 };
            }
        }
        depth
    }
}

impl<F: LanguageFamily, O: StitchOp> Pattern<F, O> {
    /// Builds the abstraction body with HO apps spliced in: each occurrence of
    /// `?#k` with non-empty `variable_indices[k]` is wrapped as
    /// `(@ … (@ ?#k $vis[h-1]) … $vis[0])`. Other positions copy through
    /// unchanged.
    pub fn build_with_ho(&self, variable_indices: &[Vec<i32>]) -> RecExpr<F::Apply<OpWithVar<O>>> {
        // RevExpr id → which metavar k (if any) lives at this position.
        let mut pos_to_k: FxHashMap<usize, usize> = FxHashMap::default();
        for (k, ids) in self.vars.iter().enumerate() {
            for &id in ids {
                pos_to_k.insert(usize::from(id), k);
            }
        }
        // Walk RevExpr from leaves (high indices) to root (index 0), copying
        // each node into a fresh RecExpr. Children get id-mapped; var positions
        // get HO-app-wrapped.
        let mut out: RecExpr<F::Apply<OpWithVar<O>>> = RecExpr::default();
        let mut id_map: Vec<Id> = vec![Id::from(0); self.pattern.nodes.len()];
        for i in (0..self.pattern.nodes.len()).rev() {
            let node = &self.pattern.nodes[i];
            let new_children: Vec<Id> = node.children().iter().map(|&c| id_map[usize::from(c)]).collect();
            let new_node = F::make(node.discriminant(), new_children);
            let mut new_id = out.add(new_node);
            if let Some(&k) = pos_to_k.get(&i)
                && !variable_indices[k].is_empty()
            {
                let vis = &variable_indices[k];
                let db_args: Vec<i32> = vis.iter().rev().copied().collect();
                new_id = F::wrap_pattern_with_db_apps::<O>(&mut out, new_id, &db_args);
            }
            id_map[i] = new_id;
        }
        out
    }

    /// String form of `build_with_ho`. Short-circuits to `to_string()` when no
    /// wrapping is needed.
    pub fn display_with_ho(&self, variable_indices: &[Vec<i32>]) -> String {
        if variable_indices.iter().all(|v| v.is_empty()) {
            return self.to_string();
        }
        <F::Apply<OpWithVar<O>> as StitchLanguage>::display_recexpr(&self.build_with_ho(variable_indices))
    }

    /// Render this abstraction as a closed lambda term — see
    /// `LanguageFamily::display_pattern_as_lambda`.
    pub fn display_as_lambda(&self, variable_indices: &[Vec<i32>]) -> String {
        F::display_pattern_as_lambda::<O>(&self.pattern.nodes, &self.vars, &self.var_depth, variable_indices)
    }
}

/// Recursively compare two pattern subtrees for structural equality. Walks from
/// the root rather than comparing the underlying `nodes` vec directly, because
/// vec layout depends on expansion order and is not canonical.
fn nodes_eq<F: LanguageFamily, O: StitchOp>(a: &PatternRecExpr<F, O>, b: &PatternRecExpr<F, O>, ai: Id, bi: Id) -> bool {
    let na = &a[ai];
    let nb = &b[bi];
    if na.discriminant() != nb.discriminant() {
        return false;
    }
    let ca = na.children();
    let cb = nb.children();
    ca.len() == cb.len() && ca.iter().zip(cb).all(|(&xa, &xb)| nodes_eq::<F, O>(a, b, xa, xb))
}

/// Recursively hash a pattern subtree by walking from the root, mirroring
/// `nodes_eq` so equal patterns hash identically regardless of vec layout.
fn hash_node<F: LanguageFamily, O: StitchOp, H: std::hash::Hasher>(expr: &PatternRecExpr<F, O>, id: Id, state: &mut H) {
    use std::hash::Hash;
    let n = &expr[id];
    n.discriminant().hash(state);
    for &c in n.children() {
        hash_node::<F, O, H>(expr, c, state);
    }
}

/// The underlying vec layout depends on expansion order and is not canonical.
/// These impls recurse from the root (Id(0)) using canonical var names instead.
impl<F: LanguageFamily, O: StitchOp> PartialEq for Pattern<F, O> {
    fn eq(&self, other: &Self) -> bool {
        nodes_eq::<F, O>(&self.pattern, &other.pattern, Id::from(0), Id::from(0))
    }
}

impl<F: LanguageFamily, O: StitchOp> Eq for Pattern<F, O> {}

impl<F: LanguageFamily, O: StitchOp> std::hash::Hash for Pattern<F, O> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        hash_node::<F, O, H>(&self.pattern, Id::from(0), state);
    }
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
    fn expand_reused_var_duplicates_subtree() {
        let mut p: Pattern<OpChildren, Op> = Pattern::single_var();
        p.expand(0, &op("+", 2)); // (+ ?#0 ?#1)
        p.reuse(0, 1); // (+ ?#0 ?#0)
        assert_eq!(p.vars.len(), 1);
        p.expand(0, &op("*", 2)); // (+ (* ?#0 ?#1) (* ?#0 ?#1))
        assert_eq!(p.to_string(), "(+ (* ?#0 ?#1) (* ?#0 ?#1))");
        assert_eq!(p.vars.len(), 2);
        assert_vars_canonical(&p);

        // Each occurrence of the expanded var got its own copy of the new
        // subtree, so each new var now has one RecExpr slot per tree occurrence
        // (no DAG sharing).
        assert_eq!(p.vars[0].len(), 2);
        assert_eq!(p.vars[1].len(), 2);
        // The new ids must be distinct nodes, not a single shared one.
        assert_ne!(p.vars[0][0], p.vars[0][1]);
        assert_ne!(p.vars[1][0], p.vars[1][1]);
        // Syntactic occurrence count matches the number of positions (2 each).
        assert_eq!(p.var_occurrences, vec![2, 2]);
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
