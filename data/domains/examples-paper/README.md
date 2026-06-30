# examples-paper

A minimal worked example showing that a compressive *commutative ordering* need
not fall out of canonicalization — so library learning with **live** rewrites
beats "minimize first, then abstract" (`--only-use-dsrs-at-start`).

The only rewrite is commutativity of `+` (`rules.rewrites`):

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
```

Each program (after a shared `preamble` at index 0) is a 3-level sum of
*blocks*; block `k` pairs a shared anchor `k1/k2/k3` with a per-program value.

- **`corpus_a.json`** — every block is anchor-first, so all programs are
  instances of one skeleton

  ```
  (+ (+ k1 ?) (+ (+ k2 ?) (+ k3 ?)))
  ```

  A plain *syntactic* (rule-free) search finds it.

- **`corpus_b.json`** — the size-minimal canonical form. egg's min-term
  extractor breaks the `(+ P Q)` vs `(+ Q P)` tie by putting the smaller child
  e-class id first, and ids follow parse order. The corpus introduces some
  values *before* the anchors (smaller id) and some *after* (larger id), so that
  canonical orientation is **inconsistent** across programs. The shared skeleton
  is therefore invisible to a syntactic search, and — crucially — extracting the
  minimal term does **not** re-align it.

The key fact: the minimal term of A *is* B, and B is its own minimal term. So
`--only-use-dsrs-at-start` (which abstracts over the extracted minimal corpus) is
stuck with the scrambled B and compresses poorly, while live commutativity
re-aligns the blocks and recovers A's compression.

Measured with best-first, `--max-arity 3`:

| corpus | rule-free | `--only-use-dsrs-at-start` | live |
|--------|-----------|----------------------------|------|
| A      | ~1.67×    | ~1.11×                     | ~1.67× |
| B      | ~1.11×    | ~1.11×                     | ~1.67× |

`tests/example_paper_test.rs` pins all of this: A ≡ B under commutativity, B is
the minimal term (and stays scrambled), syntactic search compresses A but not B,
and live rewriting beats `--only-use-dsrs-at-start` on both.
