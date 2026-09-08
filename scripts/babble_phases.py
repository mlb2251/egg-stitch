#!/usr/bin/env python3
"""Attribute every babble DNF in tables 3/5/7 to one of babble's pipeline phases.

Re-runs each DNF cell under ``RUST_LOG=info``, which brackets each phase of a
round -- DSR saturation, co-occurrence, anti-unification, dedup, beam search,
extraction -- with a timing log. A run that blows its time or memory budget
stops mid-phase, naming the phase responsible.

Each cell is re-run at the settings its own table row records (beams, lps and
max arity from the method label, rounds from the table config, timeout from the
recorded budget), under the tables' shared memory cap.
"""

import json
import math
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from expts._subproc import _limit_address_space  # noqa: E402
from expts.bench import MEM_LIMIT_BYTES  # noqa: E402
from expts.folders import current_folder_path, summary_results_path  # noqa: E402
from expts.run_models.babble import (  # noqa: E402
    EGG_STITCH_DIR, BABBLE_DIR, _expand_bidir_rewrites,
    babble_circuits_bin, babble_molecules_bin,
)
from expts.runner import _RESOURCE_KILL_SIGNALS, domain_type, input_files, rewrites_path  # noqa: E402

# The tables that run babble with DSRs live, i.e. the ones whose cells can DNF.
TABLES = ("table3.json", "table5.json", "table7.json")

# Only these domain types have an op-children babble binary the script can drive.
BINARIES = {"molecules": babble_molecules_bin, "epfl-circuits": babble_circuits_bin}

# Substrings of the log line that opens each phase, in the order a round runs
# them. The next timing line closes the open phase.
PHASE_STARTS = [
    ("DSRs...", "dsrs"),
    ("Running co-occurrence analysis", "co-occurrence"),
    ("Running anti-unification", "anti-unification"),
    ("Deduplicating patterns", "dedup"),
    ("Adding libs and running beam search", "beam-search"),
    ("Extracting", "extraction"),
]

# The size a phase reports alongside its time, if any.
PHASE_COUNTS = [
    (re.compile(r"(\d+) patterns"), "patterns"),
    (re.compile(r"final egraph size: (\d+)"), "nodes"),
]

# babble warns once per DFTA state whose anti-unification set exceeds 10k.
AU_WARNING = re.compile(r"Large number of antiunifications for state .*: (\d+)")


def prepare_inputs(domain: str, out_dir: Path) -> tuple[Path, Path]:
    """Write the corpus as babble's one-per-line ``.bab`` and the domain's
    rewrites expanded into babble's directed form."""
    [src] = input_files(domain)
    bab = out_dir / f"{src.stem}.bab"
    bab.write_text("\n".join(json.load(open(src))) + "\n")
    rew = out_dir / f"{src.stem}.babble.rewrites"
    rew.write_text(_expand_bidir_rewrites((EGG_STITCH_DIR / rewrites_path(domain)).read_text()))
    return bab, rew


@dataclass(frozen=True)
class Cell:
    """A babble DNF cell, with the settings its table row was produced under."""

    table: str
    domain: str
    rounds: int
    args: tuple[tuple[str, str], ...]
    timeout: float | None


def dnf_cells() -> list[Cell]:
    """Every babble cell that DNF'd across :data:`TABLES`, one per domain.

    A DNF row records no compression ratio, and its ``elapsed_secs`` is the
    budget the run was given (NaN when the table capped nothing).
    """
    cells: dict[str, Cell] = {}
    for table in TABLES:
        data = json.load(open(summary_results_path(table)))
        for domain, entry in data["domains"].items():
            for rep in entry["runs"].get("babble", []):
                for row in rep:
                    if row["compression_ratio"] is not None or domain in cells:
                        continue
                    budget = row["elapsed_secs"]
                    # The method label is Babble's dataclass repr, e.g.
                    # "Babble(beams=400, lps=1, max_arity=2)".
                    label = row["method"].removeprefix("Babble(").removesuffix(")")
                    cells[domain] = Cell(
                        table=table.removesuffix(".json"),
                        domain=domain,
                        rounds=data["config"]["num_abstractions"],
                        args=tuple(tuple(a.split("=")) for a in label.split(", ")),
                        timeout=None if math.isnan(budget) else budget,
                    )
    return list(cells.values())


def run_babble(binary: Path, bab: Path, rew: Path, *, cell: Cell, dump: Path) -> tuple[str, float, str | None]:
    """Run one babble invocation; return its ``(stderr, elapsed, dnf)``, where
    ``dnf`` names the budget it blew (or None if it finished).

    The table counts both budgets as a DNF: wall clock, and the memory cap,
    which Rust hits as an allocation failure that aborts the process.
    """
    cmd = [
        str(binary), str(bab), f"--rounds={cell.rounds}", f"--output={dump.with_suffix('.csv')}",
        f"--dump-json={dump}", "--learn-constants", f"--dsr={rew}",
        *(f"--{name.replace('_', '-')}={value}" for name, value in cell.args),
    ]
    start = time.time()
    try:
        res = subprocess.run(
            cmd, cwd=BABBLE_DIR, capture_output=True, text=True, encoding="utf-8",
            env={**os.environ, "RUST_LOG": "info"},
            timeout=cell.timeout,
            preexec_fn=_limit_address_space(MEM_LIMIT_BYTES),
        )
        if -res.returncode in _RESOURCE_KILL_SIGNALS:
            return res.stderr, time.time() - start, "out of memory"
        if res.returncode:
            sys.stderr.write(res.stderr)
            raise SystemExit(f"babble exited {res.returncode}")
        return res.stderr, time.time() - start, None
    except subprocess.TimeoutExpired as e:
        # The exception carries raw bytes even under text=True.
        captured = e.stderr or b""
        return captured.decode("utf-8", "replace"), time.time() - start, "timeout"


@dataclass
class Phase:
    """One phase of one round; ``ms`` is None for a phase the run died inside."""

    name: str
    ms: int | None = None
    count: str | None = None

    def __str__(self) -> str:
        return f"{self.name} {'INCOMPLETE' if self.ms is None else f'{self.ms}ms'}" + (
            f" ({self.count})" if self.count else ""
        )


def reported_count(line: str) -> str | None:
    """The size a completion line reports, e.g. ``"977 patterns"``."""
    for pat, unit in PHASE_COUNTS:
        if m := pat.search(line):
            return f"{m.group(1)} {unit}"
    return None


def parse_rounds(stderr: str) -> list[list[Phase]]:
    """The phases of each round, in the order they ran."""
    rounds: list[list[Phase]] = []
    open_phase: Phase | None = None
    for line in stderr.splitlines():
        if "beam_experiment" not in line:
            continue
        name = next((name for marker, name in PHASE_STARTS if marker in line), None)
        if name is not None:
            if name == "dsrs":
                rounds.append([])
            open_phase = Phase(name)
            rounds[-1].append(open_phase)
        elif open_phase is not None and (ms := re.search(r"(\d+)ms", line)):
            open_phase.ms = int(ms.group(1))
            open_phase.count = reported_count(line)
            open_phase = None
    return rounds


def report(cell: Cell, stderr: str, elapsed: float, dnf: str | None, dump: Path) -> None:
    """Print the per-round phase breakdown and anti-unification blowup stats."""
    outcome = f"DNF ({dnf}) after {elapsed:.0f}s" if dnf else (
        f"finished in {elapsed:.0f}s, compression {json.load(open(dump))['compression']:.3f}")
    print(f"\n{cell.domain} ({cell.table}): {outcome}")
    for i, phases in enumerate(parse_rounds(stderr), 1):
        print(f"  round {i}: " + " | ".join(str(p) for p in phases))
    # A DNF stops mid-enumeration, so its count is a lower bound.
    blowups = [int(m.group(1)) for m in AU_WARNING.finditer(stderr)]
    if blowups:
        print(f"  anti-unification blowups: {len(blowups)} states over 10k, worst {max(blowups)}")


def main() -> None:
    cells = dnf_cells()
    unrunnable = [c.domain for c in cells if domain_type(c.domain) not in BINARIES]
    if unrunnable:
        raise SystemExit(f"no op-children babble binary for {', '.join(unrunnable)}")

    out_dir = current_folder_path() / "babble-phases"
    out_dir.mkdir(parents=True, exist_ok=True)
    for cell in cells:
        bab, rew = prepare_inputs(cell.domain, out_dir)
        dump = out_dir / f"{cell.domain.replace(':', '-')}.json"
        stderr, elapsed, dnf = run_babble(BINARIES[domain_type(cell.domain)](), bab, rew, cell=cell, dump=dump)
        report(cell, stderr, elapsed, dnf, dump)


if __name__ == "__main__":
    main()
