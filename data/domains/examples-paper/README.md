# examples-paper

A minimal worked example (in the shape of the e-stitch figure-1 running example)
showing that a compressive abstraction need not fall out of canonicalization —
so library learning with **live** rewrites beats "minimize first, then abstract"
(`--only-use-dsrs-at-start`).

The rewrites (`rules.rewrites`) are commutativity of `+` and the additive
identity:

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
add_zero:  ?x       <=> (+ 0 ?x)
```

The shared abstraction is

```
f0 = (+ ?x (* ?y ?y))      // x + y², arity 2
```

Its slots are filled with varied per-program subterms — bare symbols, larger
`(* ..)` / `(/ ..)` terms — and some programs are wrapped in an outer function
(`sqrt`, `f1`). Six programs are of `f0` shape; the seventh is the bare square
`(* (/ x 2) (/ x 2))`, which has no `+` and only fits `f0` after an `add_zero`
expansion: `(+ 0 (* (/ x 2) (/ x 2))) = (f0 0 (/ x 2))`.

- **`corpus_a.json`** — every `f0` program is written with the square *second*,
  so a plain *syntactic* (rule-free) search finds `f0`.

- **`corpus_b.json`** — the same programs, commutatively scrambled (half put the
  square first). Because *both* operands of each `+` are per-program subterms (no
  shared anchor leaf), the left operand is parsed first and gets the smaller
  e-class id, so egg's min-term extractor keeps each `+` in its written
  orientation — it does **not** re-align them. With a balanced split, no single
  orientation wins, so a search over the minimal corpus falls back to the weaker
  `(* ?y ?y)` squaring. Live rewriting re-aligns every `+` *and* uses `add_zero`
  on the bare square, recovering the full `f0` across all programs.

So `--only-use-dsrs-at-start`, which abstracts over the extracted minimal corpus,
is stuck with bare squaring, while live rewriting recovers the richer, more
compressive `f0`.

Measured with best-first, `--max-arity 2` (seven programs):

| corpus | rule-free | `--only-use-dsrs-at-start` | live |
|--------|-----------|----------------------------|------|
| A      | ~1.19× (`(+ ?x (* ?y ?y))`) | — | — |
| B      | ~1.16× (`(* ?y ?y)`)        | ~1.16× | ~1.26× (`(+ (* ?y ?y) ?x)`) |

`tests/example_paper_test.rs` pins this: A ≡ B under the rewrites, B is
size-minimal, syntactic search finds `f0` on A but only squaring on B, and live
rewriting recovers `f0` on B (beating `--only-use-dsrs-at-start`).
