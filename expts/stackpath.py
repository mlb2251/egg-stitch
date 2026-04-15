"""Global directory stack for grouping run outputs.

The bottom of the stack is a uniquely-named timestamp directory under
``viz/runs/``, created lazily on first use. Use :func:`subgroup` as a
context manager to push a subdirectory for the duration of a block, and
:func:`stackpath` to get the path to a file inside the current directory.
"""

import time
from contextlib import contextmanager
from pathlib import Path

from .folders import unique_path

RUNS_DIR = Path(__file__).parent.parent / "viz" / "runs"

_root: Path | None = None
_stack: list[str] = []


def _root_path() -> Path:
    """Return the lazily-created timestamp root, unique across collisions."""
    global _root
    if _root is None:
        _root = unique_path(RUNS_DIR / time.strftime("%Y-%m-%d_%H-%M-%S"))
        _root.mkdir(parents=True, exist_ok=True)
        print(f"[stackpath] root: {_root}", flush=True)
    return _root


def current_dir() -> Path:
    """Return the absolute directory for the current stack state, creating it."""
    d = _root_path()
    for name in _stack:
        d = d / name
    d.mkdir(parents=True, exist_ok=True)
    return d


@contextmanager
def subgroup(name: str):
    """Push ``name`` onto the stack for the duration of the ``with`` block."""
    _stack.append(name)
    try:
        yield current_dir()
    finally:
        _stack.pop()


def stackpath(filename: str) -> str:
    """Return the string path to ``filename`` inside the current stack directory."""
    return str(current_dir() / filename)
