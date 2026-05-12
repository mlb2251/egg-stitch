"""Per-tool subprocess runners for the bench.

Each submodule (``ours``, ``stitch``, ``babble``) exposes a callable
dataclass that carries its own hyperparameters as fields. Construct the
runner you want (with overrides as kwargs) and pass the instance to
:func:`expts.runner.run_method`. They share only the
:class:`~expts.bench.BenchResult` shape and the cross-tool
:data:`~expts.bench.MAX_ARITY` default.
"""

from .babble import Babble
from .ours import OursBf, OursSmc, egg_stitch
from .stitch import Stitch
