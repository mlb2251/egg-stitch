#!/usr/bin/env python3
"""External equivalence checker for egg-stitch outputs.

Reads a *.out.json file produced by `egg-stitch --output …` and, for each
(original, rewritten) program pair, checks that they are equivalent. Library
abstractions are inlined via their closed `lambda` form, then β-reduction
plus any user-supplied DSRs from a `--rewrites` file are used to decide
equivalence. This is a standalone oracle for the search machinery — it does
not import anything from the egg-stitch crate.

Usage:
    scripts/check_equiv.py path/to/run.out.json [--rewrites RULES] [-v]

With no `--rewrites`, the checker does pure β-normalization and structural
comparison. With `--rewrites`, it runs a small e-graph saturation that
applies β and the DSRs together, then checks whether the two roots end up
in the same e-class. Capped by `--iters` and `--nodes` so divergent rule
sets (e.g. `?x => (+ 0 ?x)`) fail cleanly rather than hang.

Exit 0 if every pair is provably equivalent. Nonzero on any mismatch or
unresolved saturation cap.

This handles lambda-calc outputs (library entries with a `lambda` field).
OpChildren outputs (no lambdas) are skipped — those would need positional
`?#k` substitution against the `pattern` field instead.

Term representation (tuples for hashability):
    ("var", n)              — de Bruijn variable $n
    ("sym", s)              — atomic symbol (operator, primitive, library name)
    ("lam", body)
    ("app", f, a)
    ("programs", (kids…))
Patterns use ("pvar", name) for `?x`-style metavariables.
"""

import argparse
import json
import sys


# ---------- s-expression parsing ----------

def tokenize(s):
    out = []
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c.isspace():
            i += 1
        elif c == "(" or c == ")":
            out.append(c)
            i += 1
        else:
            j = i
            while j < n and not s[j].isspace() and s[j] not in "()":
                j += 1
            out.append(s[i:j])
            i = j
    return out


def parse_sexp(toks, i):
    if toks[i] == "(":
        items = []
        i += 1
        while toks[i] != ")":
            x, i = parse_sexp(toks, i)
            items.append(x)
        return items, i + 1
    return toks[i], i + 1


def atom_to_term(a, pattern=False):
    if pattern and a.startswith("?"):
        return ("pvar", a[1:])
    if a.startswith("$") and a[1:].isdigit():
        return ("var", int(a[1:]))
    return ("sym", a)


def sexp_to_term(sexp, pattern=False):
    """Mirror egg-stitch's `LambdaCalc::parse_program`: only `lam`/`lambda`/`λ`,
    `@`, and `programs` are special heads; everything else (including the
    literal symbol `app` used by stitch's corpora) is a leaf and the surrounding
    list curries via structural `@`."""
    if isinstance(sexp, str):
        return atom_to_term(sexp, pattern)
    if not sexp:
        raise ValueError("empty s-expression list")
    head = sexp[0]
    if isinstance(head, str):
        if head in ("lam", "lambda", "λ"):
            if len(sexp) != 2:
                raise ValueError(f"lam expects 1 arg, got {len(sexp) - 1}")
            return ("lam", sexp_to_term(sexp[1], pattern))
        if head == "@":
            if len(sexp) != 3:
                raise ValueError(f"@ expects 2 args, got {len(sexp) - 1}")
            return ("app", sexp_to_term(sexp[1], pattern), sexp_to_term(sexp[2], pattern))
        if head == "programs":
            return ("programs", tuple(sexp_to_term(c, pattern) for c in sexp[1:]))
    cur = sexp_to_term(head, pattern)
    for arg in sexp[1:]:
        cur = ("app", cur, sexp_to_term(arg, pattern))
    return cur


def parse_term(s, pattern=False):
    toks = tokenize(s)
    sexp, _ = parse_sexp(toks, 0)
    return sexp_to_term(sexp, pattern)


# ---------- rewrite rule parsing ----------

def parse_rewrites(path):
    """Returns a list of (lhs, rhs) pattern pairs. Each `<=>` rule expands to
    two entries (both directions)."""
    rules = []
    with open(path) as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            _name, body = line.split(":", 1)
            if "<=>" in body:
                lhs, rhs = body.split("<=>", 1)
                lhs_t, rhs_t = parse_term(lhs.strip(), pattern=True), parse_term(rhs.strip(), pattern=True)
                rules.append((lhs_t, rhs_t))
                rules.append((rhs_t, lhs_t))
            elif "=>" in body:
                lhs, rhs = body.split("=>", 1)
                rules.append((parse_term(lhs.strip(), pattern=True), parse_term(rhs.strip(), pattern=True)))
            else:
                raise ValueError(f"rule has no => or <=>: {raw!r}")
    return rules


# ---------- β-reduction (de Bruijn, $0 = innermost) ----------

def shift(t, d, cutoff):
    tag = t[0]
    if tag == "var":
        return ("var", t[1] + d) if t[1] >= cutoff else t
    if tag == "lam":
        return ("lam", shift(t[1], d, cutoff + 1))
    if tag == "app":
        return ("app", shift(t[1], d, cutoff), shift(t[2], d, cutoff))
    if tag == "programs":
        return ("programs", tuple(shift(c, d, cutoff) for c in t[1]))
    return t


def subst(t, j, s):
    """t[$j := s]; `s` is in the original context, shifted as we cross binders."""
    tag = t[0]
    if tag == "var":
        return s if t[1] == j else t
    if tag == "lam":
        return ("lam", subst(t[1], j + 1, shift(s, 1, 0)))
    if tag == "app":
        return ("app", subst(t[1], j, s), subst(t[2], j, s))
    if tag == "programs":
        return ("programs", tuple(subst(c, j, s) for c in t[1]))
    return t


def beta_contract(body, arg):
    """Reduce `((lam body) arg)` — shift_down( body[$0 := shift_up(arg)] )."""
    return shift(subst(body, 0, shift(arg, 1, 0)), -1, 0)


def beta_normalize(t, fuel):
    """Leftmost-outermost full-reduction normal form. Returns (term, fuel_left).
    On fuel exhaustion the returned term is partially reduced."""
    while fuel > 0:
        t2, hit, fuel = _beta_step(t, fuel)
        if not hit:
            return t, fuel
        t = t2
    return t, 0


def _beta_step(t, fuel):
    if fuel <= 0:
        return t, False, fuel
    tag = t[0]
    if tag == "app":
        # Outermost redex first.
        if t[1][0] == "lam":
            return beta_contract(t[1][1], t[2]), True, fuel - 1
        f2, hit, fuel = _beta_step(t[1], fuel)
        if hit:
            return ("app", f2, t[2]), True, fuel
        a2, hit, fuel = _beta_step(t[2], fuel)
        return ("app", t[1], a2), hit, fuel
    if tag == "lam":
        b2, hit, fuel = _beta_step(t[1], fuel)
        return ("lam", b2), hit, fuel
    if tag == "programs":
        kids = list(t[1])
        for i, c in enumerate(kids):
            c2, hit, fuel = _beta_step(c, fuel)
            if hit:
                kids[i] = c2
                return ("programs", tuple(kids)), True, fuel
    return t, False, fuel


# ---------- library inlining ----------

def inline_symbols(t, lib):
    """Replace every `Sym(name)` for `name` in `lib` with `lib[name]`."""
    tag = t[0]
    if tag == "sym":
        return lib.get(t[1], t)
    if tag == "lam":
        return ("lam", inline_symbols(t[1], lib))
    if tag == "app":
        return ("app", inline_symbols(t[1], lib), inline_symbols(t[2], lib))
    if tag == "programs":
        return ("programs", tuple(inline_symbols(c, lib) for c in t[1]))
    return t


def build_library(entries):
    """Returns {fn_name → lambda term}, with each entry inlining all earlier ones
    so the final term has no library references left."""
    resolved = {}
    for entry in entries:
        name = entry["pattern"].split(":", 1)[0].strip()
        lam_str = entry.get("lambda")
        if lam_str is None:
            continue
        body = parse_term(lam_str)
        resolved[name] = inline_symbols(body, resolved)
    return resolved


# ---------- minimal e-graph with β as a built-in rule ----------

class EGraph:
    """Hash-consed e-graph with a union-find over e-class ids. Supports
    pattern-based rewrites and a built-in β-step. No analyses; congruence is
    re-established by `rebuild` after each batch of unions."""

    def __init__(self):
        self.uf = {}                  # eid → parent
        self.hashcons = {}            # canonical enode → eid
        self.eclass_nodes = {}        # canonical eid → set of canonical enodes
        self._next = 0

    def _new_eid(self):
        eid = self._next
        self._next += 1
        self.uf[eid] = eid
        self.eclass_nodes[eid] = set()
        return eid

    def find(self, eid):
        while self.uf[eid] != eid:
            self.uf[eid] = self.uf[self.uf[eid]]
            eid = self.uf[eid]
        return eid

    def _canon_enode(self, enode):
        op, kids = enode
        return (op, tuple(self.find(k) for k in kids))

    def add(self, op, kids):
        canon = (op, tuple(self.find(k) for k in kids))
        if canon in self.hashcons:
            return self.find(self.hashcons[canon])
        eid = self._new_eid()
        self.hashcons[canon] = eid
        self.eclass_nodes[eid].add(canon)
        return eid

    def add_term(self, t):
        tag = t[0]
        if tag == "var":
            return self.add(("var", t[1]), ())
        if tag == "sym":
            return self.add(("sym", t[1]), ())
        if tag == "lam":
            return self.add("lam", (self.add_term(t[1]),))
        if tag == "app":
            return self.add("app", (self.add_term(t[1]), self.add_term(t[2])))
        if tag == "programs":
            return self.add("programs", tuple(self.add_term(c) for c in t[1]))
        raise ValueError(f"unhandled term: {t!r}")

    def union(self, a, b):
        a, b = self.find(a), self.find(b)
        if a == b:
            return False
        self.uf[b] = a
        return True

    def rebuild(self):
        """Re-canonicalize enodes after unions; merge any eclasses whose
        canonical enodes collide. Iterates to fixpoint."""
        changed = True
        while changed:
            changed = False
            new_hashcons = {}
            new_eclass_nodes = {}
            for enode, eid in self.hashcons.items():
                ceid = self.find(eid)
                cenode = self._canon_enode(enode)
                if cenode in new_hashcons:
                    if self.union(new_hashcons[cenode], ceid):
                        changed = True
                else:
                    new_hashcons[cenode] = ceid
            for ceid in set(self.find(e) for e in self.eclass_nodes):
                new_eclass_nodes[ceid] = set()
            for cenode, eid in new_hashcons.items():
                new_eclass_nodes[self.find(eid)].add(cenode)
            self.hashcons = new_hashcons
            self.eclass_nodes = new_eclass_nodes
            if changed:
                continue
            return

    def total_nodes(self):
        return len(self.hashcons)


def _pat_op_kids(pat):
    """Decompose a non-pvar pattern into (op-tag, child-patterns)."""
    tag = pat[0]
    if tag == "var":
        return ("var", pat[1]), ()
    if tag == "sym":
        return ("sym", pat[1]), ()
    if tag == "lam":
        return "lam", (pat[1],)
    if tag == "app":
        return "app", (pat[1], pat[2])
    if tag == "programs":
        return "programs", pat[1]
    raise ValueError(f"unhandled pattern: {pat!r}")


def ematch(eg, pat, eid, subst):
    """Yield substitutions making `pat` match e-class `eid`. `subst` maps pvar
    name → e-class id; bindings must be consistent across the pattern."""
    eid = eg.find(eid)
    if pat[0] == "pvar":
        name = pat[1]
        if name in subst:
            if subst[name] == eid:
                yield subst
            return
        new = dict(subst)
        new[name] = eid
        yield new
        return
    p_op, p_kids = _pat_op_kids(pat)
    for enode in list(eg.eclass_nodes.get(eid, ())):
        op, kids = enode
        if op != p_op or len(kids) != len(p_kids):
            continue
        substs = [subst]
        for pk, ek in zip(p_kids, kids):
            substs = [s for prev in substs for s in ematch(eg, pk, ek, prev)]
            if not substs:
                break
        yield from substs


def instantiate(eg, pat, subst):
    """Add the pattern's RHS to the e-graph, binding pvars to their matched
    e-classes. Returns the resulting e-class id."""
    tag = pat[0]
    if tag == "pvar":
        return subst[pat[1]]
    if tag == "var":
        return eg.add(("var", pat[1]), ())
    if tag == "sym":
        return eg.add(("sym", pat[1]), ())
    if tag == "lam":
        return eg.add("lam", (instantiate(eg, pat[1], subst),))
    if tag == "app":
        return eg.add("app", (instantiate(eg, pat[1], subst), instantiate(eg, pat[2], subst)))
    if tag == "programs":
        return eg.add("programs", tuple(instantiate(eg, c, subst) for c in pat[1]))
    raise ValueError(f"unhandled rhs pattern: {pat!r}")


def extract_smallest(eg, eid, memo=None, in_progress=None):
    """Pick a deterministic small representative term from the e-class. Used
    only by the β-step (which needs concrete body/arg terms). Cycles are
    avoided by tracking `in_progress` and returning an `None`-children sentinel
    for the offending eclass — but in practice, β operates on freshly-added
    structure so cycles don't arise on the substituted side."""
    if memo is None:
        memo = {}
        in_progress = set()
    eid = eg.find(eid)
    if eid in memo:
        return memo[eid]
    if eid in in_progress:
        return None
    in_progress.add(eid)
    best = None
    best_size = None
    for enode in eg.eclass_nodes.get(eid, ()):
        op, kids = enode
        child_terms = []
        ok = True
        for k in kids:
            ct = extract_smallest(eg, k, memo, in_progress)
            if ct is None:
                ok = False
                break
            child_terms.append(ct)
        if not ok:
            continue
        if op == "lam":
            term = ("lam", child_terms[0])
        elif op == "app":
            term = ("app", child_terms[0], child_terms[1])
        elif op == "programs":
            term = ("programs", tuple(child_terms))
        elif isinstance(op, tuple):
            term = op
        else:
            continue
        size = term_size(term)
        if best_size is None or size < best_size:
            best, best_size = term, size
    in_progress.discard(eid)
    if best is not None:
        memo[eid] = best
    return best


def term_size(t):
    tag = t[0]
    if tag in ("var", "sym"):
        return 1
    if tag == "lam":
        return 1 + term_size(t[1])
    if tag == "app":
        return 1 + term_size(t[1]) + term_size(t[2])
    if tag == "programs":
        return 1 + sum(term_size(c) for c in t[1])
    return 1


def saturate(eg, rules, max_iters, max_nodes):
    """Run e-saturation: each iteration applies every DSR pattern rule to every
    matching e-class and fires β on every `App(Lam(_), _)`. Stops at fixpoint,
    when iteration cap is hit, or when the node cap is exceeded.

    Returns ("ok", iters) on saturation, ("iters", iters) if iteration cap hit,
    ("nodes", iters) if node cap hit."""
    for it in range(max_iters):
        if eg.total_nodes() > max_nodes:
            return ("nodes", it)
        unions_made = False

        # Gather all eclasses up front so we don't iterate while the graph
        # is being mutated mid-loop.
        eclass_ids = list(set(eg.find(e) for e in eg.eclass_nodes))

        # 1. Fire all DSR rules.
        for lhs, rhs in rules:
            for eid in eclass_ids:
                for subst in list(ematch(eg, lhs, eid, {})):
                    new_eid = instantiate(eg, rhs, subst)
                    if eg.union(eid, new_eid):
                        unions_made = True

        # 2. β-step: for every `App(Lam(body), arg)` enode, β-reduce and union.
        beta_targets = []
        for eid in eclass_ids:
            for op, kids in list(eg.eclass_nodes.get(eid, ())):
                if op != "app":
                    continue
                f_eid, a_eid = kids
                for f_op, f_kids in list(eg.eclass_nodes.get(eg.find(f_eid), ())):
                    if f_op == "lam":
                        beta_targets.append((eid, f_kids[0], a_eid))
                        break
        for app_eid, body_eid, arg_eid in beta_targets:
            body = extract_smallest(eg, body_eid)
            arg = extract_smallest(eg, arg_eid)
            if body is None or arg is None:
                continue
            reduced = beta_contract(body, arg)
            new_eid = eg.add_term(reduced)
            if eg.union(app_eid, new_eid):
                unions_made = True

        eg.rebuild()

        if not unions_made:
            return ("ok", it + 1)
    return ("iters", max_iters)


def equiv_under_rules(t1, t2, rules, max_iters, max_nodes):
    """True iff `t1` and `t2` end up in the same e-class after saturation.
    Returns (verdict_bool, status_string)."""
    eg = EGraph()
    eid1 = eg.add_term(t1)
    eid2 = eg.add_term(t2)
    eg.rebuild()
    status, iters = saturate(eg, rules, max_iters, max_nodes)
    same = eg.find(eid1) == eg.find(eid2)
    return same, f"{status} after {iters} iters ({eg.total_nodes()} nodes)"


# ---------- pretty-printing (for mismatch reports) ----------

def render(t):
    tag = t[0]
    if tag == "var":
        return f"${t[1]}"
    if tag == "sym":
        return t[1]
    if tag == "lam":
        return f"(lam {render(t[1])})"
    if tag == "app":
        return f"(@ {render(t[1])} {render(t[2])})"
    if tag == "programs":
        return "(programs " + " ".join(render(c) for c in t[1]) + ")"
    return repr(t)


# ---------- driver ----------

def check_pair_beta(o, r, fuel):
    """β-only equivalence check. Returns (ok, msg)."""
    o_nf, fo = beta_normalize(o, fuel)
    r_nf, fr = beta_normalize(r, fuel)
    if fo == 0 or fr == 0:
        return False, f"β fuel exhausted (orig left {fo}, rewr left {fr})"
    if o_nf != r_nf:
        return False, f"mismatch\n  orig nf: {render(o_nf)}\n  rewr nf: {render(r_nf)}"
    return True, "ok"


def check_file(path, args):
    with open(path) as f:
        data = json.load(f)
    library_entries = data.get("library", [])
    if library_entries and not any(e.get("lambda") for e in library_entries):
        print(f"{path}: library has no `lambda` fields (non-lambda-calc run); skipping.")
        return True

    lib = build_library(library_entries)
    originals = data["original_programs"]
    rewritten = data["rewritten_programs"]
    if len(originals) != len(rewritten):
        print(f"{path}: original/rewritten length mismatch ({len(originals)} vs {len(rewritten)})")
        return False

    rules = parse_rewrites(args.rewrites) if args.rewrites else None

    ok = True
    for i, (o_str, r_str) in enumerate(zip(originals, rewritten)):
        o = parse_term(o_str)
        r = inline_symbols(parse_term(r_str), lib)
        if rules is None:
            pair_ok, msg = check_pair_beta(o, r, args.fuel)
        else:
            same, status = equiv_under_rules(o, r, rules, args.iters, args.nodes)
            pair_ok = same
            msg = "ok" if same else f"not unified ({status})"
            if args.verbose and same:
                msg = f"ok ({status})"
        if not pair_ok:
            ok = False
            print(f"{path}[{i}]: {msg}")
            print(f"  original : {o_str}")
            print(f"  rewritten: {r_str}")
        elif args.verbose:
            print(f"{path}[{i}]: {msg}")
    if ok and args.verbose:
        print(f"{path}: all {len(originals)} programs equivalent.")
    return ok


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("paths", nargs="+", help="*.out.json files to check")
    ap.add_argument("--rewrites", help="DSR file (`name: lhs => rhs` / `<=>`); enables e-graph mode")
    ap.add_argument("--fuel", type=int, default=100_000, help="β-only mode: max β steps per program")
    ap.add_argument("--iters", type=int, default=30, help="e-graph mode: max saturation iterations")
    ap.add_argument("--nodes", type=int, default=10_000, help="e-graph mode: max enodes before bailing")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    all_ok = True
    for p in args.paths:
        if not check_file(p, args):
            all_ok = False
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
