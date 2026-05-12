"""Shared build/version helpers for the per-tool wrappers.

Each external compressor wrapper (under :mod:`expts.run_models`) calls
:func:`cargo_build` to ensure its binary is up to date, and
:func:`check_clean_main` to assert the source tree is on a clean, synced
``main`` (so reported numbers are reproducible from a known commit).
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
        cwd=project_dir, capture_output=True, text=True,
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


def check_clean_main(repo_dir: Path, expected_origin: str) -> None:
    """Assert ``repo_dir`` is on main, clean, and synced with ``expected_origin``.

    Raises ``RuntimeError`` if origin's URL doesn't match, the working tree
    isn't on ``main``, has uncommitted/untracked changes, or has diverged
    from ``origin/main`` after a fetch.
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
        raise RuntimeError(
            f"{repo_dir}: working tree has uncommitted changes:\n{dirty}"
        )
    _git(repo_dir, "fetch", "origin", "main")
    local = _git(repo_dir, "rev-parse", "main")
    remote = _git(repo_dir, "rev-parse", "origin/main")
    if local != remote:
        raise RuntimeError(
            f"{repo_dir}: local main ({local[:8]}) is not in sync with origin/main ({remote[:8]})"
        )
