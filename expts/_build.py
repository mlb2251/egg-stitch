"""Shared build/version helpers for the per-tool wrappers.

Each external compressor wrapper (under :mod:`expts.run_models`) calls
:func:`cargo_build` to ensure its binary is up to date, and
:func:`check_pinned` to assert the source tree is a clean checkout of the
pinned commit (so reported numbers are reproducible from a known commit).
"""

import subprocess
from pathlib import Path


def cargo_build(project_dir: Path, bin_name: str) -> Path:
    """Run ``cargo build --release --bin=<bin_name>`` and return the binary path.

    Output is captured by default to keep table runs quiet; on build failure
    the captured stdout/stderr is replayed before re-raising. Set
    ``EXPTS_VERBOSE=1`` to stream output instead.
    """
    import os
    import sys
    verbose = os.environ.get("EXPTS_VERBOSE", "").lower() in ("1", "true", "yes")
    if verbose:
        print(f"+ cargo build --release --bin={bin_name}  (in {project_dir})", flush=True)
        subprocess.run(
            ["cargo", "build", "--release", "--bin", bin_name],
            check=True, cwd=project_dir,
        )
        return project_dir / "target" / "release" / bin_name
    res = subprocess.run(
        ["cargo", "build", "--release", "--bin", bin_name],
        cwd=project_dir, capture_output=True, text=True, encoding="utf-8",
    )
    if res.returncode != 0:
        sys.stdout.write(res.stdout)
        sys.stderr.write(res.stderr)
        raise subprocess.CalledProcessError(res.returncode, ["cargo", "build", "--release", "--bin", bin_name])
    return project_dir / "target" / "release" / bin_name


def _git(repo_dir: Path, *args: str) -> str:
    """Run ``git`` in ``repo_dir`` and return stripped stdout."""
    return subprocess.run(
        ["git", *args],
        check=True,
        cwd=repo_dir,
        capture_output=True,
        text=True,
    ).stdout.strip()


def check_pinned(repo_dir: Path, expected_commit: str) -> None:
    """Assert ``repo_dir`` is a clean checkout of ``expected_commit``.

    Bumping a tool's pinned commit is what records that its behaviour changed;
    drop the cached results the bump invalidates along with it.
    """
    dirty = _git(repo_dir, "status", "--porcelain")
    if dirty:
        raise RuntimeError(
            f"{repo_dir}: working tree has uncommitted changes:\n{dirty}"
        )
    head = _git(repo_dir, "rev-parse", "HEAD")
    if head != expected_commit:
        raise RuntimeError(
            f"{repo_dir}: pinned to {expected_commit[:8]} but HEAD is at {head[:8]}. "
            f"If that change is meant to affect the numbers, bump the pin and drop the "
            f"cached results it invalidates."
        )
