"""Per-tool subprocess wrappers for the bench.

Each submodule (`ours`, `stitch`, `babble`) is roughly self-contained: it
owns the tool's CLI invocation, JSON parsing, and any tool-specific
hyperparameter constants. They share only the
:class:`~expts.bench.BenchResult` shape and the cross-tool
:data:`~expts.bench.MAX_ARITY` dial.
"""

from .babble import run_babble
from .ours import egg_stitch, run_ours_bf, run_ours_smc
from .stitch import run_stitch
