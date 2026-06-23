#!/usr/bin/env python3
"""Build the regex corpus for the live-vs-at-start experiment from REAL regexes
mined out of the locally-installed Python libraries (string literals passed to
re.* calls), encoded as egg-stitch op-children trees over Cat/Alt/Star/Plus/Opt
with interned leaf tokens. Skips regexes that use features outside the supported
subset (lookaround, named groups, backrefs).

NOTE: the harvested set depends on which Python packages are installed, so this
is *provenance*, not a reproducible build — the frozen corpus that the experiment
actually uses is checked in at data/domains/regex/regex.json (+ regex_legend.json).
Re-running this overwrites those with whatever this machine's libraries yield."""
import ast, glob, json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTDIR = os.path.join(ROOT, "data", "domains", "regex")
STD = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.dirname(os.path.abspath(__import__("re").__file__)))

def harvest(root):
    pats = set()
    for path in [p for p in glob.glob(f"{root}/**/*.py", recursive=True) if "test" not in p]:
        try:
            tree = ast.parse(open(path, encoding="utf-8", errors="ignore").read())
        except Exception:
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) \
               and isinstance(node.func.value, ast.Name) and node.func.value.id == "re" \
               and node.func.attr in ("compile", "match", "search", "fullmatch", "sub", "findall", "split"):
                idx = 1 if node.func.attr == "sub" else 0
                if len(node.args) > idx and isinstance(node.args[idx], ast.Constant) and isinstance(node.args[idx].value, str):
                    pats.add(node.args[idx].value)
    return sorted(pats)

class Skip(Exception): pass

class RX:
    def __init__(s, t): s.t = t; s.i = 0
    def peek(s): return s.t[s.i] if s.i < len(s.t) else None
    def eat(s): c = s.t[s.i]; s.i += 1; return c
    def alt(s):
        parts = [s.seq()]
        while s.peek() == "|": s.eat(); parts.append(s.seq())
        node = parts[-1]
        for p in reversed(parts[:-1]): node = ("Alt", p, node)
        return node
    def seq(s):
        items = []
        while s.peek() is not None and s.peek() not in "|)": items.append(s.post())
        if not items: return ("Eps",)
        node = items[-1]
        for it in reversed(items[:-1]): node = ("Cat", it, node)
        return node
    def post(s):
        a = s.atom(); c = s.peek()
        if c == "*": s.eat(); return ("Star", a)
        if c == "+": s.eat(); return ("Plus", a)
        if c == "?": s.eat(); return ("Opt", a)
        if c == "{":
            j = s.t.index("}", s.i); q = s.t[s.i:j+1]; s.i = j+1; return ("Rep", a, ("lit", q))
        return a
    def atom(s):
        c = s.eat()
        if c == "(":
            if s.t[s.i:s.i+1] == "?":
                if s.t[s.i:s.i+2] == "?:": s.i += 2
                else: raise Skip("group ext")
            node = s.alt()
            if s.peek() != ")": raise Skip("unbalanced")
            s.eat(); return node
        if c == "[":
            j = s.i
            if s.t[j:j+1] == "^": j += 1
            if s.t[j:j+1] == "]": j += 1
            k = s.t.index("]", j); cls = s.t[s.i-1:k+1]; s.i = k+1; return ("lit", cls)
        if c == "\\":
            d = s.eat()
            if d.isdigit(): raise Skip("backref")
            return ("lit", "\\" + d)
        return ("lit", c)

def parse(rx):
    if any(x in rx for x in ("(?=", "(?!", "(?<", "(?P", "\\b")): raise Skip("hard feature")
    p = RX(rx); node = p.alt()
    if p.i != len(rx): raise Skip("trailing")
    return node

legend = {}
def leaf(tok):
    if tok not in legend: legend[tok] = f"L{len(legend)}"
    return legend[tok]
def size(n): return 1 + sum(size(c) for c in n[1:] if isinstance(c, tuple))
def emit(n):
    if n[0] == "lit": return leaf(n[1])
    if n[0] == "Eps": return "Eps"
    return "(" + n[0] + " " + " ".join(emit(c) for c in n[1:]) + ")"

pats = harvest(STD)
progs, kept = [], 0
for rx in pats:
    try:
        node = parse(rx)
        if size(node) < 4: continue
        progs.append(emit(node)); kept += 1
    except Exception:
        pass
# the experiment uses the alternation-bearing, moderate-size, deduped subset
cand = [p for p in progs if "Alt" in p and 4 <= p.count("(") <= 22]
seen = set(); uniq = [p for p in cand if not (p in seen or seen.add(p))][:250]
os.makedirs(OUTDIR, exist_ok=True)
json.dump(uniq, open(os.path.join(OUTDIR, "regex.json"), "w"), indent=0)
json.dump({v: k for k, v in legend.items()}, open(os.path.join(OUTDIR, "regex_legend.json"), "w"), indent=0)
print(f"harvested {len(pats)} literals; encoded {kept}; alt-bearing moderate uniq -> {len(uniq)} written to {OUTDIR}")
