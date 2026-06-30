# examples-paper

A minimal worked example (in the shape of the e-stitch figure-1 running example)
showing that a compressive *commutative ordering* need not fall out of
canonicalization — so library learning with **live** rewrites beats "minimize
first, then abstract" (`--only-use-dsrs-at-start`).

The only rewrite is commutativity of `+` (`rules.rewrites`):

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
```

The shared abstraction is

```
f0 = (+ ?x (* ?y ?y))      // x + y², arity 2
```

Its `?x` slot is filled with varied per-program subterms — bare symbols and
larger `(* ..)` terms — and some programs are wrapped in an outer function
(`sqrt`, `f1`).

- **`corpus_a.json`** — every program is written with the square *second*,
  matching `f0`, so a plain *syntactic* (rule-free) search finds it.

- **`corpus_b.json`** — the same programs, commutatively scrambled: half put the
  square first. Because *both* operands of each `+` are per-program subterms (no
  shared anchor leaf), the left operand is parsed first and gets the smaller
  e-class id, so egg's min-term extractor keeps each `+` in its written
  orientation — it does **not** re-align them. With a balanced split, no single
  orientation wins, so a search over the minimal corpus falls back to the weaker
  `(* ?y ?y)` and misses the `+`. Live commutativity re-aligns every program and
  recovers the full `f0`.

So `--only-use-dsrs-at-start`, which abstracts over the extracted minimal corpus,
is stuck with the scrambled B, while live commutativity recovers A's compression.

Measured with best-first, `--max-arity 2` (four programs):

| corpus | rule-free | `--only-use-dsrs-at-start` | live |
|--------|-----------|----------------------------|------|
| A      | ~1.14× (`(+ ?x (* ?y ?y))`) | ~1.14× | ~1.14× |
| B      | ~1.04× (`(* ?y ?y)`)        | ~1.04× | ~1.14× (`(+ (* ?y ?y) ?x)`) |

The headline distinction is *which* abstraction is found: A and live-B recover
the full `(+ ?x (* ?y ?y))`; B's minimal corpus only yields bare `(* ?y ?y)`.

`tests/example_paper_test.rs` pins this: A ≡ B under commutativity, B is
size-minimal, syntactic search compresses A but not B, and live rewriting beats
`--only-use-dsrs-at-start` on B.
