# examples-paper

A minimal worked example (in the shape of the e-stitch figure-1 running example)
showing that the abstraction-exposing corpus is **not** the minimal one, and its
compressive form does not fall out of canonicalization — so library learning
with **live** rewrites beats "minimize first, then abstract"
(`--only-use-dsrs-at-start`).

The rewrites (`rules.rewrites`) are:

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
add_zero:  ?x       <=> (+ 0 ?x)
neg_zero:  (- 0)    <=> 0
```

The shared abstraction is

```
f0 = (+ (- ?0) (* ?1 ?1))      // -x + y², arity 2
```

Both operands of the `+` are structured — a negation `(- ?0)` and a square
`(* ?1 ?1)` — so bare `(* ?1 ?1)` squaring is a much weaker fallback (it misses
the `-` and the `+`). Slots are filled with varied per-program subterms (bare
symbols, `(* ..)` / `(/ ..)` terms) and one program is wrapped in `sqrt`. The
last program buries a square with no `+` inside a larger `(exp ..)` term,
so the abstraction appears deep in a general program; that square fits `f0` only
through the two identities: `(* d d) = (+ 0 (* d d)) = (+ (- 0) (* d d))`, i.e.
`(f0 0 d)`.

- **`corpus_a.json`** — the *expanded* corpus, written to match `f0`
  syntactically: the `f0` programs put `(- ?0)` first, and the last buries
  `(+ (- 0) (* (/ x 2) (/ x 2)))` inside `(exp ..)`. A plain rule-free
  search finds `f0`. A is deliberately **not** minimal — that `(+ (- 0) ..)`
  collapses to `(* ..)`.

- **`corpus_b.json`** — the size-minimal form: the last program buries the bare
  square `(* (/ x 2) (/ x 2))` inside `(exp ..)`, and one `f0` program (the
  `sqrt`-wrapped one) is commutatively swapped to put the square first. Because *both* operands of each
  `+` are per-program subterms (no shared anchor leaf), the left operand is parsed
  first and gets the smaller e-class id, so egg's min-term extractor keeps each `+`
  in its written orientation — it does **not** re-align them. One swap is enough:
  an arity-2 abstraction body costs 6 nodes and saves only ~3 per use, so it needs
  *three* aligned uses to pay for itself, and the swap leaves at most two programs
  sharing any one orientation. So a search over the minimal corpus falls back to
  the weak `(* ?1 ?1)` squaring. Live rewriting re-aligns every `+` *and*
  `add_zero`/`neg_zero`-expands the bare square, lifting `f0` to four aligned uses
  and recovering it.

This is the paper's point that the abstraction-exposing corpus is not the minimal
one: rule-free A (cost 25) beats abstracting B's minimal term (at-start, cost
27), and live rewriting recovers A's cost (25) from B.

Measured with best-first, `--max-arity 2` (four programs):

| corpus | rule-free | `--only-use-dsrs-at-start` | live |
|--------|-----------|----------------------------|------|
| A      | ~1.32× (`(+ (- ?0) (* ?1 ?1))`) | — | — |
| B      | ~1.11× (`(* ?1 ?1)`)            | ~1.11× | ~1.20× (`(+ (- ?0) (* ?1 ?1))`) |

`tests/example_paper_test.rs` pins this: A ≡ B under the rewrites, B is minimal
while A is expanded, syntactic search finds `f0` on A but only squaring on B, and
live rewriting recovers `f0` on B (beating `--only-use-dsrs-at-start`).
