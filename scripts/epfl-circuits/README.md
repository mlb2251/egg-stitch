# epfl-circuits

Turns EPFL benchmark circuits into op-children corpora under
`data/domains/epfl-circuits/`. The generators live here; the corpora the search
consumes are written next to the rewrite files. Circuits: `mult`, `square`, `bar`.

## Generation
1. `fetch_aigs.py`: download the source circuits from the EPFL suite
   (`github.com/lsils/benchmarks`) at a pinned commit. Vendored as `<circuit>.aig`
   — gitignored (reproducible), not checked in.
2. `aig_cones.py`: binary-AIGER parser and input-bounded (k-feasible-cut) cone
   extraction. Per AND gate, grow its fan-in cone until it would exceed K
   distinct input signals, with a node-size cap to bound the blow-up from unrolling
   a reconvergent DAG into a tree.
3. `aig_to_egg.py <circuit>`: drive the above to build a corpus. For each AND gate
   take its K=6 cone, keep cones ≥6 nodes, name the leaves (`sN`) so abstraction
   can metavar over inputs, and stride-sample to 800. Writes
   `data/domains/epfl-circuits/<circuit>.json` (deterministic, byte-for-byte
   reproducible).

The abstraction experiment over these corpora is `table7` in `expts/tables.py`.
