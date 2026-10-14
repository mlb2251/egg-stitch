use egg::{Id, Language, RecExpr};

use super::{LambdaCalcDisc, LambdaCalcLanguage, OpChildrenLanguage, OpWithVar, StitchDisc, StitchEgraph, StitchLanguage, StitchOp, Weights};

/// A type-level type constructor `L<_>` for a language family.
///
/// Rust has no first-class higher-kinded types, so we simulate "the same
/// language `L<_>` instantiated with a different leaf-Op type" with this trait
/// plus GATs: `F::Apply<O>` is the spelling of `L<O>`. Programs are built over
/// `F::Apply<O>`; the corresponding pattern AST is built over
/// `F::Apply<OpWithVar<O>>`. Both share the same `F` (the language constructor),
/// and only the leaf-Op type differs.
///
/// `Discriminant<O>` is the discriminant of `Apply<O>`. Often it's just `O`
/// (`OpChildrenLanguage`), but languages with structural variants beyond a single
/// leaf-op slot can use a wrapper sum so the discriminant carries the variant tag.
///
/// Cost weights are runtime values (`Weights`) carried on `StitchAnalysis`, so
/// they no longer parameterize this trait.
pub trait LanguageFamily: Clone + 'static {
    /// Discriminant type for `Apply<O>`. A `StitchOp`: besides hash/eq/size/var
    /// detection, it can be built from a name and from a De Bruijn index, which
    /// lets language-agnostic code construct representative leaves.
    type Discriminant<O: StitchOp>: StitchOp;

    /// The Language obtained by instantiating this family with leaf-Op `O`.
    type Apply<O: StitchOp>: StitchLanguage<Discriminant = Self::Discriminant<O>>;

    /// Construct an enode from a discriminant op and a list of children. For
    /// families with fixed-arity structural variants, this dispatches on the
    /// variant + arity.
    fn make<P: StitchOp>(op: Self::Discriminant<P>, kids: Vec<Id>) -> Self::Apply<P>;

    /// Functor map over the leaf-Op slot of the discriminant. Structural
    /// variants pass through unchanged; embedded leaves go through `f`.
    /// Lifting a program-side discriminant into the pattern-side one is just
    /// `map_discriminant(op, OpWithVar::Node)`.
    fn map_discriminant<A: StitchOp, B: StitchOp>(op: Self::Discriminant<A>, f: impl FnMut(A) -> B) -> Self::Discriminant<B>;

    /// Add a `name(children...)` application to the egraph and return its Id.
    /// For families with binary `App` this builds a curried application chain.
    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;

    /// Structural cost (sum of node costs over all enodes added by
    /// `add_stub_application`) of an `arity`-arg stub application — the
    /// head plus any spine nodes (e.g. curried `App`s) the family inserts.
    fn stub_application_size(arity: usize, weights: &Weights) -> u32;

    /// Cost of a single symbol (leaf) in this family — the unit that the
    /// `--max-forced-expansion` slack is denominated in, so a cap of `k` means
    /// "`k` symbols' worth of forced expansion".
    fn symbol_cost(weights: &Weights) -> u32;

    /// Build a pattern leaf containing the given pattern variable.
    fn make_var<O: StitchOp>(v: egg::Var) -> Self::Apply<OpWithVar<O>>;

    /// Validate the fast-path cost against the slow-path (rebuilt-egraph)
    /// cost for this family. `OpChildren` requires strict equality;
    /// `LambdaCalc` allows `fast >= slow` because the fast path's wrap
    /// accounting is an upper bound — it can miss cases where a wrapped
    /// operand collides with another existing eclass that already has a
    /// cheaper rewrite.
    fn check_fast_vs_slow(fast: i64, slow: i64);

    /// Wrap an eclass in `n` lambda binders, returning the new eclass id.
    fn wrap_lams<O: StitchOp>(child: Id, n: u32, egraph: &mut StitchEgraph<Self::Apply<O>>) -> Id;

    /// Total node-count cost of `n` stacked lambda binders under `weights`.
    fn lams_cost(n: u32, weights: &Weights) -> u32;

    /// In a pattern-side `RecExpr`, wrap `head` in `n` curried applications to
    /// fresh DB-var leaves `$(n-1), $(n-2), …, $0` (outer-local first). Returns
    /// the id of the outermost App. Used by `Pattern::display_with_ho` to render
    /// HO body uses as `(@ … (@ ?#k $(n-1)) … $0)`.
    ///
    /// The reverse iteration order is what makes the corresponding η-wrap built
    /// by `wrap_subst_args` β-reduce back to the captured term without any
    /// per-index reflection: the first-applied arg `$(n-1)` binds the outermost
    /// wrap-lam, so a captured reference to local-$i lands at de Bruijn `$i`
    /// inside the body. For `n ≤ 1` the order is irrelevant; for `n ≥ 2` it
    /// matters (see `data/domains/ho-bugs/arity2_capture.json`).
    fn wrap_pattern_with_db_apps<O: StitchOp>(recexpr: &mut egg::RecExpr<Self::Apply<OpWithVar<O>>>, head: Id, db_args: &[i32]) -> Id;

    /// Inverse of [`wrap_pattern_with_db_apps`]: if `recexpr[id]` is the η-wrap
    /// shape this language renders for `?#k` with `ho_arity > 0` — `db_args` in
    /// the same `(n-1, n-2, …, 0)` reverse order produced by the wrapper — peel
    /// it off and return the inner head id. Returns `id` unchanged when the
    /// subtree isn't an η-wrap. Used by follow-mode matching to collapse the
    /// optimiser's display-vi choice (bare `?#k` vs `(?#k $0 …)`) so the check
    /// doesn't care which candidate the displayer happened to select.
    ///
    /// Default treats every subtree as un-wrapped; languages whose
    /// `wrap_pattern_with_db_apps` is a no-op (no binders → no HO display) can
    /// inherit it directly.
    fn unwrap_pattern_db_apps<O: StitchOp>(_nodes: &[Self::Apply<OpWithVar<O>>], id: Id) -> Id {
        id
    }

    /// Wrap `body` in the binders for an `n`-ary abstraction (`n > 0`), appending
    /// them to `recexpr`, and return the id of the outermost binder.
    fn wrap_pattern_lams<O: StitchOp>(recexpr: &mut RecExpr<Self::Apply<OpWithVar<O>>>, body: Id, n: u32) -> Id;

    /// Render an abstraction body as a closed lambda term with `vars.len()`
    /// binders, in this family's binder shape. Each `?#k` becomes the de Bruijn
    /// variable for the `k`-th outer binder (shifted by its local binder depth),
    /// applied to its captured indices if the slot is higher-order. Inlining a
    /// call site `(fn_N a_0 … a_{n-1})` against the result and β-reducing
    /// recovers the original captured term.
    fn display_pattern_as_lambda<O: StitchOp>(nodes: &[Self::Apply<OpWithVar<O>>], vars: &[Vec<Id>], var_depth: &[u32], variable_indices: &[Vec<i32>]) -> String {
        let arity = vars.len();
        let (mut out, body) = crate::pattern::rebuild_pattern::<Self, O>(nodes, vars, var_depth, variable_indices, |k, depth| {
            let idx = (arity - 1 - k) as i32 + depth as i32;
            // Leaf ops without a DB-var variant (plain `Op`) fall back to a
            // `$n` symbol, which renders identically.
            let disc = StitchOp::make_db_var(idx).unwrap_or_else(|| StitchOp::from_name(&format!("${idx}")));
            Some(Self::make(disc, vec![]))
        });
        if arity > 0 {
            Self::wrap_pattern_lams::<O>(&mut out, body, arity as u32);
        }
        <Self::Apply<OpWithVar<O>> as StitchLanguage>::display_recexpr(&out)
    }

    /// Parse a `--follow` pattern string into the internal pattern shape.
    /// Defaults to egg's stock `RecExpr` parser, which is fine for any flat
    /// language whose `from_op` accepts arbitrary-arity nodes (e.g.
    /// `OpChildren`). Languages with shape constraints (e.g. lambda-calc's
    /// curried `App`) or that display var-headed forms `(?#k a b c)` must
    /// override.
    fn parse_follow_pattern<O: StitchOp>(s: &str) -> anyhow::Result<crate::revexpr::RevExpr<Self::Apply<OpWithVar<O>>>> {
        let expr: egg::RecExpr<Self::Apply<OpWithVar<O>>> = s.parse().map_err(|e| anyhow::anyhow!("parse follow {s:?}: {e:?}"))?;
        Ok(expr.into())
    }
}

/// Marker for the `OpChildrenLanguage<_>` family.
#[derive(Clone, Copy, Debug)]
pub struct OpChildren;

impl LanguageFamily for OpChildren {
    type Discriminant<O: StitchOp> = O;
    type Apply<O: StitchOp> = OpChildrenLanguage<O>;

    fn make<P: StitchOp>(op: P, kids: Vec<Id>) -> OpChildrenLanguage<P> {
        OpChildrenLanguage { op, children: kids }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: A, mut f: impl FnMut(A) -> B) -> B {
        f(op)
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<OpChildrenLanguage<O>>) -> Id {
        egraph.add(Self::make(O::from_name(name), children))
    }

    fn stub_application_size(_arity: usize, weights: &Weights) -> u32 {
        // OpChildren has no application spine: a stub is a single leaf node.
        weights.sym_var_cost
    }

    fn symbol_cost(weights: &Weights) -> u32 {
        weights.sym_var_cost
    }

    fn check_fast_vs_slow(fast: i64, slow: i64) {
        assert_eq!(fast, slow, "Fast rewrite size {} != slow rewrite size {} (OpChildren)", fast, slow);
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> OpChildrenLanguage<OpWithVar<O>> {
        Self::make(OpWithVar::Var(v), vec![])
    }

    fn wrap_lams<O: StitchOp>(_child: Id, _n: u32, _egraph: &mut StitchEgraph<OpChildrenLanguage<O>>) -> Id {
        panic!("OpChildren has no lambda binders; higher-order capture is unreachable here");
    }

    fn lams_cost(_n: u32, _weights: &Weights) -> u32 {
        panic!("OpChildren has no lambda binders; higher-order capture is unreachable here");
    }

    fn wrap_pattern_with_db_apps<O: StitchOp>(_recexpr: &mut egg::RecExpr<OpChildrenLanguage<OpWithVar<O>>>, _head: Id, _db_args: &[i32]) -> Id {
        panic!("OpChildren has no apps/binders; higher-order display is unreachable here");
    }

    fn wrap_pattern_lams<O: StitchOp>(recexpr: &mut RecExpr<OpChildrenLanguage<OpWithVar<O>>>, body: Id, n: u32) -> Id {
        // No real binders: the body is wrapped in `n` `lam`-headed symbol nodes.
        let mut current = body;
        for _ in 0..n {
            current = recexpr.add(Self::make(OpWithVar::Node(O::from_name("lam")), vec![current]));
        }
        current
    }
}

/// LambdaCalc family. Cost behavior is selected at runtime via the `Weights`
/// stored on `StitchAnalysis` (defaults to all-ones for babble parity; tune
/// per-kind via the `--sym-cost`/`--app-cost`/`--lam-cost` CLI flags).
#[derive(Clone, Copy, Debug)]
pub struct LambdaCalc;

impl LanguageFamily for LambdaCalc {
    type Discriminant<O: StitchOp> = LambdaCalcDisc<O>;
    type Apply<O: StitchOp> = LambdaCalcLanguage<O>;

    fn make<P: StitchOp>(op: LambdaCalcDisc<P>, kids: Vec<Id>) -> LambdaCalcLanguage<P> {
        match (op, kids.as_slice()) {
            (LambdaCalcDisc::Leaf(o), &[]) => LambdaCalcLanguage::Leaf(o),
            (LambdaCalcDisc::App, &[f, a]) => LambdaCalcLanguage::App([f, a]),
            (LambdaCalcDisc::Lam, &[b]) => LambdaCalcLanguage::Lam([b]),
            (LambdaCalcDisc::Programs, _) => LambdaCalcLanguage::Programs(kids),
            (op, _) => panic!("LambdaCalc::make: {op} got wrong arity ({} children)", kids.len()),
        }
    }

    fn map_discriminant<A: StitchOp, B: StitchOp>(op: LambdaCalcDisc<A>, mut f: impl FnMut(A) -> B) -> LambdaCalcDisc<B> {
        match op {
            LambdaCalcDisc::Leaf(a) => LambdaCalcDisc::Leaf(f(a)),
            LambdaCalcDisc::App => LambdaCalcDisc::App,
            LambdaCalcDisc::Lam => LambdaCalcDisc::Lam,
            LambdaCalcDisc::Programs => LambdaCalcDisc::Programs,
        }
    }

    fn add_stub_application<O: StitchOp>(name: &str, children: Vec<Id>, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>>) -> Id {
        let mut current = egraph.add(LambdaCalcLanguage::Leaf(O::from_name(name)));
        for child in children {
            current = egraph.add(LambdaCalcLanguage::App([current, child]));
        }
        current
    }

    fn stub_application_size(arity: usize, weights: &Weights) -> u32 {
        // Leaf head plus one curried `App` per argument.
        weights.sym_var_cost + arity as u32 * weights.app_cost
    }

    fn symbol_cost(weights: &Weights) -> u32 {
        weights.sym_var_cost
    }

    fn check_fast_vs_slow(fast: i64, slow: i64) {
        assert!(fast >= slow, "Fast rewrite size {} < slow rewrite size {} (LambdaCalc) — fast path must be an upper bound", fast, slow);
    }

    fn make_var<O: StitchOp>(v: egg::Var) -> LambdaCalcLanguage<OpWithVar<O>> {
        Self::make(LambdaCalcDisc::Leaf(OpWithVar::Var(v)), vec![])
    }

    fn wrap_lams<O: StitchOp>(child: Id, n: u32, egraph: &mut StitchEgraph<LambdaCalcLanguage<O>>) -> Id {
        let mut current = child;
        for _ in 0..n {
            current = egraph.add(LambdaCalcLanguage::Lam([current]));
        }
        current
    }

    fn lams_cost(n: u32, weights: &Weights) -> u32 {
        n * weights.lam_cost
    }

    fn wrap_pattern_with_db_apps<O: StitchOp>(recexpr: &mut egg::RecExpr<LambdaCalcLanguage<OpWithVar<O>>>, head: Id, db_args: &[i32]) -> Id {
        let mut current = head;
        for &db in db_args {
            let var_op = OpWithVar::Node(O::make_db_var(db).expect("higher-order display needs a DB-var-bearing leaf op"));
            let var_id = recexpr.add(LambdaCalcLanguage::Leaf(var_op));
            current = recexpr.add(LambdaCalcLanguage::App([current, var_id]));
        }
        current
    }

    /// Peel left-leaning `App([X, $n])` apps whose right children are DB-var
    /// leaves with *strictly ascending* indices read outer-to-inner — exactly
    /// the shape `wrap_pattern_with_db_apps` produces from `vis.iter().rev()`
    /// for any sorted-ascending `vis`. Commits to the unwrap only when the
    /// inner head is a metavar leaf, so genuine binary-op chains like
    /// `(f $0 $1)` (whose head isn't a metavar) pass through unchanged.
    fn unwrap_pattern_db_apps<O: StitchOp>(nodes: &[LambdaCalcLanguage<OpWithVar<O>>], id: Id) -> Id {
        let mut cur = id;
        let mut last_db: Option<i32> = None;
        while let LambdaCalcLanguage::App([left, right]) = nodes[usize::from(cur)] {
            let Some(db) = nodes[usize::from(right)].discriminant().de_bruijn_index() else { break };
            if let Some(prev) = last_db
                && db <= prev
            {
                break;
            }
            last_db = Some(db);
            cur = left;
        }
        if last_db.is_some() && nodes[usize::from(cur)].discriminant().as_var().is_some() { cur } else { id }
    }

    fn wrap_pattern_lams<O: StitchOp>(recexpr: &mut RecExpr<LambdaCalcLanguage<OpWithVar<O>>>, body: Id, n: u32) -> Id {
        let mut current = body;
        for _ in 0..n {
            current = recexpr.add(LambdaCalcLanguage::Lam([current]));
        }
        current
    }

    fn parse_follow_pattern<O: StitchOp>(s: &str) -> anyhow::Result<crate::revexpr::RevExpr<LambdaCalcLanguage<OpWithVar<O>>>> {
        // Reuse the program-side parser at `OpWithVar<O>`: `OpWithVar::from_name`
        // already routes `?#k` atoms to `Var(v)`, so flat-form follow patterns
        // (including var-headed apps like `(?#0 a b)`) appify correctly via the
        // same curried-App handling.
        Ok(LambdaCalcLanguage::<OpWithVar<O>>::parse_program(s)?.into())
    }
}
