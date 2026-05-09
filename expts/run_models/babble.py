"""Wrappers around the external babble compressor.

babble has two binaries: ``drawings`` (cogsci, flat s-exprs) and
``benchmark`` (dreamcoder, curried lambda-calc). :func:`run_babble`
dispatches between them based on ``weighting`` so the rest of the pipeline
sees a single tool.

Both binaries expose ``--dump-json`` for a uniform output format. Babble's
``benchmark`` binary auto-loads its DSRs from ``<DSR_PATH>/<domain>.rewrites``
by domain name (it has no flag for an arbitrary location), so the wrapper
recovers the domain from the input file's parent directory.
"""

import json
import subprocess
import time
from functools import cache
from pathlib import Path

from .._build import cargo_build, check_clean_main
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


# ─── Hyperparameters ───────────────────────────────────────────────────────

BABBLE_BEAMS = 400
BABBLE_LPS = 1


# ─── Path inference for apps-equal ─────────────────────────────────────────
DREAMCODER_DOMAIN_PATHS: dict[Path, str] = {
    Path("data/domains/list"):    "list",
    Path("data/domains/physics"): "physics",
    Path("data/domains/text"):    "text",
    Path("data/domains/logo"):    "logo",
    Path("data/domains/towers"):  "towers",
}


def run_babble(rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Dispatch between babble's ``drawings`` and ``benchmark`` binaries based
    on ``weighting``."""
    if weighting == "no-apps":
        return _run_drawings(rounds, input_path, rewrites_path)
    assert weighting == "apps-equal"
    return _run_benchmark(rounds, input_path, rewrites_path)


def _run_drawings(rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
    """Run babble's ``drawings`` binary on the ``.bab`` file matching
    ``input_path``.

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
        f"--beams={BABBLE_BEAMS}",
        f"--lps={BABBLE_LPS}",
        f"--rounds={rounds}",
        f"--max-arity={MAX_ARITY}",
        f"--output={csv_out}",
        f"--dump-json={json_dump}",
    ]
    if rewrites_path is not None:
        cmd += [f"--dsr={rewrites_path}"]
    print("+", " ".join(cmd), flush=True)
    start = time.time()
    subprocess.run(cmd, check=True, cwd=BABBLE_DIR)
    elapsed = time.time() - start
    with open(json_dump) as f:
        data = json.load(f)
    return _result_from_dump(data, elapsed)


def _run_benchmark(rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
    """Run babble's ``benchmark`` binary in single-file mode (``--input-file``).

    ``rewrites_path`` must equal what babble would auto-load for the inferred
    domain, or be ``None`` (which switches the binary to ``--mode au``).
    Babble has no flag for an arbitrary DSR path.
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
    if rewrites_path is not None:
        from ..runner import rewrites_path as _expected_rewrites
        expected = _expected_rewrites(domain)
        assert rewrites_path == expected, (
            f"babble auto-loads its own DSRs for {domain!r}; passed "
            f"rewrites_path={rewrites_path!r} but only {expected!r} would be used"
        )
    mode = "babble" if rewrites_path is not None else "au"
    json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
    csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
    cmd = [
        str(babble_bench_bin()),
        "--domain", domain,
        "--input-file", str(input_path),
        "--output", str(csv_out),
        "--dump-json", str(json_dump),
        "--beam-size", str(BABBLE_BEAMS),
        "--lps", str(BABBLE_LPS),
        "--rounds", str(rounds),
        "--max-arity", str(MAX_ARITY),
        "--lib-iter-limit", "1",
        "--use-all", "0",
        "--mode", mode,
    ]
    print("+", " ".join(cmd), flush=True)
    start = time.time()
    subprocess.run(cmd, check=True, cwd=BABBLE_DIR)
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
    ``abstractions=[{id,body}]``. Wall-clock time is taken from the wrapper's
    own ``time.time()`` rather than babble's reported value so it's
    comparable to the other tools.
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
