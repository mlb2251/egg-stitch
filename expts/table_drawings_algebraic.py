"""Standalone drawing-domain algebraic-DSR experiment (not a paper table).

Runs the cogsci drawing domains (nuts-bolts/dials/wheels/furniture) with our
algebraic drawing DSRs (``data/domains/cogsci/drawings.rewrites``), comparing the
DSRs kept LIVE during search against applied only AT START. The rules are
deliberately *non-confluent* (transform factoring, repeat<->unroll, overlay
assoc/comm, scale/translate interchange) — they expose multiple equivalent normal
forms whose best choice depends on the library being built. Live keeps all forms
so each abstraction can align to the matching one; at-start commits to a single
greedy min-term up front, so live wins (and the gap widens with expressiveness).
Same roster shape as table5/7 (Enum/SMC sweeps + dsrs-only-at-start baseline, no
babble — it can't parse the constant_folding/matmul directives).

Kept out of the main ``tables`` / ``render_tables`` pipeline because it isn't used
in the paper for now. Run ``table_drawings_algebraic()`` to (re)generate its
results, then ``scripts/render_drawings_algebraic.py`` to render them. The shared
run infrastructure still lives in :mod:`expts.tables`.
"""

from pathlib import Path

from .bench import MEM_LIMIT_BYTES
from .run_models import OursBf
from .tables import BASELINE_BFS_STEPS, _require_free_memory, _run_table, _sweep_runners

DOMAINS = [f"drawings:{d}" for d in ("nuts-bolts", "dials", "wheels", "furniture")]
NUM_ABSTRACTIONS = 4
ITER_LIMIT = 6
# Per-factor row cap (the `--max-match-set` metric is a factor's row count): the
# commutativity blowup is one entangled factor whose equivalent parse trees pile
# up as rows. 24 bounds dials/furniture memory while sparing the high-usage
# patterns; above ~64 furniture's blowup escapes the cap.
MATCH_SET_CAP = 24
# Decompose factors before the row cap can prune them: must be <= the cap (the
# binary asserts it) so a benign independent product just under the cap isn't
# mistaken for an entangled blowup. Pinned equal to the cap.
DECOMPOSE_MIN_ROWS = 24
# Arity 4 (vs the paper tables' 2): the drawing domains have deeper repeated
# part-hierarchies, and higher arity both raises absolute compression and widens
# the live-vs-at-start gap (more holes => more normal-form alignment that live can
# exploit but at-start commits away).
MAX_ARITY = 4
TIMEOUT = 300.0  # seconds, per tool invocation


def _runners() -> tuple[tuple[str, object], ...]:
    """Enum/SMC sweeps (live DSRs) plus the dsrs-only-at-start baseline, every
    runner at arity 4 with the per-factor match-set cap + iter-limit the
    non-confluent algebra needs (no babble — it can't parse the rules)."""
    common = dict(
        max_arity=MAX_ARITY,
        iter_limit=ITER_LIMIT,
        max_match_set=MATCH_SET_CAP,
        decompose_min_rows=DECOMPOSE_MIN_ROWS,
        timeout=TIMEOUT,
        mem_limit=MEM_LIMIT_BYTES,
    )
    return (
        _sweep_runners(**common)
        + (("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **common)),)
    )


def table_drawings_algebraic() -> Path:
    """Run the cogsci drawing domains with the non-confluent algebraic DSRs: the
    table5/7 roster (Enum/SMC sweeps + dsrs-only-at-start baseline, no babble) at
    arity 4 with the per-factor match-set cap. Demonstrates live > at-start on a
    non-confluent rule set. Writes ``results/table_drawings_algebraic.json``."""
    _require_free_memory("table_drawings_algebraic")
    return _run_table(
        domains=DOMAINS,
        runners=_runners(),
        num_abstractions=NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="table_drawings_algebraic",
        output_name="table_drawings_algebraic.json",
    )
