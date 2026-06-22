#!/usr/bin/env python3
"""Convert real FPBench FPCore benchmarks (s-expr numerical kernels) into an
egg-stitch lambda-calc corpus. Faithful: NO inlining and NO dropping of control
flow. Arithmetic ops map to float ops; `if`/comparisons/other functions become
plain leaf-op nodes (parser curries them); binder forms (`let`, `let*`, `while`,
`while*`, `for`, `tensor`) are encoded structurally with real `lam` binders and
de Bruijn `$n`. Reads /tmp/fpb/*.fpcore, writes data/domains/fpbench/all.json."""
import glob, json, os, re

SRC = "/tmp/fpb"
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "data", "domains", "fpbench", "all.json")

# arithmetic/transcendental FPCore ops -> egg-stitch float leaf ops
OPS = {"+": "+.", "-": "-.", "*": "*.", "/": "/.",
       "sqrt": "sqrt", "exp": "exp", "log": "ln", "pow": "power",
       "sin": "sin", "cos": "cos", "tan": "tan", "atan": "atan",
       "asin": "asin", "acos": "acos", "sinh": "sinh", "cosh": "cosh",
       "tanh": "tanh", "fabs": "abs"}
CONSTS = {"PI": "pi"}            # other free atoms pass through verbatim as leaves
BINDERS = {"while", "while*", "for", "for*", "tensor", "tensor*"}


def tokenize(s):
    s = re.sub(r";[^\n]*", "", s)
    s = s.replace("[", "(").replace("]", ")")
    toks, i = [], 0
    while i < len(s):
        c = s[i]
        if c in "() \t\n":
            if c not in " \t\n":
                toks.append(c)
            i += 1
        elif c == '"':
            j = s.index('"', i + 1); toks.append(s[i:j + 1]); i = j + 1
        else:
            j = i
            while j < len(s) and s[j] not in '() \t\n"':
                j += 1
            toks.append(s[i:j]); i = j
    return toks


def parse(toks):
    pos = 0
    def rd():
        nonlocal pos
        t = toks[pos]; pos += 1
        if t == "(":
            lst = []
            while toks[pos] != ")":
                lst.append(rd())
            pos += 1
            return lst
        return t
    forms = []
    while pos < len(toks):
        forms.append(rd())
    return forms


NUM = re.compile(r"^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$|^[+-]?\d+/\d+$")


def fmt_num(tok):
    if "/" in tok:
        a, b = tok.split("/"); return repr(float(a) / float(b))
    return repr(float(tok))


def db(env, name):
    return f"${len(env) - 1 - max(i for i, v in enumerate(env) if v == name)}"


def lams(s, n):
    for _ in range(n):
        s = f"(lam {s})"
    return s


class Skip(Exception):
    pass


def conv(e, env):
    """FPCore expr -> egg-stitch s-expr string; env = names in scope (inner last)."""
    if isinstance(e, str):
        if NUM.match(e):
            return fmt_num(e)
        if e in env:
            return db(env, e)
        return CONSTS.get(e, e)                     # free atom = constant leaf
    if not e:
        raise Skip("empty")
    head = e[0]
    if isinstance(head, list):
        raise Skip("application with computed head")

    if head in ("let", "let*"):
        # `(let value (lam body))`: a `let` leaf node; body sits under a real lam
        # (arg position, allowed), so the binding is faithful with no inlining and
        # no beta-redex. Progressive env covers both let and let* (parallel rhs
        # never reference sibling binders, so the extra binders only shift indices).
        binds, body = e[1], e[2]
        cur, vals = env[:], []
        for b in binds:
            vals.append(conv(b[1], cur)); cur = cur + [b[0]]
        out = conv(body, cur)
        for val in reversed(vals):
            out = f"(let {val} (lam {out}))"
        return out

    if head in BINDERS:
        # (while cond ([v init update]...) body)  — structural, vars as lam binders
        binds, body = e[2], e[3]
        n = len(binds)
        inits = [conv(b[1], env) for b in binds]
        new = env + [b[0] for b in binds]
        cond = lams(conv(e[1], new), n)
        updates = [lams(conv(b[2], new), n) for b in binds]
        bod = lams(conv(body, new), n)
        parts = [head] + inits + [cond] + updates + [bod]
        return "(" + " ".join(parts) + ")"

    if head == "!":                                 # annotation wrapper: drop props
        rest = e[1:]; k = 0
        while k < len(rest) and isinstance(rest[k], str) and rest[k].startswith(":"):
            k += 2
        return conv(rest[k], env)

    if head == "-" and len(e) == 2:
        return f"(-. 0.0 {conv(e[1], env)})"

    op = OPS.get(head, head)                         # arithmetic mapped; else leaf verbatim
    return "(" + op + " " + " ".join(conv(a, env) for a in e[1:]) + ")"


def main():
    progs, kept, skipped = [], 0, 0
    for path in sorted(glob.glob(f"{SRC}/*.fpcore")):
        for form in parse(tokenize(open(path).read())):
            if not isinstance(form, list) or form[0] != "FPCore":
                continue
            idx = 1
            if isinstance(form[idx], str):              # optional symbolic name
                idx += 1
            args = form[idx]; rest = form[idx + 1:]
            k = 0
            while k < len(rest) and isinstance(rest[k], str) and rest[k].startswith(":"):
                k += 2
            if k >= len(rest):
                skipped += 1; continue
            try:
                s = lams(conv(rest[k], list(args)), len(args))
            except Skip:
                skipped += 1; continue
            progs.append(s); kept += 1
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    json.dump(progs, open(OUT, "w"), indent=0)
    print(f"kept {kept}, skipped {skipped} -> {OUT}")
    for p in progs[:6]:
        print("  ", p[:150])


if __name__ == "__main__":
    main()
