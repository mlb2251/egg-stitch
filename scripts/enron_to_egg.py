#!/usr/bin/env python3
"""Convert real Enron spreadsheet formulas into an egg-stitch op-children corpus.
Keeps real cell refs / numbers as leaves (so abstraction must introduce metavars
to share across them — the realistic case). Self-contained parser (keeps leaf
values). Filters to the non-trivial tail and samples to a tractable size."""
import glob, re, json, os, collections, openpyxl

TOK = re.compile(r"""
  (?P<ws>\s+)
| (?P<str>"(?:[^"]|"")*")
| (?P<err>\#[A-Z0-9_/]+[!?]?)
| (?P<bool>\bTRUE\b|\bFALSE\b)
| (?P<num>\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+)
| (?P<sheetref>'[^']+'!|\b[A-Za-z_][A-Za-z0-9_.]*!)
| (?P<ref>\$?[A-Z]{1,3}\$?\d+)
| (?P<func>[A-Za-z_][A-Za-z0-9_.]*\()
| (?P<name>[A-Za-z_][A-Za-z0-9_.]*)
| (?P<op><=|>=|<>|[-+*/^&=<>%])
| (?P<punct>[(),:])
""", re.VERBOSE)

OPNAME = {"+": "add", "-": "sub", "*": "mul", "/": "div", "^": "pow", "&": "concat",
          "=": "eq", "<>": "ne", "<": "lt", ">": "gt", "<=": "le", ">=": "ge"}
LBP = {"=": 1, "<>": 1, "<": 1, ">": 1, "<=": 1, ">=": 1, "&": 2,
       "+": 3, "-": 3, "*": 4, "/": 4, "^": 5}
LEAVES = {"num", "str", "bool", "err", "ref", "name"}


def tok(s):
    out, i = [], 0
    while i < len(s):
        m = TOK.match(s, i)
        if not m:
            raise ValueError("tok")
        i = m.end()
        if m.lastgroup != "ws":
            out.append((m.lastgroup, m.group()))
    return out


class P:
    def __init__(s, ts): s.ts, s.i = ts, 0
    def peek(s): return s.ts[s.i] if s.i < len(s.ts) else (None, None)
    def nx(s): t = s.ts[s.i]; s.i += 1; return t
    def parse(s):
        e = s.expr(0)
        if s.i != len(s.ts): raise ValueError("trail")
        return e
    def expr(s, rbp):
        left = s.nud()
        while True:
            k, v = s.peek()
            if k == "op" and LBP.get(v, 0) > rbp:
                s.nx(); left = (OPNAME[v], left, s.expr(LBP[v]))
            elif k == "punct" and v == ":" and 7 > rbp:
                s.nx(); left = ("range", left, s.expr(7))
            elif k == "op" and v == "%":
                s.nx(); left = ("pct", left)
            else:
                return left
    def nud(s):
        k, v = s.nx()
        if k in ("num", "str", "bool", "err", "ref", "name"): return (k, v)
        if k == "sheetref": return s.nud()
        if k == "op" and v in ("-", "+"):
            return ("neg", s.expr(6)) if v == "-" else s.expr(6)
        if k == "punct" and v == "(":
            e = s.expr(0); s.eat(")"); return e
        if k == "func":
            nm = v[:-1].upper(); args = []
            if s.peek() != ("punct", ")"):
                args.append(s.expr(0))
                while s.peek() == ("punct", ","):
                    s.nx(); args.append(s.expr(0))
            s.eat(")")
            return ("fn:" + nm, *args)
        raise ValueError("nud")
    def eat(s, ch):
        if s.nx()[1] != ch: raise ValueError("eat")


def parse(f): return P(tok(f[1:])).parse()
def size(t): return 1 if t[0] in LEAVES else 1 + sum(size(c) for c in t[1:])


def san(s):
    s = re.sub(r"[^A-Za-z0-9]", "", s)
    return s or "x"


def sexpr(t):
    """op-children s-expr string; real refs/numbers kept as leaves."""
    k = t[0]
    if k == "num": return t[1]
    if k == "ref": return san(t[1])                       # $A$1 -> A1
    if k == "bool": return t[1].upper()
    if k == "str": return "str"
    if k == "err": return "err"
    if k == "name": return "n_" + san(t[1])
    head = k[3:] if k.startswith("fn:") else k            # fn:IF -> IF
    return "(" + head + " " + " ".join(sexpr(c) for c in t[1:]) + ")"


def main():
    files = sorted(glob.glob("/tmp/enron/xlsx2/*.xlsx"))
    progs = []
    for path in files:
        try:
            wb = openpyxl.load_workbook(path, data_only=False, read_only=True)
        except Exception:
            continue
        for ws in wb.worksheets:
            for row in ws.iter_rows():
                for c in row:
                    if isinstance(c.value, str) and c.value.startswith("=") and len(c.value) > 1:
                        try:
                            t = parse(c.value)
                        except Exception:
                            continue
                        if size(t) >= 7:
                            progs.append(sexpr(t))
        wb.close()
    # deterministic stride-sample to ~3000, preserving recurrence
    N = 3000
    if len(progs) > N:
        step = len(progs) / N
        progs = [progs[int(i * step)] for i in range(N)]
    out = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                       "data", "domains", "enron", "all.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    json.dump(progs, open(out, "w"), indent=0)
    print(f"wrote {len(progs)} programs -> {out}")
    print(f"distinct programs: {len(set(progs))}")
    for p in progs[:6]:
        print("  ", p[:120])


if __name__ == "__main__":
    main()
