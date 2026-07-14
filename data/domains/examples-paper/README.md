Worked example for use as e-stitch figure-1.

In this case, the abstraction-exposing corpus is
not the minimal term.

Shared abstraction `f0 = (+ (- ?0) (* ?1 ?1))` (arity 2).

- **`corpus_a.json`** — expanded: every `f0` written `(- ?0)`-first, so a
  rule-free search finds `f0`. Not minimal.
- **`corpus_b.json`** — the size-minimal `a`: one `f0` is commutatively swapped
  and the last is a bare square. egg's extractor keeps each `+` as written, so
  rule-free/at-start only reach the weak `(* ?1 ?1)` squaring. Live rewriting
  re-aligns the `+`s and expands the bare square, recovering `f0`.

Measured (best-first, `--max-arity 2`):

| corpus | rule-free | at-start | live |
|--------|-----------|----------|------|
| A | ~1.32× (`f0`) | — | — |
| B | ~1.11× (squaring) | ~1.11× | ~1.20× (`f0`) |

Pinned by `tests/example_paper_test.rs`.
