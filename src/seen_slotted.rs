//! Slotted-egraph backend for the search's seen-set.
//!
//! The seen-set asks one question repeatedly: "have I already explored a search
//! state equivalent to this `(pattern, freeze-mask)`?" The interesting source of
//! redundancy is *alpha-equivalence*: when a domain has commutative/associative
//! rewrites, the search emits the same pattern over and over with its
//! metavariables permuted (`(+ ?#0 ?#1)` vs `(+ ?#1 ?#0)`), and a syntactic hash
//! treats each permutation as new. A [slotted e-graph](https://github.com/memoryleak47/slotted-egraphs)
//! canonicalises terms up to renaming of their free *slots*, so it collapses
//! exactly these permutations natively, with no rewrite rules required.
//!
//! Encoding (see [`SeenLang`]):
//! - a *non-frozen* metavariable `?#k` becomes a free slot, materialised as the
//!   leaf e-node `(pv $k)`. Reuse of `?#k` reuses slot `$k`, so sharing is kept.
//! - a *frozen* metavariable `?#k` becomes `(fpv $k)` — the same slot `$k`, but
//!   under a distinct head. Frozenness is thus structural (so a frozen position
//!   never collapses with a free one) yet still travels with the slot under
//!   renaming (so two states that differ only by relabelling frozen/free vars
//!   *do* collapse). See the module tests for the worked cases.
//! - every program operator becomes `Op(symbol, children)` — a single
//!   variable-arity node, so we don't have to know the operator set at compile
//!   time.
//! - the whole top-level term is wrapped in `Op($root$, [term])` so a membership
//!   lookup hits only when the *whole* pattern was inserted, not when it merely
//!   occurs as a subterm of a larger inserted pattern (`add_expr` interns every
//!   subterm).
//!
//! Domain rewrite rules (DSRs) are lifted onto `SeenLang` and run to saturation
//! so the e-graph also dedups modulo DSR-equivalence, matching the behaviour of
//! the egg-backed seen-egraph it replaces.

use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use crate::pattern::Pattern;
use egg::{Language as EggLanguage, RecExpr as EggRecExpr};
use rustc_hash::FxHashMap;
use slotted_egraphs::{AppliedId, EGraph as SlottedEGraph, Language, Pattern as SlottedPattern, RecExpr as SlottedRecExpr, Rewrite, Slot, SlotMap, Symbol, SyntaxElem, apply_rewrites, lookup_rec_expr};

/// Reserved head for the top-level wrapper. No real operator is spelled this way.
const ROOT_NAME: &str = "$root$";
/// Head for a non-frozen pattern variable leaf `(pv $k)`.
const PVAR_NAME: &str = "pv";
/// Head for a frozen pattern variable leaf `(fpv $k)`.
const FVAR_NAME: &str = "fpv";

/// The seen-set's slotted language: a variable-arity operator node plus the two
/// pattern-variable leaves. `Op`'s `Symbol` carries the operator name and its
/// `Vec<AppliedId>` carries the children, so one variant covers every operator
/// (and the `$root$` wrapper) regardless of arity. `Var`/`FVar` each expose a
/// single public slot — the metavariable's identity.
///
/// This hand-writes [`Language`] rather than using `define_language!` because the
/// macro fixes each variant's arity and ties it to one operator name, neither of
/// which fits a corpus whose operator set is only known at runtime. The impl just
/// chains the per-child [`slotted_egraphs::LanguageChildren`] methods exactly as
/// the macro would.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SeenLang {
    Op(Symbol, Vec<AppliedId>),
    Var(Slot),
    FVar(Slot),
}

impl Language for SeenLang {
    fn all_slot_occurrences_mut(&mut self) -> Vec<&mut Slot> {
        match self {
            SeenLang::Op(_, kids) => kids.iter_mut().flat_map(|a| a.m.values_mut()).collect(),
            SeenLang::Var(s) | SeenLang::FVar(s) => vec![s],
        }
    }

    fn public_slot_occurrences_mut(&mut self) -> Vec<&mut Slot> {
        // No binders, so every slot is public; an operator's public slots are
        // exactly its children's public slots.
        self.all_slot_occurrences_mut()
    }

    fn applied_id_occurrences_mut(&mut self) -> Vec<&mut AppliedId> {
        match self {
            SeenLang::Op(_, kids) => kids.iter_mut().collect(),
            SeenLang::Var(_) | SeenLang::FVar(_) => vec![],
        }
    }

    fn all_slot_occurrences(&self) -> Vec<Slot> {
        match self {
            SeenLang::Op(_, kids) => kids.iter().flat_map(|a| a.m.values_immut().copied()).collect(),
            SeenLang::Var(s) | SeenLang::FVar(s) => vec![*s],
        }
    }

    fn public_slot_occurrences(&self) -> Vec<Slot> {
        self.all_slot_occurrences()
    }

    fn applied_id_occurrences(&self) -> Vec<&AppliedId> {
        match self {
            SeenLang::Op(_, kids) => kids.iter().collect(),
            SeenLang::Var(_) | SeenLang::FVar(_) => vec![],
        }
    }

    fn to_syntax(&self) -> Vec<SyntaxElem> {
        match self {
            SeenLang::Op(sym, kids) => {
                let mut v = vec![SyntaxElem::String(sym.to_string())];
                v.extend(kids.iter().map(|a| SyntaxElem::AppliedId(a.clone())));
                v
            }
            SeenLang::Var(s) => vec![SyntaxElem::String(PVAR_NAME.to_string()), SyntaxElem::Slot(*s)],
            SeenLang::FVar(s) => vec![SyntaxElem::String(FVAR_NAME.to_string()), SyntaxElem::Slot(*s)],
        }
    }

    fn from_syntax(elems: &[SyntaxElem]) -> Option<Self> {
        let SyntaxElem::String(head) = elems.first()? else {
            return None;
        };
        match head.as_str() {
            PVAR_NAME => match elems {
                [_, SyntaxElem::Slot(s)] => Some(SeenLang::Var(*s)),
                _ => None,
            },
            FVAR_NAME => match elems {
                [_, SyntaxElem::Slot(s)] => Some(SeenLang::FVar(*s)),
                _ => None,
            },
            _ => {
                // Every remaining element must be an applied-id child (operators
                // never take a bare slot in our rule patterns).
                let mut kids = Vec::with_capacity(elems.len() - 1);
                for e in &elems[1..] {
                    match e {
                        SyntaxElem::AppliedId(a) => kids.push(a.clone()),
                        _ => return None,
                    }
                }
                Some(SeenLang::Op(Symbol::from(head.as_str()), kids))
            }
        }
    }

    fn slots(&self) -> slotted_egraphs::SmallHashSet<Slot> {
        self.public_slot_occurrences().into_iter().collect()
    }

    fn weak_shape_inplace(&mut self) -> SlotMap {
        use slotted_egraphs::LanguageChildren;
        let m = &mut (SlotMap::new(), 0u32);
        match self {
            SeenLang::Op(_, kids) => {
                for a in kids.iter_mut() {
                    a.weak_shape_impl(m);
                }
            }
            SeenLang::Var(s) | SeenLang::FVar(s) => {
                s.weak_shape_impl(m);
            }
        }
        m.0.inverse()
    }
}

/// Build the slot for metavariable index `k`. Distinct indices give distinct
/// slots; the same index always gives the same slot (so reuse is shared).
fn var_slot(k: usize) -> Slot {
    Slot::numeric(k as u32)
}

/// Diagnostic encoding overrides, read once from the environment. These are pure
/// experiment knobs for understanding *what* inflates the seen-egraph (see
/// `docs/slotted_seen_set.html`); both off by default so normal runs use the
/// real encoding. They deliberately change dedup semantics, so they're for
/// measurement only, never production.
struct EncodingMode {
    /// `SEEN_NO_FROZEN`: encode frozen metavars as ordinary free vars (drop the
    /// `fpv` head), to measure how much the frozen/free split inflates the egraph.
    no_frozen: bool,
    /// `SEEN_UNIFY_VARS`: encode every metavar as the *same* slot `$0`, collapsing
    /// all variable identity (and reuse distinctions), to measure how much slot
    /// diversity costs. Over-merges by design.
    unify_vars: bool,
    /// `SEEN_VARS_AS_CONST`: encode metavar `?#k` as a *distinct nullary operator*
    /// `vK` (or `fvK` if frozen) carrying **no slot**, mimicking the old egg
    /// encoding where `Var(k)` was a plain constant leaf. With no slots, slotted
    /// alpha-equivalence folds *nothing*, so variable permutations a rewrite
    /// generates stay distinct e-nodes. Comparing egraph size against the real
    /// (slot-bearing) encoding measures exactly how much slotted's native alpha
    /// folds *during saturation* — the win that only shows once rules run.
    vars_as_const: bool,
}

/// The active diagnostic encoding mode, read once from the environment.
fn encoding_mode() -> &'static EncodingMode {
    use std::sync::OnceLock;
    static MODE: OnceLock<EncodingMode> = OnceLock::new();
    MODE.get_or_init(|| EncodingMode {
        no_frozen: std::env::var_os("SEEN_NO_FROZEN").is_some(),
        unify_vars: std::env::var_os("SEEN_UNIFY_VARS").is_some(),
        vars_as_const: std::env::var_os("SEEN_VARS_AS_CONST").is_some(),
    })
}

/// A node of `Op` with `arity` placeholder children, ready for `RecExpr`
/// (`add_expr`/`lookup_rec_expr` overwrite the placeholders from the children).
fn op_node(name: &str, arity: usize) -> SeenLang {
    SeenLang::Op(Symbol::from(name), vec![AppliedId::null(); arity])
}

/// Convert a pattern plus its freeze mask into a `$root$`-wrapped slotted
/// `RecExpr`. Every metavariable leaf becomes `(pv $k)` or `(fpv $k)` per the
/// mask; every other node becomes `Op(name, children)` keyed by the operator's
/// display name. Family-generic: it only relies on each node's discriminant
/// (display name + var detection) and child list.
pub fn pattern_to_seen_recexpr<F: LanguageFamily, O: StitchOp>(pattern: &Pattern<F, O>, frozen: &[bool]) -> SlottedRecExpr<SeenLang> {
    // Vars are detected *intrinsically* per node (via `as_var`), not by position:
    // converting the pattern's `RevExpr` to an egg `RecExpr` reverses the node
    // array and remaps child ids, so the pattern's own var-id table no longer
    // lines up with the converted nodes. The canonical invariant numbers `?#k` as
    // `egg::Var::from(k)`, so recover `k` (hence its slot and freeze bit) from a
    // `Var -> k` map over `0..frozen.len()`.
    let var_to_k: FxHashMap<egg::Var, usize> = (0..frozen.len()).map(|k| (egg::Var::from(k as u32), k)).collect();
    let src: EggRecExpr<F::Apply<OpWithVar<O>>> = pattern.pattern.clone().into();
    let nodes: Vec<F::Apply<OpWithVar<O>>> = src.into();
    let root = nodes.len() - 1; // egg `RecExpr` keeps the root last.
    let body = build_seen::<F, O>(&nodes, &var_to_k, frozen, root);
    wrap_root(body)
}

/// Wrap a body term in the `$root$` sentinel. Built programmatically because
/// `$root$` starts with the slot sigil `$` and so isn't parseable from a string.
pub fn wrap_root(body: SlottedRecExpr<SeenLang>) -> SlottedRecExpr<SeenLang> {
    SlottedRecExpr { node: op_node(ROOT_NAME, 1), children: vec![body] }
}

/// Recursively build the slotted `RecExpr` for the program-pattern node at `i`.
/// A metavariable leaf becomes a `(pv $k)`/`(fpv $k)` slot leaf (keyed by its
/// `Var`'s index `k`); any other node becomes `Op(display-name, children)`.
fn build_seen<F: LanguageFamily, O: StitchOp>(nodes: &[F::Apply<OpWithVar<O>>], var_to_k: &FxHashMap<egg::Var, usize>, frozen: &[bool], i: usize) -> SlottedRecExpr<SeenLang> {
    let node = &nodes[i];
    let disc = node.discriminant();
    if let Some(v) = disc.as_var() {
        let k = var_to_k[&v];
        // Diagnostic overrides (off by default): `vars_as_const` drops slots
        // entirely (egg-style constants, so alpha folds nothing); `unify_vars`
        // maps every var to one slot; `no_frozen` drops the frozen/free
        // distinction. See [`EncodingMode`].
        let mode = encoding_mode();
        let frozen_here = frozen[k] && !mode.no_frozen;
        if mode.vars_as_const {
            let idx = if mode.unify_vars { 0 } else { k };
            let name = if frozen_here { format!("fv{idx}") } else { format!("v{idx}") };
            return SlottedRecExpr { node: op_node(&name, 0), children: vec![] };
        }
        let slot = if mode.unify_vars { var_slot(0) } else { var_slot(k) };
        let leaf = if frozen_here { SeenLang::FVar(slot) } else { SeenLang::Var(slot) };
        return SlottedRecExpr { node: leaf, children: vec![] };
    }
    let name = disc.to_string();
    let children: Vec<SlottedRecExpr<SeenLang>> = node.children().iter().map(|&c| build_seen::<F, O>(nodes, var_to_k, frozen, usize::from(c))).collect();
    SlottedRecExpr { node: op_node(&name, children.len()), children }
}

/// Lift the program-language DSRs in `path` onto [`SeenLang`]. The file uses the
/// usual `name: lhs => rhs` (or `<=>`) format with `//` comments; operator names
/// and `?x` metavariables carry over verbatim because the slotted pattern syntax
/// is the same. A rule whose sides don't parse as `SeenLang` patterns (e.g. a
/// lambda-calc rule using de-Bruijn `$i`, which the slotted parser reads as a
/// slot) is skipped with a warning rather than aborting the run. `constant_folding`
/// directives are not supported by the seen-egraph and are skipped.
///
/// Returns the lifted rewrites and the count of rules skipped, so the caller can
/// surface how faithfully the theory transferred.
pub fn lift_rules_from_file(path: &str) -> (Vec<Rewrite<SeenLang>>, usize) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("seen-egraph: cannot read rules {path:?}: {e}");
            return (vec![], 0);
        }
    };
    let mut rules = Vec::new();
    let mut skipped = 0usize;
    for line in contents.lines().map(|l| l.split_once("//").map_or(l, |(l, _)| l).trim()).filter(|l| !l.is_empty()) {
        let Some((name, rewrite)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "constant_folding" {
            skipped += 1;
            continue;
        }
        // `=>` is a substring of `<=>`, so test the bidirectional arrow first.
        let (lhs, rhs, bidir) = match rewrite.split_once("<=>") {
            Some((l, r)) => (l.trim(), r.trim(), true),
            None => match rewrite.split_once("=>") {
                Some((l, r)) => (l.trim(), r.trim(), false),
                None => continue,
            },
        };
        // Pre-validate both sides parse as SeenLang patterns; `Rewrite::new`
        // would otherwise panic on a parse error.
        if SlottedPattern::<SeenLang>::parse(lhs).is_err() || SlottedPattern::<SeenLang>::parse(rhs).is_err() {
            eprintln!("seen-egraph: skipping rule {name:?} (sides not parseable as SeenLang)");
            skipped += 1;
            continue;
        }
        if bidir {
            rules.push(Rewrite::new(&format!("{name}-rev"), rhs, lhs));
        }
        rules.push(Rewrite::new(name, lhs, rhs));
    }
    (rules, skipped)
}

/// A slotted seen-egraph plus the lifted DSRs, exposing the small surface the
/// [`crate::search::SeenTracker`] needs: membership lookup, insert, saturate,
/// and a couple of size stats.
pub struct SlottedSeen {
    egraph: SlottedEGraph<SeenLang>,
    rules: Vec<Rewrite<SeenLang>>,
}

impl SlottedSeen {
    /// New empty seen-egraph carrying `rules` (already lifted to [`SeenLang`]).
    pub fn new(rules: Vec<Rewrite<SeenLang>>) -> Self {
        SlottedSeen { egraph: SlottedEGraph::new(()), rules }
    }

    /// Number of lifted DSRs.
    pub fn num_rules(&self) -> usize {
        self.rules.len()
    }

    /// Whether the `$root$`-wrapped term is already present (modulo slot renaming
    /// and any merges saturation has made) — i.e. an equivalent state was seen.
    pub fn contains(&self, re: &SlottedRecExpr<SeenLang>) -> bool {
        lookup_rec_expr(re, &self.egraph).is_some()
    }

    /// Insert the term (and its subterms) into the e-graph, returning the applied
    /// id of the (`$root$`-wrapped) top-level class. A freshly inserted root is a
    /// single-enode class; see [`Self::root_is_multinode`].
    pub fn insert(&mut self, re: &SlottedRecExpr<SeenLang>) -> AppliedId {
        self.egraph.add_expr(re.clone())
    }

    /// Whether the root class behind `aid` now holds more than one e-node. Since
    /// no rule mentions `$root$`, a root class only gains a second node by merging
    /// (via congruence) with another inserted root — i.e. saturation found two
    /// distinct inserted states to be DSR-equivalent. The dynamic batch tuner uses
    /// this as its "did the last flush dedup anything" signal.
    pub fn root_is_multinode(&self, aid: &AppliedId) -> bool {
        let id = self.egraph.find_applied_id(aid).id;
        self.egraph.enodes(id).len() > 1
    }

    /// Run the lifted DSRs to saturation (or until `iter_limit`), rebuilding as
    /// it goes. Returns whether the e-graph changed and the number of iterations.
    pub fn saturate(&mut self, iter_limit: usize) -> (bool, usize) {
        let mut iters = 0;
        let mut changed = false;
        while iters < iter_limit {
            if !apply_rewrites(&mut self.egraph, &self.rules) {
                break;
            }
            changed = true;
            iters += 1;
        }
        (changed, iters)
    }

    /// Canonical e-class id of a (`$root$`-wrapped) term, if present.
    pub fn class_of(&self, re: &SlottedRecExpr<SeenLang>) -> Option<slotted_egraphs::Id> {
        lookup_rec_expr(re, &self.egraph).map(|a| self.egraph.find_applied_id(&a).id)
    }

    /// Number of live e-classes.
    pub fn num_classes(&self) -> usize {
        self.egraph.ids().len()
    }

    /// Total number of e-nodes.
    pub fn num_nodes(&self) -> usize {
        self.egraph.total_number_of_nodes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotted_egraphs::RecExpr as R;

    /// Parse a `SeenLang` *body* term (pv/fpv leaves), then wrap it in `$root$`.
    fn body(s: &str) -> R<SeenLang> {
        wrap_root(R::parse(s).unwrap())
    }

    /// `(T ?#0 ?#1)` and `(T ?#1 ?#0)` are alpha-equivalent: after inserting one,
    /// the other is already a member — no rules required. (Membership, not
    /// `eg.eq` on the returned applied-ids: those bind the free slots in opposite
    /// order, so they differ as *applied* terms while sharing an e-class.)
    #[test]
    fn alpha_equivalent_metavars_collapse() {
        let mut seen = SlottedSeen::new(vec![]);
        seen.insert(&body("(T (pv $0) (pv $1))"));
        assert!(seen.contains(&body("(T (pv $1) (pv $0))")), "swapped metavars are the same state");
        assert_eq!(seen.num_classes(), seen.num_classes()); // smoke
    }

    /// Distinct-but-relabelled frozen states collapse: "first child frozen,
    /// second free" is the same state regardless of which label is on which.
    #[test]
    fn relabelled_frozen_collapses() {
        let mut seen = SlottedSeen::new(vec![]);
        seen.insert(&body("(T (fpv $0) (pv $1))"));
        assert!(seen.contains(&body("(T (fpv $1) (pv $0))")), "frozenness travels with the slot under renaming");
    }

    /// Frozen and free at the *same* position are different states and must stay
    /// distinct — frozenness is structural.
    #[test]
    fn frozen_vs_free_stays_distinct() {
        let mut seen = SlottedSeen::new(vec![]);
        seen.insert(&body("(T (fpv $0) (pv $1))"));
        assert!(!seen.contains(&body("(T (pv $0) (fpv $1))")), "which child is frozen must distinguish states");
    }

    /// Distinct top-level patterns must NOT be reported as members of each other
    /// (no rules). Guards against subterm/root-wrapper false positives.
    #[test]
    fn distinct_patterns_not_members() {
        let mut seen = SlottedSeen::new(vec![]);
        seen.insert(&body("(T (pv $0) (pv $1))"));
        assert!(!seen.contains(&body("(M (pv $0) (pv $1) (pv $2) (pv $3))")), "M-pattern wrongly a member after inserting T");
        seen.insert(&body("(T (pv $0) (M (pv $1) (pv $2) (pv $3) (pv $4)))"));
        // The subterm (M ...) is interned but was never a *top-level* insert.
        assert!(!seen.contains(&body("(M (pv $0) (pv $1) (pv $2) (pv $3))")), "interned subterm wrongly a top-level member");
        assert!(!seen.contains(&body("(pv $0)")), "single-var pattern wrongly a member");
        assert!(!seen.contains(&body("l")), "leaf wrongly a member");
    }

    /// A scale-1 DSR (`l => (T l (M 1 0 0 0))`) merges a term with its scaled
    /// form after saturation, exactly as the egg seen-egraph did.
    #[test]
    fn dsr_saturation_merges() {
        let rules = vec![Rewrite::new("scale_1_l", "l", "(T l (M 1 0 0 0))")];
        let mut seen = SlottedSeen::new(rules);
        let plain = body("l");
        let scaled = body("(T l (M 1 0 0 0))");
        seen.insert(&plain);
        assert!(!seen.contains(&scaled), "not equivalent before saturation");
        seen.saturate(1000);
        assert!(seen.contains(&scaled), "scale-1 DSR makes the scaled form a repeat");
    }

    /// The "right sharing", part 1 — *commutativity* reordering of a pattern's
    /// variables is folded by alpha-equivalence with NO rule: `(+ ?#0 ?#1)`
    /// already makes `(+ ?#1 ?#0)` a member. This is the blow-up source the
    /// slotted backend exists to fold, handled structurally at insert time.
    #[test]
    fn commutativity_reordering_folds_via_alpha() {
        let mut seen = SlottedSeen::new(vec![]); // no rules at all
        seen.insert(&body("(+ (pv $0) (pv $1))"));
        assert!(seen.contains(&body("(+ (pv $1) (pv $0))")), "commuted variables are the same state");
    }

    /// The "right sharing", part 2 — *associativity* reshapes the tree, which
    /// alpha-equivalence does NOT fold (a leaf slot can't be slot-renamed into a
    /// subtree), so it must come from the DSR. This pins both halves: without the
    /// rule the two bracketings stay distinct (no over-merging); with the rule
    /// saturated they share. Together with part 1 this is exactly the right
    /// equivalence — comm for free, assoc on demand, nothing spurious.
    #[test]
    fn associativity_needs_dsr_not_alpha() {
        let left = body("(+ (+ (pv $0) (pv $1)) (pv $2))");
        let right = body("(+ (pv $0) (+ (pv $1) (pv $2)))");

        // alpha alone must NOT reassociate.
        let mut bare = SlottedSeen::new(vec![]);
        bare.insert(&left);
        assert!(!bare.contains(&right), "alpha must not silently reassociate");

        // the assoc DSR, saturated, does.
        let assoc = Rewrite::new("assoc", "(+ (+ ?a ?b) ?c)", "(+ ?a (+ ?b ?c))");
        let mut withrule = SlottedSeen::new(vec![assoc]);
        withrule.insert(&left);
        withrule.saturate(1000);
        assert!(withrule.contains(&right), "the assoc DSR reassociates");
    }
}
