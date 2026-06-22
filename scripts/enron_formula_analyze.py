#!/usr/bin/env python3
"""Parse Excel formulas from converted Enron .xlsx files and measure whether they
carry the property we want: many distinct formulas sharing a common structure
that is NOT canonical. Reports size/depth distribution, recurring structural
sketches (cell refs + constants collapsed to leaf kinds), and how much an AC
normalization (sorting +/*/& operands) collapses distinct sketches — the gap is
the live-DSR opportunity that at-start can't capture."""
import glob, re, sys, collections, openpyxl

# ---------- tokenizer ----------
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


def tok(s):
    out, i = [], 0
    while i < len(s):
        m = TOK.match(s, i)
        if not m:
            raise ValueError(f"tok fail at {s[i:i+10]!r}")
        i = m.end()
        k = m.lastgroup
        if k == "ws":
            continue
        out.append((k, m.group()))
    return out


# ---------- Pratt parser ----------
LBP = {"=": 1, "<>": 1, "<": 1, ">": 1, "<=": 1, ">=": 1, "&": 2,
       "+": 3, "-": 3, "*": 4, "/": 4, "^": 5, ":": 7}


class P:
    def __init__(self, ts): self.ts, self.i = ts, 0
    def peek(self): return self.ts[self.i] if self.i < len(self.ts) else (None, None)
    def next(self): t = self.ts[self.i]; self.i += 1; return t

    def parse(self):
        e = self.expr(0)
        if self.i != len(self.ts):
            raise ValueError("trailing")
        return e

    def expr(self, rbp):
        left = self.nud()
        while True:
            k, v = self.peek()
            if k == "op" and LBP.get(v, 0) > rbp:
                self.next(); right = self.expr(LBP[v]); left = ("op:" + v, left, right)
            elif k == "punct" and v == ":" and LBP[":"] > rbp:
                self.next(); right = self.expr(LBP[":"]); left = ("range", left, right)
            elif k == "op" and v == "%":              # postfix
                self.next(); left = ("pct", left)
            else:
                return left

    def nud(self):
        k, v = self.next()
        if k == "num": return ("NUM",)
        if k == "str": return ("STR",)
        if k == "bool": return ("BOOL",)
        if k == "err": return ("ERR",)
        if k == "ref": return ("REF",)
        if k == "name": return ("NAME",)
        if k == "sheetref":                            # sheet prefix then a ref/name
            return self.nud()
        if k == "op" and v in ("-", "+"):
            return ("neg", self.expr(6)) if v == "-" else self.expr(6)
        if k == "punct" and v == "(":
            e = self.expr(0); self.expect(")"); return e
        if k == "func":
            name = v[:-1].upper(); args = []
            if not (self.peek() == ("punct", ")")):
                args.append(self.expr(0))
                while self.peek() == ("punct", ","):
                    self.next(); args.append(self.expr(0))
            self.expect(")")
            return ("fn:" + name, *args)
        raise ValueError(f"nud {k}:{v}")

    def expect(self, ch):
        k, v = self.next()
        if v != ch: raise ValueError(f"want {ch} got {v}")


def parse(f):
    return P(tok(f[1:])).parse()                        # drop leading '='


LEAVES = {"NUM", "STR", "BOOL", "ERR", "REF", "NAME"}
COMM = {"op:+", "op:*", "op:&"}


def size(t): return 1 if t[0] in LEAVES else 1 + sum(size(c) for c in t[1:])
def depth(t): return 0 if t[0] in LEAVES else 1 + max((depth(c) for c in t[1:]), default=0)
def ops(t): return 0 if t[0] in LEAVES else 1 + sum(ops(c) for c in t[1:])


def sketch(t):
    if t[0] in LEAVES: return t[0]
    return t[0] + "(" + ",".join(sketch(c) for c in t[1:]) + ")"


def ac_flatten(t):
    """Proper AC normal form: flatten nested +/*/& chains into one n-ary node and
    sort operands, recursively. So a+(b+c) == (a+b)+c == c+(b+a)."""
    if t[0] in LEAVES:
        return t
    kids = [ac_flatten(c) for c in t[1:]]
    if t[0] in COMM:
        flat = []
        for k in kids:
            if k[0] == t[0]:
                flat.extend(k[1:])
            else:
                flat.append(k)
        flat.sort(key=lambda x: sketch(x))
        return (t[0], *flat)
    return (t[0], *kids)


def ac_sketch(t):
    return sketch(ac_flatten(t))


def main():
    files = sorted(glob.glob("/tmp/enron/xlsx2/*.xlsx")) or sorted(glob.glob("/tmp/enron/xlsx/*.xlsx"))
    raw, parsed, unparsed = 0, [], 0     # parsed: list of (tree, filebasename)
    for path in files:
        base = path.split("/")[-1]
        try:
            wb = openpyxl.load_workbook(path, data_only=False, read_only=True)
        except Exception:
            continue
        for ws in wb.worksheets:
            for row in ws.iter_rows():
                for c in row:
                    if isinstance(c.value, str) and c.value.startswith("=") and len(c.value) > 1:
                        raw += 1
                        try:
                            parsed.append((parse(c.value), base))
                        except Exception:
                            unparsed += 1
        wb.close()
    trees = [t for t, _ in parsed]
    print(f"files={len(files)} formula_cells={raw} parsed={len(trees)} unparsed={unparsed} "
          f"(coverage {100*len(trees)/max(raw,1):.0f}%)\n")

    sizes = [size(t) for t in trees]
    import statistics as st
    print(f"size: median={st.median(sizes):.0f} mean={st.mean(sizes):.1f} max={max(sizes)} "
          f">=5 nodes: {sum(s>=5 for s in sizes)} ({100*sum(s>=5 for s in sizes)/len(sizes):.0f}%)  "
          f">=10: {sum(s>=10 for s in sizes)}")
    print(f"depth>=3: {sum(depth(t)>=3 for t in trees)}  ops>=4: {sum(ops(t)>=4 for t in trees)}\n")

    NT = [(t, f) for t, f in parsed if size(t) >= 7]      # deep tail with provenance
    print(f"=== non-trivial tail (>=7 nodes): {len(NT)} formulas ===")
    raw_sk = collections.Counter(sketch(t) for t, _ in NT)
    ac_sk = collections.Counter(ac_sketch(t) for t, _ in NT)
    print(f"unique raw sketches:           {len(raw_sk)}")
    print(f"unique AC-normalized (flat):   {len(ac_sk)}   "
          f"(proper AC flatten+sort merges {len(raw_sk)-len(ac_sk)} surface forms)\n")

    # Provenance: for each raw sketch, how many DISTINCT files? (fill-down vs cross-author)
    files_per = collections.defaultdict(set)
    for t, f in NT:
        files_per[sketch(t)].add(f)
    single_file = [sk for sk in raw_sk if len(files_per[sk]) == 1]
    print(f"raw sketches confined to ONE file (fill-down): {len(single_file)}/{len(raw_sk)}; "
          f"formulas they cover: {sum(raw_sk[sk] for sk in single_file)}/{len(NT)}\n")

    print("top recurring raw sketches  (count, #files : structure):")
    for sk, n in raw_sk.most_common(15):
        print(f"  {n:5} in {len(files_per[sk]):3} files : {sk[:96]}")


if __name__ == "__main__":
    main()
