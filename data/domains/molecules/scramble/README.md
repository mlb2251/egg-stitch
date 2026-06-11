# Scramble corpora — DSRs as canonicalisation

These corpora test whether the project's symmetry **DSRs + stitch** can stand in
for a hand-engineered canonical form (like canonical SMILES) as a
**search-space-reduction / alignment** step.

The setup: take real molecules that share a common substructure, encode each one
two ways — a **canonical** rooting (the reference) and a fully **scrambled** one
(random root + random child order at every node, as if no canonicaliser existed)
— and ask how much of the canonical alignment the DSRs can recover from the
scrambled input.

## Families (each a real PubChem substructure search)

| name | shared motif | substructure | molecules |
|------|--------------|--------------|-----------|
| `hexyl`  | linear alkyl backbone   | `CCCCCC`   | 80 |
| `ester`  | ester functional group  | `CC(=O)OC` | 80 |
| `glycol` | polyether / PEG backbone | `OCCOCCO` | 56 |

All are acyclic, **stereocentre-free** (so full neighbour commutativity is sound
— no child swap can fabricate an enantiomer — and a random scramble is perfectly
reversible by the rules), neutral, and ≤ 13–14 heavy atoms (kept small so the
saturated search stays tractable).

## Files

For each `<name>` (`hexyl`, `ester`, `glycol`):
- `<name>.canon.json` — canonical-rooted encoding (reference)
- `<name>.scram.json` — the *same* molecules, random rooting + child order

The re-rooting + full-commutativity DSRs are general molecule symmetry rules (not
scramble-specific), shared by all families and kept at the molecules domain level:
- `data/domains/molecules/symmetries.rewrites` — covers every edge type and head
  shape any atom can take, so it works for any of these molecule corpora

## Regenerate / run

```bash
# rebuild the corpora from PubChem (SMILES cached in scripts/molecules/scramble_smiles_cache.json)
python3 scripts/molecules/gen_scramble_corpora.py

# run the four-way comparison on all families
python3 scripts/molecules/run_scramble_experiment.py
```

The four conditions, run by hand, are (e.g. for `hexyl`):

```bash
D=data/domains/molecules/scramble
# reference (canonical encoding, no rules):
cargo run --release -- --input $D/hexyl.canon.json --search best-first --num-abstractions 8 --num-steps 80000 --max-arity 12
# scrambled, no recovery:
cargo run --release -- --input $D/hexyl.scram.json --search best-first --num-abstractions 8 --num-steps 80000 --max-arity 12
# DSR canonicalisation (saturate symmetries -> per-molecule min-term -> search):
cargo run --release -- --input $D/hexyl.scram.json -r data/domains/molecules/symmetries.rewrites --only-use-dsrs-at-start --search best-first --num-abstractions 8 --num-steps 80000 --max-arity 12
# DSRs kept live during search:
cargo run --release -- --input $D/hexyl.scram.json -r data/domains/molecules/symmetries.rewrites --search best-first --num-abstractions 8 --num-steps 80000 --max-arity 12
```
