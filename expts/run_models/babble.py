"""Wrappers around the external babble compressor.

babble has four binaries: ``drawings`` (cogsci, flat s-exprs), ``benchmark``
(dreamcoder, curried lambda-calc), ``molecules`` (molecule op-children graphs),
and ``circuits`` (EPFL boolean-circuit op-children graphs). The :class:`Babble`
runner dispatches between them based on ``weighting`` and the input path so the
rest of the pipeline sees a single tool.

All binaries expose ``--dump-json`` for a uniform output format. The
``benchmark`` binary auto-loads its DSRs from ``<DSR_PATH>/<domain>.rewrites``
by domain name (it has no flag for an arbitrary location), so the runner
recovers the domain from the input file's parent directory.
"""

import json
import time
from dataclasses import dataclass, field
from functools import cache
from pathlib import Path

from .._build import cargo_build, check_clean_main
from .._subproc import run as _subproc_run
from ..bench import Abstraction, BenchResult, MAX_ARITY, Weighting
from ..folders import current_folder_path, unique_path


# Repo root for *this* project — used to compute the dreamcoder input path's
# parent relative to the egg-stitch tree, so DREAMCODER_DOMAIN_PATHS keys can
# be plain ``Path("data/domains/<name>")`` rather than absolutes.
EGG_STITCH_DIR: Path = Path(__file__).resolve().parent.parent.parent

# Babble lives as a sibling clone of this repo.
BABBLE_DIR: Path = (EGG_STITCH_DIR.parent / "babble").resolve()


@cache
def _babble_ready() -> None:
    """Verify ``../babble`` is on a clean, synced main exactly once per process.

    Both binaries below build from the same source tree, so we share one
    check between them.
    """
    check_clean_main(BABBLE_DIR, "git@github.com:kavigupta/babble.git")


@cache
def babble_bin() -> Path:
    """Build (if needed) and return the path to babble's ``drawings`` binary
    — the cogsci (flat s-expr) runner."""
    _babble_ready()
    return cargo_build(BABBLE_DIR, "drawings")


@cache
def babble_bench_bin() -> Path:
    """Build (if needed) and return the path to babble's ``benchmark`` binary
    — the dreamcoder (curried lambda-calc) runner."""
    _babble_ready()
    return cargo_build(BABBLE_DIR, "benchmark")


@cache
def babble_molecules_bin() -> Path:
    """Build (if needed) and return the path to babble's ``molecules`` binary
    — the op-children molecule-graph runner."""
    _babble_ready()
    return cargo_build(BABBLE_DIR, "molecules")


@cache
def babble_circuits_bin() -> Path:
    """Build (if needed) and return the path to babble's ``circuits`` binary
    — the op-children boolean-circuit (and/or/not over ``$N`` inputs) runner."""
    _babble_ready()
    return cargo_build(BABBLE_DIR, "circuits")


def _expand_bidir_rewrites(text: str) -> str:
    """Rewrite egg-stitch's ``<=>`` rules into the directed ``=>`` form babble
    parses: each ``name: lhs <=> rhs`` becomes ``name`` (forward) and
    ``name_rev`` (backward). Plain ``=>`` rules pass through; comments drop."""
    out = []
    for raw in text.splitlines():
        line = raw.split("//", 1)[0].strip()
        if not line:
            continue
        name, _, body = line.partition(":")
        name, body = name.strip(), body.strip()
        if "<=>" in body:
            lhs, _, rhs = body.partition("<=>")
            out.append(f"{name}: {lhs.strip()} => {rhs.strip()}")
            out.append(f"{name}_rev: {rhs.strip()} => {lhs.strip()}")
        else:
            out.append(f"{name}: {body}")
    return "\n".join(out) + "\n"


# Maps the parent directory of a dreamcoder input file (relative to the
# egg-stitch tree) to the babble domain name, so the runner can pass
# ``--domain`` to ``benchmark``.
DREAMCODER_DOMAIN_PATHS: dict[Path, str] = {
    Path("data/domains/list"):    "list",
    Path("data/domains/physics"): "physics",
    Path("data/domains/text"):    "text",
    Path("data/domains/logo"):    "logo",
    Path("data/domains/towers"):  "towers",
}


@dataclass(frozen=True)
class Babble:
    """Run babble (drawings, benchmark, or molecules, picked by input) on one file."""

    beams: int = 400
    lps: int = 1
    max_arity: int = MAX_ARITY
    # Wall-clock cap (seconds) and address-space cap (bytes) per invocation;
    # excluded from repr so the method label is unchanged.
    timeout: float | None = field(default=None, repr=False)
    mem_limit: int | None = field(default=None, repr=False)

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        if weighting == "no-apps":
            # Molecule and EPFL-circuit corpora are op-children s-exprs like
            # cogsci, but each uses its own grammar/DSRs, so each gets its own
            # babble binary; plain cogsci drawings use the drawings binary.
            if "molecules" in input_path.parts:
                return self._run_op_children(rounds, input_path, rewrites_path, babble_molecules_bin())
            if "epfl-circuits" in input_path.parts:
                return self._run_op_children(rounds, input_path, rewrites_path, babble_circuits_bin())
            return self._run_drawings(rounds, input_path, rewrites_path)
        assert weighting == "apps-equal"
        return self._run_benchmark(rounds, input_path, rewrites_path)

    def _run_op_children(self, rounds: int, input_path: Path, rewrites_path: str | None, binary: Path) -> BenchResult:
        """Run an op-children babble binary (``molecules``/``circuits``) on a
        single family corpus.

        The corpus JSON (a flat array of s-exprs) is written out as the
        one-per-line ``.bab`` format the binary reads, and the egg-stitch
        ``<=>`` DSR file is expanded to babble's directed form. Both derived
        files land in the current results folder. ``binary`` selects the
        grammar-specific runner.
        """
        with open(input_path) as f:
            programs = json.load(f)
        bab = unique_path(current_folder_path() / f"{input_path.stem}.bab")
        bab.write_text("\n".join(programs) + "\n")

        json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
        csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
        cmd = [
            str(binary),
            str(bab),
            f"--beams={self.beams}",
            f"--lps={self.lps}",
            f"--rounds={rounds}",
            f"--max-arity={self.max_arity}",
            f"--output={csv_out}",
            f"--dump-json={json_dump}",
            "--learn-constants",
        ]
        if rewrites_path is not None:
            src = (EGG_STITCH_DIR / rewrites_path).resolve()
            dsr = unique_path(current_folder_path() / f"{input_path.stem}.babble.rewrites")
            dsr.write_text(_expand_bidir_rewrites(src.read_text()))
            cmd.append(f"--dsr={dsr}")
        start = time.time()
        _subproc_run(cmd, cwd=BABBLE_DIR, timeout=self.timeout, mem_limit=self.mem_limit)
        elapsed = time.time() - start
        with open(json_dump) as f:
            data = json.load(f)
        return _result_from_dump(data, elapsed)

    def _run_drawings(self, rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
        """Run babble's ``drawings`` binary on the ``.bab`` file matching ``input_path``.

        The cogsci JSON corpus and babble's ``.bab`` text format hold the same
        s-expressions, so we map ``data/domains/cogsci/<stem>.json`` →
        ``<BABBLE_DIR>/harness/data/cogsci/<stem>.bab``.
        """
        bab = BABBLE_DIR / "harness" / "data" / "cogsci" / f"{input_path.stem}.bab"
        json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
        csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
        cmd = [
            str(babble_bin()),
            str(bab),
            f"--beams={self.beams}",
            f"--lps={self.lps}",
            f"--rounds={rounds}",
            f"--max-arity={self.max_arity}",
            f"--output={csv_out}",
            f"--dump-json={json_dump}",
            # Match the dreamcoder ``benchmark`` binary's hardcoded
            # ``learn_constants=true``, so 0-arity (constant) abstractions
            # are findable in cogsci runs too. Without this flag the
            # cogsci binary would silently disallow them, handicapping
            # babble vs. its dreamcoder behavior and against our tool.
            "--learn-constants",
        ]
        if rewrites_path is not None:
            cmd += [f"--dsr={rewrites_path}"]
        start = time.time()
        _subproc_run(cmd, cwd=BABBLE_DIR, timeout=self.timeout, mem_limit=self.mem_limit)
        elapsed = time.time() - start
        with open(json_dump) as f:
            data = json.load(f)
        return _result_from_dump(data, elapsed)

    def _run_benchmark(self, rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
        """Run babble's ``benchmark`` binary in single-file mode (``--input-file``).

        Babble's ``benchmark`` binary expects a ``CompressionInput`` JSON
        (with grammar/DSL metadata), not the flat program list our
        ``data/domains/<dom>/*.json`` holds. We map our path to the matching
        ``harness/data/dreamcoder-benchmarks/benches/<domain>_<set>/<file>.json``.

        ``rewrites_path`` must equal what babble would auto-load for the
        inferred domain, or be ``None`` (which switches the binary to
        ``--mode au``). Babble has no flag for an arbitrary DSR path.
        """
        parent = input_path.parent
        try:
            rel_parent = parent.relative_to(EGG_STITCH_DIR)
        except ValueError:
            rel_parent = parent
        domain = DREAMCODER_DOMAIN_PATHS.get(rel_parent) or DREAMCODER_DOMAIN_PATHS.get(parent)
        assert domain is not None, (
            f"can't resolve dreamcoder domain for {input_path}; "
            f"add its parent to DREAMCODER_DOMAIN_PATHS"
        )
        # Translate ``<dataset>__<bench>_itN`` → ``<domain>_<dataset>/<bench>_itN.json``.
        stem = input_path.stem
        dataset, sep, bench_file = stem.partition("__")
        assert sep, f"unexpected dreamcoder filename {stem!r}; expected '<dataset>__<bench>'"
        babble_input = (
            BABBLE_DIR / "harness" / "data" / "dreamcoder-benchmarks" / "benches"
            / f"{domain}_{dataset}" / f"{bench_file}.json"
        )
        assert babble_input.exists(), f"babble input not found at {babble_input}"
        if rewrites_path is not None:
            from ..runner import rewrites_path as _expected_rewrites
            expected = _expected_rewrites(domain)
            assert rewrites_path == expected, (
                f"babble auto-loads its own DSRs for {domain!r}; passed "
                f"rewrites_path={rewrites_path!r} but only {expected!r} would be used"
            )
        # ``--mode babble`` runs the full library-learning pipeline with the
        # auto-loaded DSRs applied to the e-graph; ``--mode au`` skips DSRs
        # entirely.
        mode = "babble" if rewrites_path is not None else "au"
        json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
        csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
        cmd = [
            str(babble_bench_bin()),
            "--domain", domain,
            "--input-file", str(babble_input),
            "--output", str(csv_out),
            "--dump-json", str(json_dump),
            "--beam-size", str(self.beams),
            "--lps", str(self.lps),
            "--rounds", str(rounds),
            "--max-arity", str(self.max_arity),
            "--lib-iter-limit", "1",
            "--use-all", "0",
            "--mode", mode,
        ]
        start = time.time()
        _subproc_run(cmd, cwd=BABBLE_DIR, timeout=self.timeout, mem_limit=self.mem_limit)
        elapsed = time.time() - start
        with open(json_dump) as f:
            data = json.load(f)
        # benchmark dump nests under "files"; in --input-file mode there's exactly one.
        files = data["files"]
        assert len(files) == 1, f"expected 1 file in babble dump, got {len(files)}"
        return _result_from_dump(files[0], elapsed)


def _result_from_dump(data: dict, elapsed: float) -> BenchResult:
    """Common BenchResult constructor for both babble dump shapes.

    Both dumps expose ``original``, ``rewritten``, and
    ``abstractions=[{id,body}]``. Wall-clock time is taken from the
    wrapper's own ``time.time()`` rather than babble's reported value so
    it's comparable to the other tools.
    """
    abstractions = [
        Abstraction(name=f"fn_{a['id']}", body=a["body"])
        for a in data.get("abstractions", [])
    ]
    return BenchResult(
        elapsed_secs=elapsed,
        initial_corpus=list(data["original"]),
        final_corpus=list(data["rewritten"]),
        abstractions=abstractions,
    )
