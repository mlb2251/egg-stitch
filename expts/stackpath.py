"""Global directory stack for grouping run outputs.

The bottom of the stack is a uniquely-named timestamp directory under
``viz/stackpath/``, created lazily on first use. Use :func:`subgroup` as a
context manager to push a subdirectory for the duration of a block, and
:func:`stackpath` to get the path to a file inside the current directory.
"""

import json
import time
from contextlib import contextmanager
from pathlib import Path

RUNS_DIR = Path(__file__).parent.parent / "viz" / "stackpath"

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


def stackpathpush(name: str) -> Path:
    """Push ``name`` onto the stack and return the new current directory."""
    _stack.append(name)
    return current_dir()


def stackpathpop() -> str:
    """Pop the top of the stack and return the popped name."""
    return _stack.pop()


@contextmanager
def subgroup(name: str):
    """Push ``name`` onto the stack for the duration of the ``with`` block."""
    stackpathpush(name)
    try:
        yield current_dir()
    finally:
        stackpathpop()


def stackpath(filename: str) -> str:
    """Return the string path to ``filename`` inside the current stack directory."""
    return str(current_dir() / filename)


def save_run(result: dict, config: dict, type_: str) -> Path:
    """Write ``result.json``, ``config.json``, and ``type.txt`` into the current
    stack directory. Crashes if a run is already saved there (detected via the
    presence of ``type.txt``).

    ``result`` and ``config`` are dumped as JSON; ``type_`` is written as a
    single-line string into ``type.txt``. Returns the result file path.
    """
    d = current_dir()
    type_path = d / "type.txt"
    assert not type_path.exists(), f"run already exists at {d}"
    result_path = d / "result.json"
    with open(result_path, "w") as f:
        json.dump(result, f, indent=2)
    with open(d / "config.json", "w") as f:
        json.dump(config, f, indent=2)
    with open(type_path, "w") as f:
        f.write(f"{type_}\n")
    return result_path
