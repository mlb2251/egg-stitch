#!/usr/bin/env python3
"""Encode the RegExLib corpus into the egg-stitch regex domain.

Source: the 8,699 RegExLib regexes from the ReDoSHunter artifact
(github.com/yetingli/ReDoSHunter, data/paper_dataset/regexlib.txt), from
  Li et al., "ReDoSHunter: A Combined Static and Dynamic Approach for Regular
  Expression DoS Detection", USENIX Security 2021.
These are independently-authored regexes keyed by task (email/date/phone/...),
so equivalence between them is *latent* (different authors, different spellings),
not constructed -- which is what the live-vs-at-start experiment needs.

Encoding (legible, no interning):
 - leaf tokens are the raw regex fragment, percent-encoding only parser-unsafe
   chars (space, parens, quote, %);
 - positive character classes decompose into an Alt of their members
   (single-range-class leaves [a-z],[0-9],... or literal atoms), so existing Alt
   commutativity/associativity unifies all member reorderings ([a-zA-Z]==[A-Za-z])
   generically -- no per-spelling rules. Negated/POSIX classes stay opaque;
 - quantifiers x{n}/x{n,m} -> (Range x lo hi); Star/Plus/Opt kept (their relation
   to Range is a rewrite rule, not the encoding).

Usage: python3 scripts/regexlib_corpus.py [path/to/regexlib.txt] [N=400]
The frozen corpus (data/domains/regex/regexlib.json) is what the experiment uses."""
import html, json, os, re, sys

class Skip(Exception): pass
def safe(t):
    return (t.replace("%","%25").replace("(","%28").replace(")","%29")
             .replace('"',"%22").replace(" ","%20").replace("\t","%09").replace("\n","%0a"))

def class_members(body):
    """body = chars between [ ], not negated. Return list of ('range',lo,hi)|('atom',a)."""
    out=[]; i=0
    while i<len(body):
        if body[i]=="\\": a=body[i:i+2]; i+=2
        else: a=body[i]; i+=1
        if i+0<len(body) and body[i:i+1]=="-" and i+1<len(body) and body[i+1]!="]":
            i+=1
            if body[i]=="\\": hi=body[i:i+2]; i+=2
            else: hi=body[i]; i+=1
            out.append(("range",a,hi))
        else:
            out.append(("atom",a))
    return out

def class_node(body, negated):
    if negated or "[:" in body:                      # complement / POSIX -> opaque leaf
        return ("lit", "["+("^" if negated else "")+body+"]")
    ms=class_members(body)
    if not ms: raise Skip("empty class")
    def memleaf(m):
        if m[0]=="range": return ("lit", "["+m[1]+"-"+m[2]+"]")
        return ("lit", m[1])                          # atom: literal char or \X (safed in emit)
    if len(ms)==1: return memleaf(ms[0])
    node=memleaf(ms[-1])
    for m in reversed(ms[:-1]): node=("Alt", memleaf(m), node)
    return node

class RX:
    def __init__(s,t): s.t=t; s.i=0
    def peek(s): return s.t[s.i] if s.i<len(s.t) else None
    def eat(s): c=s.t[s.i]; s.i+=1; return c
    def alt(s):
        ps=[s.seq()]
        while s.peek()=="|": s.eat(); ps.append(s.seq())
        n=ps[-1]
        for p in reversed(ps[:-1]): n=("Alt",p,n)
        return n
    def seq(s):
        it=[]
        while s.peek() is not None and s.peek() not in "|)": it.append(s.post())
        if not it: return ("Eps",)
        n=it[-1]
        for x in reversed(it[:-1]): n=("Cat",x,n)
        return n
    def post(s):
        a=s.atom(); c=s.peek()
        if c=="*": s.eat(); return ("Star",a)
        if c=="+": s.eat(); return ("Plus",a)
        if c=="?": s.eat(); return ("Opt",a)
        if c=="{":
            s.eat(); j=s.t.index("}",s.i); body=s.t[s.i:j]; s.i=j+1
            if "," in body: lo,hi=body.split(",",1); lo=lo or "0"; hi=hi or "INF"
            else: lo=hi=body
            if not (lo.isdigit() and (hi=="INF" or hi.isdigit())): raise Skip("bad range")
            return ("Range",a,lo,hi)
        return a
    def atom(s):
        c=s.eat()
        if c=="(":
            if s.t[s.i:s.i+1]=="?":
                if s.t[s.i:s.i+2]=="?:": s.i+=2
                else: raise Skip("ext")
            n=s.alt()
            if s.peek()!=")": raise Skip("bal")
            s.eat(); return n
        if c=="[":
            neg=False; j=s.i
            if s.t[j:j+1]=="^": neg=True; j+=1
            st=j
            if s.t[j:j+1]=="]": j+=1
            while j<len(s.t) and s.t[j]!="]":
                j+=2 if s.t[j]=="\\" else 1
            if j>=len(s.t): raise Skip("class")
            body=s.t[st:j]; s.i=j+1
            return class_node(body, neg)
        if c=="\\":
            d=s.eat()
            if d.isdigit(): raise Skip("backref")
            return ("lit","\\"+d)
        return ("lit",c)

def parse(rx):
    if any(x in rx for x in ("(?=","(?!","(?<","(?P","(?i","(?s","(?m","(?x","\\b","\\B")): raise Skip("hard")
    p=RX(rx); n=p.alt()
    if p.i!=len(rx): raise Skip("trail")
    return n

def size(n): return 1+sum(size(c) for c in n[1:] if isinstance(c,tuple))
def emit(n):
    h=n[0]
    if h=="lit": return safe(n[1])
    if h=="Eps": return "Eps"
    if h=="Range": return f"(Range {emit(n[1])} {n[2]} {n[3]})"
    return "("+h+" "+" ".join(emit(c) for c in n[1:])+")"

SRC = sys.argv[1] if len(sys.argv) > 1 else "regexlib.txt"
N   = int(sys.argv[2]) if len(sys.argv) > 2 else 400
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "data", "domains", "regex", "regexlib.json")
lines=[html.unescape(l.strip()) for l in open(SRC) if l.strip()]
progs=[]
for rx in lines:
    try:
        n=parse(rx)
        if size(n)>=4: progs.append(emit(n))
    except Exception: pass
cand=[p for p in progs if "Alt" in p and 4<=p.count("(")<=22]   # alt-bearing, moderate size
seen=set(); uniq=[p for p in cand if not (p in seen or seen.add(p))]
json.dump(uniq[:N], open(OUT,"w"), indent=0)
print(f"encoded {len(progs)}; alt-bearing moderate uniq {len(uniq)}; wrote {min(N,len(uniq))} -> {OUT}")
