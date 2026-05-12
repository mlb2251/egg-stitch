"""Quiet subprocess invocation shared by all run_models wrappers.

By default we capture stdout/stderr from each tool subprocess so a typical
table run doesn't dump thousands of lines to the terminal — progress bars in
:mod:`expts.tables` give the user enough feedback. Set
``EXPTS_VERBOSE=1`` to fall back to streaming output (and printing each
``+ <cmd>`` line, the way the runners used to behave).

On failure, the captured output is replayed before re-raising so debugging
doesn't require rerunning verbosely.
"""

from __future__ import annotations

import os
import subprocess
import sys


def _verbose() -> bool:
    """True when ``EXPTS_VERBOSE`` is set to a truthy value (1/true/yes)."""
    return os.environ.get("EXPTS_VERBOSE", "").lower() in ("1", "true", "yes")


def run(cmd: list[str], *, cwd=None, env=None) -> None:
    """Run ``cmd`` like ``subprocess.run(check=True)`` but suppress output by default.

    In verbose mode, echoes the command and streams output (legacy behavior).
    In quiet mode, captures output and re-emits it only if the command fails.
    """
    if _verbose():
        print("+", " ".join(cmd), flush=True)
        subprocess.run(cmd, check=True, cwd=cwd, env=env)
        return
    res = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if res.returncode != 0:
        sys.stdout.write(res.stdout)
        sys.stderr.write(res.stderr)
        raise subprocess.CalledProcessError(res.returncode, cmd, res.stdout, res.stderr)
