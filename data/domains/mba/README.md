# MBA live-vs-at-start abstraction experiment

A classical-programming domain where keeping rewrite rules **live** during
abstraction search recovers the shared idiom — and compresses more at every
step — than applying the rules once up front (`--only-use-dsrs-at-start`). Here
the rules are *essential*: the shared structure is genuinely hard to find
without them, and applying them once and freezing each program to a min-size
term actively **destroys** the cross-program alignment. Same signature as the
circuits and molecules domains.

## Corpus

`mba.json` — 400 Mixed Boolean-Arithmetic expressions from the **SiMBA** datasets
(`e1_2vars` + `e1_3vars`):

> Reichenwallner & Meerwald-Stadler, *Efficient Deobfuscation of Linear Mixed
> Boolean-Arithmetic Expressions*, CheckMATE 2022
> (`github.com/DenuvoSoftwareSolutions/SiMBA`, `datasets/`).

MBA obfuscation rewrites a simple expression into syntactically-disjoint
equivalents — the 2-var set is hundreds of forms of `x+y`
(`(x&y)+(x|y)`, `(x^y)+2*(x&y)`, `(~y|x) - (~y) + ...`, …). The equivalences hold
only through the **mixed boolean+arithmetic algebra**, so the shared structure is
latent behind a non-trivial equivalence (nobody factored it) — exactly what
distinguishes live from at-start.

Encoding (`scripts/mba_corpus.py`, regenerates from the SiMBA source; the frozen
JSON here is what the experiment uses): an infix parser (C precedence) emits
op-children trees over `+ - * & | ^ ~ neg` with variables/constants as leaves,
filtered to moderate size (6–40 nodes) and deduplicated.

## Rewrite rules — `mba.rewrites`

The MBA algebra, **size-tied / expansive and non-confluent** (the kind a min-size
min-term cannot exploit): commutativity/associativity of `+ * & | ^`; the boolean
identities (De Morgan, idempotence, complement, absorption, xnor); boolean
**distribution** (`a&(b|c) = (a&b)|(a&c)`, expansive — a live-only lever); and the
arithmetic↔boolean **bridges** that are the actual deobfuscation gadgets
(`(x&y)+(x|y) = x+y`, `(x^y)+2*(x&y) = x+y`, `a|b = (a+b)-(a&b)`,
`a^b = (a+b)-2*(a&b)`, two's-complement `~a = -a-1`, `*`-over-`+` distribution, …).
The distribution rules are expansive, so the e-graph is bounded with
`--node-limit 100000 --iter-limit 2` (both well below where results change).

## Configuration: `--max-arity 2 --no-zero-arity`

Abstractions must have 1–2 parameters. As in the regex domain the arity cap is a
*meaningfulness filter* — the recognizable MBA idioms (the linear-MBA basis, the
bitwise primitives) are arity 1–2, while higher-arity abstractions are syntactic
glue.

## Reproduce

```bash
cargo build --release
python3 scripts/mba_live_vs_atstart.py
```

Prints the per-step cumulative cost and compression ratio for all three modes,
then the live vs at-start abstractions rendered back to infix MBA.

## Results

Best-first, `--max-arity 2 --no-zero-arity`, 20 abstractions, initial cost 6600:

| mode | final cost | compression | vs baseline |
|---|---|---|---|
| baseline (no rules) | 3356 | 1.967× | — |
| at-start | 3640 | 1.813× | **worse** (+284) |
| **live** | **3249** | **2.031×** | −107 |

Live gap over at-start: **391** (~11%). Live **leads at every one of the 20
steps** and is the only mode to cross **2×** compression. The key facts:

- **at-start is worse than baseline the whole way.** Applying the algebra once
  and collapsing each program to its min-size term destroys the cross-program
  alignment (1.813× vs 1.967×) — the rules pay off *only* if kept live.
- **Distribution is live-exclusive.** It helps live and does nothing for at-start
  (a min-size extractor never takes the larger side of `a&(b|c) = (a&b)|(a&c)`) —
  the expansion case of the min-size principle.

What live learns (rendered to infix, `#k` = parameter, `m` = reuse):

- **The linear-MBA basis** `#0*(x^y) + #1*(x&y)` — live's **rank-1, m=97** idiom:
  the exact `a·(x⊕y) + b·(x∧y)` skeleton SiMBA-style deobfuscators solve for, as a
  single arity-2 template covering ~97 obfuscated forms.
- the OR/AND-coefficient gadgets `#0 + #1*(x|y)` (m=224), `#0 + #1*(x&y)` (m=74),
  and the bitwise primitives `(x&y)+#0`, `(x|y)+#0`, `(x^~y)+#0` as parametric
  idioms.

at-start finds the basis too, but only at **rank 2 with lower reuse (m=65)** —
its top idioms are the *single-coefficient fragments* (`#0+(x|y)*#1`, `(x&y)*#0`,
`(x^y)*#0+#1`). Collapsing each program to one min-size spelling means the two
coefficient slots rarely co-occur in a single frozen tree, so the joint basis
can't surface early or with high support. Live keeps all equivalent spellings
alive, the slots line up across programs, and the basis becomes the #1 idiom.
That is the qualitative win: **live learns the MBA basis; at-start learns its
fragments** — the same shape as circuits (live learns the factored gate,
at-start the expanded pieces).
