#!/usr/bin/env python3
"""Exhaustive boolean-equivalence checker for the epfl-circuits domain.

The general `check_equiv.py` proves a rewrite sound by re-deriving the
equivalence through rule-saturation, but the De Morgan + *directional*
distributivity DSRs can't always bridge an aggressively-factored cone back to
its original within a practical egraph (it hits a fixpoint without unifying).

For boolean circuits that's overkill anyway: each cone is a boolean function of
at most K input signals (`$0..$k`, K=6), so we can settle equivalence
*definitively* by evaluating both sides on all 2^k assignments. For each fixture
we inline the abstraction library into every rewritten program and compare its
truth table to the original's.

Usage: check_circuit_equiv.py <fixture.out.json> ...
Exit 0 iff every program's rewritten form is boolean-equivalent to its original.
"""
import sys, json, re, itertools

# Guards against an unexpectedly wide cone blowing up the truth-table sweep
# (epfl cones are K=6-bounded by construction, so this should never trip).
MAX_VARS = 16


def tokenize(s):
    return s.replace("(", " ( ").replace(")", " ) ").split()


def parse(toks, i):
    if toks[i] == "(":
        i += 1
        head = toks[i]
        i += 1
        kids = []
        while toks[i] != ")":
            k, i = parse(toks, i)
            kids.append(k)
        return (head, kids), i + 1
    return (toks[i], []), i + 1


def parse_sexp(s):
    return parse(tokenize(s), 0)[0]


def build_lib(library):
    """`fn_N` -> body AST, where each argument slot is a hole `('?#k', [])`."""
    lib = {}
    for e in library:
        name, _, pat = e["pattern"].partition(": ")
        lib[name] = parse_sexp(pat or name)
    return lib


def subst_holes(t, args):
    head, kids = t
    m = re.fullmatch(r"\?#(\d+)", head)
    if m:
        return args[int(m.group(1))]
    return (head, [subst_holes(k, args) for k in kids])


def inline(t, lib):
    """Fully expand `fn_N(args...)` calls into primitive and/or/not/C0/$n terms.
    Bodies may reference earlier fns, so re-inline the substituted result."""
    head, kids = t
    kids = [inline(k, lib) for k in kids]
    if head in lib:
        return inline(subst_holes(lib[head], kids), lib)
    return (head, kids)


def ev(t, env):
    head, kids = t
    if head == "and":
        return all(ev(k, env) for k in kids)
    if head == "or":
        return any(ev(k, env) for k in kids)
    if head == "not":
        return not ev(kids[0], env)
    if head == "C0":
        return False
    if head.startswith("$"):
        return env[head]
    raise ValueError(f"unexpected op/leaf {head!r} (holes should be inlined)")


def free_vars(t, acc):
    head, kids = t
    if head.startswith("$"):
        acc.add(head)
    for k in kids:
        free_vars(k, acc)
    return acc


def equivalent(orig_ast, rew_ast):
    """True iff the two ASTs agree on every assignment of their shared vars."""
    vs = sorted(free_vars(orig_ast, set()) | free_vars(rew_ast, set()))
    if len(vs) > MAX_VARS:
        raise SystemExit(f"cone has {len(vs)} vars (> {MAX_VARS}); refusing 2^n sweep")
    for combo in itertools.product((False, True), repeat=len(vs)):
        env = dict(zip(vs, combo))
        if ev(orig_ast, env) != ev(rew_ast, env):
            return False, env
    return True, None


def main():
    total = mismatches = 0
    for path in sys.argv[1:]:
        d = json.load(open(path))
        lib = build_lib(d["library"])
        bad = 0
        for i, (o, r) in enumerate(zip(d["original_programs"], d["rewritten_programs"])):
            ok, env = equivalent(parse_sexp(o), inline(parse_sexp(r), lib))
            if not ok:
                bad += 1
                if bad <= 3:
                    print(f"  MISMATCH {path}[{i}] at {env}")
            total += 1
        mismatches += bad
        name = path.split("/")[-1]
        print(f"  {name:42} {len(d['original_programs']):4} programs, {bad} mismatches")
    print(f"checked {total} programs, {mismatches} boolean mismatches")
    sys.exit(1 if mismatches else 0)


if __name__ == "__main__":
    main()
