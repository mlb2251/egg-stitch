# Regex live-vs-at-start abstraction experiment

A classical-programming domain where applying rewrite rules **live** during
abstraction search beats applying them **once up front**
(`--only-use-dsrs-at-start`) — qualitatively, by surfacing shared, named idioms
that at-start can only bake as one-off constants.

## Corpus

`regex.json` — 250 real regexes mined from the locally-installed Python
libraries (string literals passed to `re.*`), restricted to the
alternation-bearing, moderate-size, deduplicated subset. Each is encoded as an
egg-stitch op-children tree over `Cat / Alt / Star / Plus / Opt / Rep` with
**interned leaf tokens** (`L0, L1, …`); `regex_legend.json` maps each `Ln` back
to the original regex fragment (a literal char, escape, or `[...]` class).

The corpus is *frozen* here for reproducibility. `scripts/regex_corpus.py`
regenerates it from whatever Python packages are installed (so its output is
environment-dependent — provenance, not a reproducible build).

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
python3 scripts/regex_live_vs_atstart.py
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
