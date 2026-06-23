#!/usr/bin/env python3
"""Higher-structure regex corpus from pygments lexers.

Pygments ships ~480 RegexLexers; their token tables contain thousands of regexes
that heavily *share* sub-structure (keyword char-chains, identifier classes,
number/string formats) both within and across languages. That shared structure
gives the live-vs-at-start abstraction experiment a much larger edge than the
grab-bag of regexes mined from arbitrary library `re.*` calls (see
scripts/regex_corpus.py): the live-vs-at-start gap roughly doubles, and
applying the rules up front (--only-use-dsrs-at-start) ends up *worse* than no
rules at all, while keeping them live is best.

Encodes the alternation-bearing, moderate-size, deduplicated subset to egg-stitch
op-children trees over Cat/Alt/Star/Plus/Opt/Rep with interned leaf tokens.
Output (env/pygments-version dependent — the frozen corpus is checked in):
  data/domains/regex/regex_pygments.json + regex_pygments_legend.json
"""
import json, os, sys
from pygments.lexers import _iter_lexerclasses
from pygments.lexer import RegexLexer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTDIR = os.path.join(ROOT, "data", "domains", "regex")
N = int(sys.argv[1]) if len(sys.argv) > 1 else 300

def harvest():
    raw = []
    for cls in _iter_lexerclasses():
        toks = getattr(cls, "tokens", None)
        if not (issubclass(cls, RegexLexer) and isinstance(toks, dict)):
            continue
        for rules in toks.values():
            for r in rules:
                if isinstance(r, tuple) and r and isinstance(r[0], str):
                    raw.append(r[0])
    return list(dict.fromkeys(raw))

class Skip(Exception): pass

class RX:
    def __init__(s, t): s.t = t; s.i = 0
    def peek(s): return s.t[s.i] if s.i < len(s.t) else None
    def eat(s): c = s.t[s.i]; s.i += 1; return c
    def alt(s):
        ps = [s.seq()]
        while s.peek() == "|": s.eat(); ps.append(s.seq())
        n = ps[-1]
        for p in reversed(ps[:-1]): n = ("Alt", p, n)
        return n
    def seq(s):
        it = []
        while s.peek() is not None and s.peek() not in "|)": it.append(s.post())
        if not it: return ("Eps",)
        n = it[-1]
        for x in reversed(it[:-1]): n = ("Cat", x, n)
        return n
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
            n = s.alt()
            if s.peek() != ")": raise Skip("unbalanced")
            s.eat(); return n
        if c == "[":
            st = s.i - 1; j = s.i
            if s.t[j:j+1] == "^": j += 1
            if s.t[j:j+1] == "]": j += 1
            while j < len(s.t) and s.t[j] != "]":
                j += 2 if s.t[j] == "\\" else 1
            if j >= len(s.t): raise Skip("class")
            cls = s.t[st:j+1]; s.i = j + 1; return ("lit", cls)
        if c == "\\":
            d = s.eat()
            if d.isdigit(): raise Skip("backref")
            return ("lit", "\\" + d)
        return ("lit", c)

def parse(rx):
    if any(x in rx for x in ("(?=", "(?!", "(?<", "(?P", "(?i", "(?s", "(?m", "(?x", "\\b", "\\B")):
        raise Skip("hard feature")
    p = RX(rx); n = p.alt()
    if p.i != len(rx): raise Skip("trailing")
    return n

legend = {}
def leaf(t):
    if t not in legend: legend[t] = f"L{len(legend)}"
    return legend[t]
def size(n): return 1 + sum(size(c) for c in n[1:] if isinstance(c, tuple))
def emit(n):
    if n[0] == "lit": return leaf(n[1])
    if n[0] == "Eps": return "Eps"
    return "(" + n[0] + " " + " ".join(emit(c) for c in n[1:]) + ")"

progs = []
for rx in harvest():
    try:
        n = parse(rx)
        if size(n) >= 4: progs.append(emit(n))
    except Exception:
        pass
cand = [p for p in progs if "Alt" in p and 4 <= p.count("(") <= 22]
seen = set(); uniq = [p for p in cand if not (p in seen or seen.add(p))][:N]
os.makedirs(OUTDIR, exist_ok=True)
json.dump(uniq, open(os.path.join(OUTDIR, "regex_pygments.json"), "w"), indent=0)
json.dump({v: k for k, v in legend.items()}, open(os.path.join(OUTDIR, "regex_pygments_legend.json"), "w"), indent=0)
print(f"encoded {len(progs)}; alt-bearing moderate uniq -> {len(uniq)} written to {OUTDIR}")
