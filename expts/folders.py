"""Session-wide results folder management for expts.

Each Python session lazily creates a fresh timestamp-named folder under
``viz/results/`` the first time it is asked for one, and subsequent calls
(including those from ``egg_stitch``/``runall``) reuse it. Use
``new_folder()`` to start a fresh one, or ``set_folder(name)`` to point at
an existing one.
"""

import time
from pathlib import Path

RESULTS_DIR = Path(__file__).parent.parent / "viz" / "results"

# Summary tableN.json files live here (checked into git, single canonical
# copy per table). Raw per-file subprocess dumps stay under ``RESULTS_DIR``
# above, which is .gitignored.
SUMMARY_RESULTS_DIR = Path(__file__).parent.parent / "results"


def summary_results_path(name: str) -> Path:
    """Return the absolute path for a checked-in summary JSON (e.g.
    ``results/table1.json``), creating the parent folder as needed."""
    SUMMARY_RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    return SUMMARY_RESULTS_DIR / name

# Lazily initialized on first use so merely importing the module doesn't
# create an empty folder on disk.
_current_folder: str | None = None


def _make_timestamp() -> str:
    """Return a filesystem-safe timestamp like '2026-04-10_14-30-45'."""
    return time.strftime("%Y-%m-%d_%H-%M-%S")


def current_folder() -> str:
    """Return the current session's folder name, creating one if needed."""
    global _current_folder
    if _current_folder is None:
        _current_folder = _make_timestamp()
        (RESULTS_DIR / _current_folder).mkdir(parents=True, exist_ok=True)
        print(f"[expts] results folder: {_current_folder}", flush=True)
    return _current_folder


def current_folder_path() -> Path:
    """Return the absolute Path to the current session's folder."""
    folder = RESULTS_DIR / current_folder()
    folder.mkdir(parents=True, exist_ok=True)
    return folder


def new_folder() -> str:
    """Start a fresh timestamp-named folder for subsequent runs."""
    global _current_folder
    _current_folder = None
    return current_folder()


def set_folder(name: str) -> str:
    """Point subsequent runs at the named folder (created if missing)."""
    global _current_folder
    _current_folder = name
    (RESULTS_DIR / name).mkdir(parents=True, exist_ok=True)
    print(f"[expts] results folder: {name}", flush=True)
    return name


def unique_path(path: Path) -> Path:
    """Return `path` if it doesn't exist yet, else append `_1`, `_2`, ... before the suffix."""
    if not path.exists():
        return path
    stem, suffix = path.stem, path.suffix
    i = 1
    while True:
        candidate = path.with_name(f"{stem}_{i}{suffix}")
        if not candidate.exists():
            return candidate
        i += 1
