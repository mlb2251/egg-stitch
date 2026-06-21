# epfl-circuits

Turns EPFL benchmark circuits into op-children corpora under
`data/domains/epfl-circuits/`. The source `.aig` files and the generators live
here; the corpora the search consumes are written next to the rewrite files.

## Generation
1. `fetch_multiplier_aig.py` — download the source circuit. Vendored as
   `multiplier.aig` (EPFL `arithmetic/multiplier`: a 64×64 multiplier, 128 in,
   128 out, 27062 AND gates); this re-downloads it from a pinned upstream commit,
   byte-for-byte.
2. `aig_cones.py` — binary-AIGER parser and input-bounded (k-feasible-cut) cone
   extraction. Per AND gate, grow its fan-in cone until it would exceed K
   distinct input signals (vs cutting at a fixed depth), with a node-size cap to
   bound the blow-up from unrolling a reconvergent DAG into a tree.
3. `aig_to_egg.py` — drive the above to build a corpus: for each AND gate take
   its K=6 cone, keep cones ≥6 nodes, name the leaves (`sN`) so abstraction can
   metavar over inputs, and stride-sample to 800. Writes
   `data/domains/epfl-circuits/mult.json` deterministically (re-running
   reproduces it byte-for-byte).

The abstraction experiment over these corpora is `table7` in `expts/tables.py`.
