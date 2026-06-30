# examples-paper

A minimal worked example showing that a compressive *commutative ordering* need
not fall out of canonicalization — so library learning with **live** rewrites
beats "minimize first, then abstract" (`--only-use-dsrs-at-start`).

The only rewrite is commutativity of `+` (`rules.rewrites`):

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
```

Each program is a 3-level sum of *blocks*; block `k` pairs a shared anchor
`k1/k2/k3` with a per-program value.

- **`corpus_a.json`** — every block is anchor-first, so all programs are
  instances of one skeleton

  ```
  (+ (+ k1 ?) (+ (+ k2 ?) (+ k3 ?)))
  ```

  A plain *syntactic* (rule-free) search finds it.

- **`corpus_b.json`** — a size-minimal canonical form whose block orientations
  are inconsistent across programs. egg's min-term extractor breaks the
  `(+ P Q)` vs `(+ Q P)` tie by smaller child e-class id first, and ids follow
  parse order, so a block `(+ k v)` extracts value-first iff `v` was introduced
  before the anchor `k`. The first program is the all-value-first one, so it
  introduces the shared values `e1,e2,e3` *before* the anchors; later programs
  reuse `e_j` for a value-first block or a fresh "late" value for an anchor-first
  block. The eight programs realize the eight distinct orientation patterns, so
  no consistent skeleton survives — and extracting the minimal term does **not**
  re-align them. (No throwaway program is needed: the all-value-first program is
  a normal member of the corpus.)

So `--only-use-dsrs-at-start`, which abstracts over the extracted minimal corpus,
is stuck with the scrambled B, while live commutativity re-aligns the blocks and
recovers A's compression.

Measured with best-first, `--max-arity 3`:

| corpus | rule-free | `--only-use-dsrs-at-start` | live |
|--------|-----------|----------------------------|------|
| A      | ~2.0×     | ~1.4×                      | ~2.0× |
| B      | ~1.1×     | ~1.1×                      | ~2.0× |

`tests/example_paper_test.rs` pins this: A ≡ B under commutativity, B is
size-minimal, syntactic search compresses A but not B, and live rewriting beats
`--only-use-dsrs-at-start` on B.
