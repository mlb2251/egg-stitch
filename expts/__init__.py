#!/usr/bin/env python3
"""Run experiment variants and collect results into viz/results/<timestamp>/.

Folder management lives in ``expts.folders`` — see that module for the
session-wide folder helpers (``current_folder``, ``new_folder``,
``set_folder``). They're re-exported here for convenience.
"""

import subprocess
from pathlib import Path

from .folders import *


# Project roots for the three compressors we call out to.
EGG_STITCH_DIR = Path(__file__).parent.parent.resolve()
BABBLE_DIR = (EGG_STITCH_DIR.parent / "babble").resolve()
STITCH_DIR = (EGG_STITCH_DIR.parent / "stitch").resolve()


def _cargo_build(project_dir: Path, bin_name: str) -> Path:
    """Run ``cargo build --release --bin=<bin_name>`` and return the binary path."""
    print(f"+ cargo build --release --bin={bin_name}  (in {project_dir})", flush=True)
    subprocess.run(
        ["cargo", "build", "--release", "--bin", bin_name],
        check=True, cwd=project_dir,
    )
    return project_dir / "target" / "release" / bin_name


def _git(repo_dir: Path, *args: str) -> str:
    """Run ``git`` in ``repo_dir`` and return stripped stdout."""
    return subprocess.run(
        ["git", *args],
        check=True, cwd=repo_dir, capture_output=True, text=True,
    ).stdout.strip()


def _check_clean_main(repo_dir: Path, expected_origin: str) -> None:
    """Assert ``repo_dir`` is on main, clean, and synced with ``expected_origin``.

    Raises ``RuntimeError`` if ``origin``'s URL doesn't match
    ``expected_origin``, the working tree isn't on the ``main`` branch, has
    uncommitted/untracked changes, or has diverged from ``origin/main``
    after a fetch.
    """
    origin_url = _git(repo_dir, "remote", "get-url", "origin")
    if origin_url != expected_origin:
        raise RuntimeError(
            f"{repo_dir}: expected origin '{expected_origin}', got '{origin_url}'"
        )
    branch = _git(repo_dir, "rev-parse", "--abbrev-ref", "HEAD")
    if branch != "main":
        raise RuntimeError(f"{repo_dir}: expected branch 'main', got '{branch}'")
    dirty = _git(repo_dir, "status", "--porcelain")
    if dirty:
        raise RuntimeError(f"{repo_dir}: working tree has uncommitted changes:\n{dirty}")
    _git(repo_dir, "fetch", "origin", "main")
    local = _git(repo_dir, "rev-parse", "main")
    remote = _git(repo_dir, "rev-parse", "origin/main")
    if local != remote:
        raise RuntimeError(
            f"{repo_dir}: local main ({local[:8]}) is not in sync with origin/main ({remote[:8]})"
        )


# Verify the external compressors are on a clean, up-to-date main before we
# build and run them so reported numbers are reproducible from a known commit.
_check_clean_main(BABBLE_DIR, "git@github.com:kavigupta/babble.git")
_check_clean_main(STITCH_DIR, "git@github.com:mlb2251/stitch.git")

# Build all four binaries once at import time and expose their paths as
# top-level constants. Cargo's incremental build makes the no-op case cheap.
# ``BABBLE_BIN`` is the cogsci ``drawings`` runner; ``BABBLE_BENCH_BIN`` is
# the dreamcoder benchmark driver used for the list/physics/etc. domains.
EGG_STITCH_BIN: Path = _cargo_build(EGG_STITCH_DIR, "egg-stitch")
BABBLE_BIN: Path = _cargo_build(BABBLE_DIR, "drawings")
BABBLE_BENCH_BIN: Path = _cargo_build(BABBLE_DIR, "benchmark")
STITCH_BIN: Path = _cargo_build(STITCH_DIR, "compress")

# Domains that have a matching drawings.<name>.rewrites file in ../babble.
DOMAINS_WITH_REWRITES = ["dials", "furniture", "nuts-bolts", "wheels"]
# Cogsci domains (single-file inputs under data/domains/cogsci/).
COGSCI_DOMAINS = ["dials", "furniture", "nuts-bolts", "wheels"]
# Dreamcoder benchmark domains (multi-file directories under data/domains/<name>/,
# generated via ``babble`` benchmark binary's ``--dump`` mode). Programs are in
# lambda-calc form so runs need ``--language lambda-calc``. Only ``list`` and
# ``physics`` ship with an equational theory in babble; ``text``/``logo``/``towers``
# are run without DSRs (matching Fig. 12 in the babble paper).
DREAMCODER_DOMAINS = ["list", "physics", "text", "logo", "towers"]
ALL_DOMAINS = COGSCI_DOMAINS + DREAMCODER_DOMAINS


def is_dreamcoder_domain(domain: str) -> bool:
    """True if ``domain`` is a dreamcoder-style multi-file benchmark."""
    return domain in DREAMCODER_DOMAINS


def dreamcoder_files(domain: str) -> list[Path]:
    """Sorted list of the per-iteration JSON input files for a dreamcoder ``domain``."""
    d = EGG_STITCH_DIR / "data" / "domains" / domain
    return sorted(p for p in d.iterdir() if p.is_file() and p.suffix == ".json")


def rewrites_path(domain: str) -> str | None:
    """Path to the babble rewrite rules for ``domain``, or ``None`` if absent.

    Cogsci domains are prefixed with ``drawings.`` in the babble repo;
    dreamcoder domains (list, physics, ...) are at the top level. The
    dreamcoder ``text``/``logo``/``towers`` benchmarks have no equational
    theory in babble, so this returns ``None`` for them.
    """
    if is_dreamcoder_domain(domain):
        path = BABBLE_DIR / "harness" / "data" / "benchmark-dsrs" / f"{domain}.rewrites"
        return f"../babble/harness/data/benchmark-dsrs/{domain}.rewrites" if path.exists() else None
    return f"../babble/harness/data/benchmark-dsrs/drawings.{domain}.rewrites"


# Pulled in at the bottom so submodules can import the constants/helpers
# defined above. `from expts import *` then re-exports everything.
from .egg_stitch import *
from .babble import *
from .stitch import *
from .table1 import *
from .table2 import *
from .table3 import *
from .table4 import *



