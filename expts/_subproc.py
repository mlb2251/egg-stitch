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
import resource
import subprocess
import sys


def _verbose() -> bool:
    """True when ``EXPTS_VERBOSE`` is set to a truthy value (1/true/yes)."""
    return os.environ.get("EXPTS_VERBOSE", "").lower() in ("1", "true", "yes")


def available_memory_bytes() -> int:
    """Bytes of currently-available RAM, from Linux ``/proc/meminfo``'s
    ``MemAvailable`` (the kernel's estimate of what's allocatable without
    swapping)."""
    with open("/proc/meminfo") as f:
        for line in f:
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    raise RuntimeError("MemAvailable not found in /proc/meminfo")


def _limit_address_space(mem_limit: int):
    """Return a ``preexec_fn`` that caps the child's virtual address space
    (RLIMIT_AS) at ``mem_limit`` bytes, so an over-allocating tool is killed
    deterministically instead of consuming the whole machine."""
    def _apply() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (mem_limit, mem_limit))
    return _apply


def run(cmd: list[str], *, cwd=None, env=None, timeout: float | None = None,
        mem_limit: int | None = None) -> None:
    """Run ``cmd`` like ``subprocess.run(check=True)`` but suppress output by default.

    In verbose mode, echoes the command and streams output (legacy behavior).
    In quiet mode, captures output and re-emits it only if the command fails.

    ``timeout`` (seconds) caps wall-clock time; on expiry the child is killed
    and ``subprocess.TimeoutExpired`` propagates to the caller. ``mem_limit``
    (bytes) caps the child's address space via RLIMIT_AS.
    """
    preexec = _limit_address_space(mem_limit) if mem_limit is not None else None
    if _verbose():
        print("+", " ".join(cmd), flush=True)
        subprocess.run(cmd, check=True, cwd=cwd, env=env, timeout=timeout, preexec_fn=preexec)
        return
    res = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True,
                         timeout=timeout, preexec_fn=preexec)
    if res.returncode != 0:
        sys.stdout.write(res.stdout)
        sys.stderr.write(res.stderr)
        raise subprocess.CalledProcessError(res.returncode, cmd, res.stdout, res.stderr)
