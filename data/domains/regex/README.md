# Regex live-vs-at-start abstraction experiment

A classical-programming domain where applying rewrite rules **live** during
abstraction search beats applying them **once up front**
(`--only-use-dsrs-at-start`) — qualitatively, by surfacing shared, named idioms
that at-start can only bake as one-off constants.

## Corpora

Two frozen corpora, both encoded as egg-stitch op-children trees over
`Cat / Alt / Star / Plus / Opt / Rep` with **interned leaf tokens** (`L0, L1, …`);
the matching `*_legend.json` maps each `Ln` back to the original regex fragment
(a literal char, escape, or `[...]` class).

- **`regex_pygments.json`** (300 regexes, higher-structure, *recommended*) — token
  regexes harvested from ~480 pygments lexers. These heavily share sub-structure
  (keyword char-chains, identifier classes, number/string formats) within and
  across languages, which gives the live-vs-at-start abstraction the largest edge.
  Generator: `scripts/regex_corpus_pygments.py`.
- **`regex.json`** (250 regexes) — regexes mined from arbitrary `re.*` calls in the
  installed Python libraries; more of a grab-bag, smaller edge. Generator:
  `scripts/regex_corpus.py`.

Both are *frozen* for reproducibility; the generators regenerate from the local
Python/pygments install (environment-dependent — provenance, not a reproducible
build).

### The edge is an early-abstraction effect, not an aggregate-compression win

Final corpus+library cost, best-first:

| corpus | size | abstractions | baseline | at-start | live |
|---|---|---|---|---|---|
| `regex` (lib mine) | 250 | 15 | 3653 | 3653 | 3554 |
| `regex_pygments` | 300 | 15 | 4401 | 4532 | 4307 |
| `regex_pygments` | 957 | 40 | 13133 | 12983 | 12996 |

The *aggregate* final-cost gap is ratio/density-dependent and **dilutes at scale**:
on the 300-regex subset at 15 abstractions live leads at-start by ~225 (and at-start
is even worse than no rules at all), but on the full 957-regex corpus with a
corpus-proportional 40-abstraction budget the final costs essentially tie
(live −13). Both runs converge (drained heaps), so this is not a budget artifact.

The robust signal is **per-step ordering — live finds the meaningful, shared
abstractions first.** On 957/40 the per-step gap (at-start − live) rises to ~75 by
step ~8, then at-start recovers the same structure in later rounds and the totals
tie out:

```
step:  1   2   3   4   5   6   7   8   9  10  ...  36  37  38  39  40
gap:  29  12  32  22  22  72  73  75  75  75  ... -14 -13 -13 -13 -13
```

So the claim is **"live surfaces meaningful/shared abstractions earlier,"** not
"live compresses more in aggregate." The durable evidence is (a) the first-K
abstractions' match counts and content, and (b) the live-only content idioms below.

## Rewrite rules — `regex.rewrites`

Regex algebra, all **size-tied or expansive, non-confluent** (the kind that the
min-size min-term of `--only-use-dsrs-at-start` cannot exploit):
commutativity + associativity of `Alt`, associativity of `Cat`, distribution of
`Cat` over `Alt` (both sides), and the `Plus`/`Opt` definitions.

Deliberately **excludes** contractive simplifications (`a**→a*`, `a*a*→a*`,
`(a|a)→a`, ε-elimination): those *help* at-start (it normalizes toward a smaller
min-term and catches up), so they shrink the live advantage. See the ablation in
the experiment notes.

`list_naturality.rewrites` is the functional-domain control (binder-free
map/cons/cdr naturality laws over `data/domains/list/`); it produced ~no live
edge because those laws rarely fire in that corpus — kept for completeness.

## Reproduce

```bash
cargo build --release
python3 scripts/regex_live_vs_atstart.py                 # default: regex_pygments
python3 scripts/regex_live_vs_atstart.py regex           # the Python-lib corpus
```

Prints the per-step side-by-side and the content-bearing idiom comparison.

## What it shows

- Total compression: live ≈ at-start within a few percent — the bulk is generic
  `Cat`/`Alt` skeletons that *both* find.
- **Per step, live extracts a more-shared abstraction**; e.g. it matches the
  `Cat`-of-3 skeleton ~3× more sites than at-start because the rule-saturated
  e-graph exposes more aligned occurrences.
- **Live surfaces shared content idioms at-start misses**: an anchored-start
  `^…` idiom (matches ~30), a quoted-string-with-escapes matcher, and a
  delimiter-parameterized **genomic-locus matcher**
  `fn(S) = (X|Y|M|\d+) S \d+ S [ACGTN]* S [ACGTN]*` (arity 1, the field
  delimiter reused 3×), which unifies three real genomic-variant regexes that
  differ only in their delimiter and share *no literal substructure*. At-start
  commits each regex to one min-size factorization; with the non-confluent rules
  those factorizations diverge (the `_`-delimited one factors differently from
  the `-`/`:` ones), so the shared abstraction is absent from the rule-free
  e-graph it searches — confirmed with `--follow` reachability.
