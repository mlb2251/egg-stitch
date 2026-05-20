use crate::lang::{LanguageFamily, OpWithVar, StitchDisc, StitchOp};
use crate::revexpr::RevExpr;
use egg::{Id, Language};
use std::collections::HashMap;

/// Tiny s-expression value used by `follow_variants` to rewrite HO-applied
/// metavars at the surface-syntax level before parsing. Atoms are strings;
/// lists are `Vec<Sexp>`.
#[derive(Clone)]
enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

fn parse_sexp(s: &str) -> Sexp {
    let tokens: Vec<String> = {
        let mut toks = Vec::new();
        let mut it = s.chars().peekable();
        while let Some(&c) = it.peek() {
            if c.is_whitespace() {
                it.next();
            } else if c == '(' || c == ')' {
                toks.push(c.to_string());
                it.next();
            } else {
                let mut a = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' {
                        break;
                    }
                    a.push(c);
                    it.next();
                }
                toks.push(a);
            }
        }
        toks
    };
    let mut pos = 0usize;
    fn read(toks: &[String], pos: &mut usize) -> Sexp {
        let t = &toks[*pos];
        *pos += 1;
        if t != "(" {
            return Sexp::Atom(t.clone());
        }
        let mut items = Vec::new();
        while toks[*pos] != ")" {
            items.push(read(toks, pos));
        }
        *pos += 1;
        Sexp::List(items)
    }
    read(&tokens, &mut pos)
}

fn render_sexp(x: &Sexp) -> String {
    match x {
        Sexp::Atom(s) => s.clone(),
        Sexp::List(xs) => format!("({})", xs.iter().map(render_sexp).collect::<Vec<_>>().join(" ")),
    }
}

fn is_meta(x: &Sexp) -> bool {
    matches!(x, Sexp::Atom(s) if s.starts_with("?#"))
}

fn is_db(x: &Sexp) -> bool {
    matches!(x, Sexp::Atom(s) if s.starts_with('$') && s[1..].chars().all(|c| c.is_ascii_digit()) && s.len() > 1)
}

/// Collapse `(?#k $a $b …)` (all args db-vars) → bare `?#k`. Recurses.
fn collapse_full(x: Sexp) -> Sexp {
    match x {
        Sexp::Atom(_) => x,
        Sexp::List(items) => {
            if let Some(head) = items.first()
                && is_meta(head)
                && items.iter().skip(1).all(is_db)
            {
                return head.clone();
            }
            Sexp::List(items.into_iter().map(collapse_full).collect())
        }
    }
}

/// Strip *leading* db-var args from any metavar application, keeping
/// trailing structural args. Recurses into kept args.
fn strip_leading(x: Sexp) -> Sexp {
    match x {
        Sexp::Atom(_) => x,
        Sexp::List(items) => {
            if let Some(head) = items.first()
                && is_meta(head)
            {
                let mut i = 1;
                while i < items.len() && is_db(&items[i]) {
                    i += 1;
                }
                let kept: Vec<Sexp> = items.iter().skip(i).cloned().map(strip_leading).collect();
                if kept.is_empty() {
                    return head.clone();
                }
                let mut out = Vec::with_capacity(1 + kept.len());
                out.push(head.clone());
                out.extend(kept);
                return Sexp::List(out);
            }
            Sexp::List(items.into_iter().map(strip_leading).collect())
        }
    }
}

/// Generate up to three surface-syntax variants of the follow target —
/// `raw`, `ho-stripped` (leading db-var HO-args dropped from metavar apps,
/// recursing into structural args), and `normalized` (fully collapse
/// `(?#k $a …)` → `?#k`). Deduplicated; `raw` is always first.
///
/// Two metavar-application forms appear in stitch output: the literal App
/// tree built by `Expand` actions (search-state's canonical form) and the
/// HO-arity-decorated form added by display when a metavar captures bound
/// vars. The same JSON string can mean either, so we try all variants.
pub fn follow_variants(raw: &str) -> Vec<String> {
    let parsed = parse_sexp(raw);
    let raw_norm = render_sexp(&parsed);
    let stripped = render_sexp(&strip_leading(parsed.clone()));
    let collapsed = render_sexp(&collapse_full(parsed));
    let mut out = vec![raw_norm.clone()];
    for v in [stripped, collapsed] {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// Structural equality of two subtrees in the follow tree. Needed because
/// RecExpr doesn't hash-cons — repeated `?#0` nodes get distinct Ids.
fn follow_subtrees_equal<F: LanguageFamily, O: StitchOp>(follow: &RevExpr<F::Apply<OpWithVar<O>>>, a: Id, b: Id) -> bool {
    if a == b {
        return true;
    }
    let (na, nb) = (&follow[a], &follow[b]);
    match (na.discriminant().as_var(), nb.discriminant().as_var()) {
        (Some(va), Some(vb)) => va == vb,
        (None, None) => na.matches(nb) && na.children().iter().zip(nb.children().iter()).all(|(&ca, &cb)| follow_subtrees_equal::<F, O>(follow, ca, cb)),
        _ => false,
    }
}

/// Checks whether a pattern is a valid prefix of a follow target.
/// Pattern Var matches any subtree (binding the var); pattern Node at a
/// follow-Var position is rejected. Uses `StitchDisc::as_var` so the same logic
/// works for any language family — the discriminant carries the var info.
pub fn check_follow<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, pid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, var_bindings: &mut HashMap<egg::Var, Id>) -> bool {
    let (pn, fn_) = (&pattern[pid], &follow[fid]);
    if let Some(v) = pn.discriminant().as_var() {
        return match var_bindings.entry(v) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(fid);
                true
            }
            std::collections::hash_map::Entry::Occupied(e) => follow_subtrees_equal::<F, O>(follow, *e.get(), fid),
        };
    }
    if fn_.discriminant().as_var().is_some() {
        return false;
    }
    pn.matches(fn_) && pn.children().iter().zip(fn_.children().iter()).all(|(&pc, &fc)| check_follow::<F, O>(pattern, pc, follow, fc, var_bindings))
}

/// Checks whether `pattern` is alpha-equivalent to the follow target — i.e. the
/// search has actually reached the goal, not just a prefix. Requires a bijection
/// between pattern vars and follow vars, with identical structure elsewhere.
pub fn check_follow_exact<F: LanguageFamily, O: StitchOp>(pattern: &RevExpr<F::Apply<OpWithVar<O>>>, pid: Id, follow: &RevExpr<F::Apply<OpWithVar<O>>>, fid: Id, p_to_f: &mut HashMap<egg::Var, egg::Var>, f_to_p: &mut HashMap<egg::Var, egg::Var>) -> bool {
    let (pn, fn_) = (&pattern[pid], &follow[fid]);
    match (pn.discriminant().as_var(), fn_.discriminant().as_var()) {
        (Some(pv), Some(fv)) => {
            let pf_ok = match p_to_f.entry(pv) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(fv);
                    true
                }
                std::collections::hash_map::Entry::Occupied(e) => *e.get() == fv,
            };
            let fp_ok = match f_to_p.entry(fv) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(pv);
                    true
                }
                std::collections::hash_map::Entry::Occupied(e) => *e.get() == pv,
            };
            pf_ok && fp_ok
        }
        (None, None) => pn.matches(fn_) && pn.children().iter().zip(fn_.children().iter()).all(|(&pc, &fc)| check_follow_exact::<F, O>(pattern, pc, follow, fc, p_to_f, f_to_p)),
        _ => false,
    }
}
