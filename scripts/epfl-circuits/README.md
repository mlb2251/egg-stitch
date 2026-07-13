# epfl-circuits

Turns EPFL benchmark circuits into op-children corpora under
`data/domains/epfl-circuits/`. The generators live here; the corpora the search
consumes are written next to the rewrite file. The committed set —
`multiplier`, `square`, `log2`, `hyp`, `voter` — is chosen by `build_benchmarks.py`
(below).

## Pipeline
1. `fetch_aigs.py`: download source circuits from the EPFL suite
   (`github.com/lsils/benchmarks`) at a pinned commit. Vendored as `<circuit>.aig`
   — gitignored (reproducible), not checked in.
2. `aig_cones.py`: binary-AIGER parser and input-bounded (k-feasible-cut) cone
   extraction. Per AND gate, grow its fan-in cone until it would exceed K
   distinct input signals, with a node-size cap to bound the blow-up from unrolling
   a reconvergent DAG into a tree.
3. `aig_to_egg.py <circuit>`: build one corpus. For each AND gate take its K=6
   cone, keep cones ≥6 nodes, name the leaves (`sN`) so abstraction can metavar
   over inputs, and stride-sample to 800. Writes
   `data/domains/epfl-circuits/<circuit>.json` (deterministic, byte-for-byte
   reproducible).

## Selecting the benchmark set
`build_benchmarks.py` surveys the whole EPFL suite and keeps the circuits above
the median on **both** axes, writing their corpora and dropping the rest:
- distinct cone shapes — structural diversity;
- no-rules compression (egg-stitch, no DSRs) — real repeated structure.

Diverse *and* redundant is where live DSRs pay off; either alone misses (diverse-
but-unique control logic and redundant-but-canonical adders both fall flat).

The abstraction experiment over the corpora is `table7` in `expts/tables.py`.
