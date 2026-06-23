# Regex live-vs-at-start abstraction experiment

A classical-programming domain where keeping rewrite rules **live** during
abstraction search finds the meaningful, shared idioms — and finds them earlier
and with more reuse — than applying the rules once up front
(`--only-use-dsrs-at-start`).

## Corpus

`regexlib.json` — 400 real regexes from **RegExLib**, taken from the ReDoSHunter
artifact (`github.com/yetingli/ReDoSHunter`, `data/paper_dataset/regexlib.txt`):

> Li et al., *ReDoSHunter: A Combined Static and Dynamic Approach for Regular
> Expression DoS Detection*, USENIX Security 2021.

RegExLib is a community archive where independent authors submit regexes keyed by
task (email / date / phone / URL / …). So within a task there are **many
independent spellings of the same matcher** — the shared structure is *latent*
(nobody factored it), which is exactly what distinguishes live from at-start.

Encoding (`scripts/regexlib_corpus.py`, regenerates from the source dump; the
frozen JSON here is what the experiment uses):

- leaf tokens are the **raw regex fragment**, percent-encoding only parser-unsafe
  chars (space → `%20`, `(` → `%28`, …) — abstractions read as regexes, no legend;
- a positive character class **decomposes into an `Alt` of its members**
  (`[a-zA-Z]` → `(Alt [a-z] [A-Z])`), so the *existing* `Alt` commutativity unifies
  all member reorderings (`[a-zA-Z]` ≡ `[A-Za-z]`, etc.) generically — no
  per-spelling rules. Negated/POSIX classes stay opaque (complement ≠ union);
- bounded quantifiers `x{n}` / `x{n,m}` → `(Range x lo hi)`; `Star`/`Plus`/`Opt`
  are kept (their relation to `Range` is a rewrite rule, not the encoding).

## Rewrite rules — `regexlib.rewrites`

Regex algebra, **size-tied / expansive and non-confluent** (the kind a min-size
min-term cannot exploit, so they create latent alignment rather than a canonical
form): commutativity + associativity of `Alt`, associativity of `Cat`,
distribution of `Cat` over `Alt`; the `Range`↔`Star`/`Plus`/`Opt` bridges and
bounded-repeat expansion (`(Range x 2 2) <=> (Cat x x)`, …); `range_add` /
`range_open`; and the shorthand↔class rules `\d <=> [0-9]`,
`\w <=> (Alt [a-z] (Alt [A-Z] (Alt [0-9] _)))`. Deliberately **no contractive
simplifications** (`a**→a*`, etc.): those help at-start normalize and shrink the
live advantage.

## Configuration: `--max-arity 2 --no-zero-arity`

Abstractions must have **1–2 parameters**. The arity cap acts as a
*meaningfulness filter*: the high-arity abstractions are overwhelmingly syntactic
glue (4-way concatenations, 4-way alternations), while the recognizable idioms
have arity 1–2. Capping arity removes the shared syntactic skeletons (which both
modes find, masking the gap) and exposes live's advantage on the meaningful ones.

## Reproduce

```bash
cargo build --release
python3 scripts/regex_live_vs_atstart.py
```

Prints the per-step side-by-side (each abstraction expanded to a real regex,
flagged meaningful/syntactic), the cumulative cost gap, and the meaningful counts.

## Results

| config | baseline | at-start | live | live gap |
|---|---|---|---|---|
| `--max-arity 4` (zero-arity ok) | 7139 | 6751 | 6685 | 66 |
| `--max-arity 4 --no-zero-arity` | 7194 | 6901 | 6778 | 123 |
| **`--max-arity 2 --no-zero-arity`** | 7879 | 7436 | **6923** | **513** |

Under the arity cap the live gap is ~8× the default and **grows monotonically**
across the 20 abstractions (the per-step gap rises to ~520). Meaningful
abstractions: **live 12/20, at-start 11/20** (vs 7/6 without the cap).

What live finds (fully expanded, `#k` = parameter):

- **digit fields** — `\d{#0}`, `\d{1,#0}`, `#0\d{#1}`
- **character classes** (via the class-as-`Alt` decomposition) — `([A-Z]|#0)`,
  `([a-z]|[A-Z]|#0)` (alphanumeric)
- **day-of-month** — `(3[01] | [12]\d | #0[1-9])`, where `#0 ∈ {0?, 0}` unifies
  the optional-vs-required-leading-zero spellings independent authors used
- plus optional-`-`, optional-fractional `(\.#0)?`, etc.

The day-of-month and the parameterized classes are the cleanest illustration:
live abstracts the part that varies across independent authors into a parameter,
so one arity-1 template covers several differently-spelled matchers, while
at-start commits each regex to one min-size spelling and either misses them or
bakes in a single concrete variant.
