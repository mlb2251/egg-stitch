#!/usr/bin/env python3
"""Benchmark a PR by running our SMC and best-first searches on two branches
and comparing.

Builds the egg-stitch binary in two ephemeral git worktrees (one at ``BASE``,
one at ``PR``), then drives all measurements from a single Python process,
swapping ``egg_stitch_bin`` between the two binaries per measurement. No
``git checkout`` happens in the main repo. For each (rep, domain, method,
DSR condition) we run base then PR back-to-back. The first rep is treated
as warmup and dropped from the aggregate. Babble and Stitch are not
invoked; only our two methods are timed. Prints a side-by-side mean elapsed
time and mean compression ratio per (domain, method). The scramble families in
``MOL_FAMILIES`` (run only with DSRs, since they're meaningless without them)
get their own ``molecules`` table with a per-family breakdown and geomean row.

It also diffs the committed `data/expected_outputs/**/*.out.json` fixtures
between the two branches, comparing every `compression_ratio` leaf, and emits a
"Compression vs <base>" report above the timing tables that flags any fixture
whose compression dropped. As a prerequisite (shared with the timing run) the
main worktree must be clean and the base ref must be up to date with its remote.

All search hyperparameters (SMC budget + per-target best-first cutoffs) live in
``scripts/bench_config.py`` — edit that one file to retune. Best-first runs to
convergence via a self-draining ``--max-forced-expansion`` cap where one bounds
the search (see the config), and falls back to a step limit elsewhere.

Testing toggles (module constants near the top): ``WORKTREES=False`` skips the
two-worktree base-vs-PR setup and instead builds + runs the current working tree
once, printing raw values with no comparison (and no clean-tree preflight);
``DO_SMC=False`` runs only best-first; ``WARMUP=False`` drops the warmup rep;
``MIN_RUNS``/``MAX_RUNS`` bound the adaptive rep count.

Usage:
    python scripts/bench_pr.py [BASE=main] [PR=<current-branch>]
"""

import json
import math
import subprocess
import sys
import time
import numpy as np
import tqdm
from itertools import product
from pathlib import Path
from statistics import mean, stdev

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import bench_config  # noqa: E402  (scripts/bench_config.py — the one knob file)
from expts.result import PerFileResult, egraph_min_from_bench  # noqa: E402
from expts.run_models import ours as _ours_mod  # noqa: E402
from expts.runner import run_method, _bench_cost  # noqa: E402

DOMAINS = ["nuts-bolts", "dials", "furniture", "wheels", "list", "physics"]
# DOMAINS = ["nuts-bolts", "dials"]

# Molecule scramble families (data/domains/molecules/scramble/): real PubChem
# substructure corpora whose shared backbone the symmetry DSRs must re-align in
# place of a canonical SMILES encoding. They're meaningless without the rewrites,
# so we run them only in the with-DSRs condition and report them in their own
# "molecules" table (per-family rows + geomean). op-children grammar -> "no-apps" weighting.
MOL_FAMILIES = ["hexyl", "ester", "glycol"]
MOL_DIR = ROOT / "data" / "domains" / "molecules" / "scramble"
MOL_REWRITES = "data/domains/molecules/molecules.rewrites"  # relative to egg-stitch cwd

# Adaptive rep count: keep adding reps until every cell's relative SEM
# (stdev / sqrt(n) / mean) is below TARGET_REL_SEM on both branches, or
# until MAX_RUNS. We can't use a fixed seed across branches (RNG draws
# don't have coherent meaning when search semantics change), so the only
# honest way to make the comparison meaningful is to drive the per-cell
# uncertainty down by sampling.
MIN_RUNS = 1
MAX_RUNS = 1
TARGET_REL_SEM = 0.02

# Temporary testing toggle: when False, skip SMC cells entirely and run/report
# only the best-first "enum" method.
DO_SMC = False

# Temporary testing toggle: when False, skip the two-worktree base-vs-PR setup
# and instead build + run the *current* working tree once, printing raw values
# with no comparison. Also skips the clean-tree / base-up-to-date preflight, so
# it doesn't care about a dirty tree.
WORKTREES = False

# Warmup rep (rep 0): an extra first run of every cell whose result is dropped
# from the aggregate, so cold-start effects (disk cache, CPU/frequency ramp,
# page-ins) don't skew the first *measured* rep. Disabled by default here for
# single-run testing — enable to restore the discarded warmup pass.
WARMUP = False

# Fixture-compression diff: where the committed outputs live, and the absolute
# ratio delta below which two blessed floats are treated as equal (their
# trailing digits are meaningless search noise).
EXPECTED_OUTPUTS = "data/expected_outputs"
COMP_EPS = 1e-9


def sh(cmd, *, cwd=None, **kw):
    """Run a subprocess, echoing the command first. Defaults cwd to the repo root."""
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run(cmd, check=True, cwd=cwd or ROOT, **kw)


def git_show(ref: str, rel: str):
    """Return the contents of ``rel`` at git ref ``ref``, or None if absent."""
    res = subprocess.run(
        ["git", "show", f"{ref}:{rel}"], cwd=ROOT, capture_output=True, text=True
    )
    return res.stdout if res.returncode == 0 else None


def rev_parse(ref: str):
    """Return the commit SHA ``ref`` resolves to, or None if it doesn't exist."""
    res = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", ref],
        cwd=ROOT, capture_output=True, text=True,
    )
    return res.stdout.strip() or None


def preflight(base: str) -> None:
    """Abort unless the main worktree is clean and ``base`` is current with its
    remote — both are prerequisites for a meaningful comparison.

    A dirty tree usually means the user is mid-edit (rarely what they want to
    benchmark), and a stale ``base`` would silently flag or bless the wrong
    compression numbers. We don't ``git checkout`` here — worktrees handle that.
    """
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True
    ).strip()
    if dirty:
        raise SystemExit(
            "bench_pr: working tree is not clean — commit or stash before running.\n"
            + dirty
        )
    fetch = subprocess.run(
        ["git", "fetch", "origin", base], cwd=ROOT, capture_output=True, text=True
    )
    if fetch.returncode != 0:
        raise SystemExit(f"bench_pr: `git fetch origin {base}` failed:\n{fetch.stderr.strip()}")
    local, remote = rev_parse(base), rev_parse("FETCH_HEAD")
    if local is None:
        raise SystemExit(f"bench_pr: baseline ref `{base}` does not exist locally")
    if local != remote:
        raise SystemExit(
            f"bench_pr: local `{base}` ({local[:12]}) is not up to date with "
            f"`origin/{base}` ({(remote or '?')[:12]}); run `git pull` on `{base}` first"
        )


def setup_worktree(branch: str, wt_dir: Path) -> Path:
    """Create a git worktree for ``branch`` at ``wt_dir``, build release, return binary path."""
    # --detach so we don't conflict with whichever branch the main worktree
    # currently has checked out (commonly the PR branch itself).
    sh(["git", "worktree", "add", "--detach", str(wt_dir), branch])
    sh(["cargo", "build", "--release", "--bin", "egg-stitch", "--quiet"], cwd=wt_dir)
    return wt_dir / "target" / "release" / "egg-stitch"


def teardown_worktree(wt_dir: Path) -> None:
    """Remove a git worktree (force-removing despite the untracked ``target/``)."""
    subprocess.run(
        ["git", "worktree", "remove", "--force", str(wt_dir)],
        cwd=ROOT, check=False,
    )


def build_current() -> Path:
    """Build release in the main repo (no worktree) and return the current
    binary path — used by single-run mode to time the working tree as-is."""
    sh(["cargo", "build", "--release", "--bin", "egg-stitch", "--quiet"])
    return ROOT / "target" / "release" / "egg-stitch"


def time_cell(binary_path: Path, runner, domain: str, use_dsrs: bool, cache_path: Path):
    """Run one (binary, domain, method, condition) cell, going through the cache.

    Monkey-patches ``expts.run_models.ours.egg_stitch_bin`` so ``run_method``
    invokes the requested binary; the rest of the pipeline (cwd, env,
    command-line construction) is shared between branches.
    """
    if cache_path.exists():
        with open(cache_path) as f:
            return [PerFileResult(**d) for d in json.load(f)]
    _ours_mod.egg_stitch_bin = lambda: binary_path
    out = run_method(runner, domain, rounds=1, use_dsrs=use_dsrs)
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with open(cache_path, "w") as f:
        json.dump([r.to_dict() for r in out], f, indent=2)
    return out


def run_mol_family(runner, family: str) -> list[PerFileResult]:
    """Run one molecule scramble ``family`` (always with the symmetry DSRs live)
    and return a one-element ``[PerFileResult]`` matching ``run_method``'s shape.

    Bypasses ``run_method``'s domain table since the scramble corpora aren't
    registered domains; the uniform ``no-apps`` cost recomputation is still
    shared via ``_bench_cost``.
    """
    weighting = "no-apps"
    f = MOL_DIR / f"{family}.scram.json"
    b = runner(1, f, MOL_REWRITES, weighting)
    ic, fc = _bench_cost(b, weighting)
    assert fc > 0, f"molecules/{family}: final_cost=0 would make compression_ratio undefined"
    # Same freeform stats firehose as runner.run_method (full JSON minus the
    # bulky program arrays).
    stats = {k: v for k, v in b.raw.items()
             if k not in ("original_programs", "rewritten_programs")} or None
    return [PerFileResult(
        method=str(runner),
        domain=f"molecules:{family}",
        file=f.stem,
        initial_cost=ic,
        final_cost=fc,
        compression_ratio=ic / fc,
        elapsed_secs=b.elapsed_secs,
        library=[f"{a.name}: {a.body}" for a in b.abstractions],
        egraph_min_term_size=egraph_min_from_bench(b.cost_after_rewrites),
        stats=stats,
    )]


def time_mol_cell(binary_path: Path, runner, family: str, cache_path: Path):
    """Cache-backed run of one molecule-family cell (mirrors ``time_cell``)."""
    if cache_path.exists():
        with open(cache_path) as f:
            return [PerFileResult(**d) for d in json.load(f)]
    _ours_mod.egg_stitch_bin = lambda: binary_path
    out = run_mol_family(runner, family)
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with open(cache_path, "w") as f:
        json.dump([r.to_dict() for r in out], f, indent=2)
    return out


def cache_path_for(session: str, branch_label: str, dsr_label: str, method: str, domain: str, rep_idx: int) -> Path:
    """Per-cell cache file path. Unique per (branch, condition, method, domain, rep)."""
    return (
        ROOT / "viz" / "results" / "bench_pr" / session
        / branch_label / dsr_label / method / domain / f"rep{rep_idx}.json"
    )


def cell_per_rep_times(session: str, branch_label: str, dsr_label: str, method: str, domain: str, num_reps: int) -> list[float]:
    """Per-rep summed elapsed_secs for one cell (reps 1..num_reps; rep 0 is warmup)."""
    out = []
    for rep in range(1, num_reps + 1):
        p = cache_path_for(session, branch_label, dsr_label, method, domain, rep)
        with open(p) as f:
            files = json.load(f)
        out.append(sum(r["elapsed_secs"] for r in files))
    return out


def cell_compressions(session: str, branch_label: str, dsr_label: str, method: str, domain: str, num_reps: int) -> list[float]:
    """Per-file compression ratios across reps 1..num_reps for one cell."""
    out = []
    for rep in range(1, num_reps + 1):
        p = cache_path_for(session, branch_label, dsr_label, method, domain, rep)
        with open(p) as f:
            files = json.load(f)
        out.extend(r["compression_ratio"] for r in files)
    return out


def rel_sem(xs: list[float]) -> float:
    """stdev/sqrt(n)/mean — the SEM as a fraction of the mean. inf if mean=0 or n<2."""
    if len(xs) < 2:
        return float("inf")
    m = mean(xs)
    if m == 0:
        return float("inf")
    return stdev(xs) / math.sqrt(len(xs)) / m


def summarize(session: str, branch_label: str, dsr_label: str, methods: list[str], reps_for: dict, domains: list[str] = DOMAINS) -> dict:
    """Aggregate cached per-cell results (dropping rep 0 as warmup) into
    ``{domain: {method: {time, compression}}}``.

    Per-rep ``time`` sums elapsed_secs across files of a domain; the cell's
    reported ``time`` is the mean of those per-rep totals. ``compression``
    is the mean of every file's compression_ratio across the kept reps.
    ``reps_for`` maps ``(dsr_label, domain, method) -> num_reps`` since
    different cells may have run different rep counts under the adaptive
    sampler.
    """
    out: dict = {}
    for domain in domains:
        out[domain] = {}
        for method in methods:
            n = reps_for[(dsr_label, domain, method)]
            times = cell_per_rep_times(session, branch_label, dsr_label, method, domain, n)
            ratios = cell_compressions(session, branch_label, dsr_label, method, domain, n)
            out[domain][method] = {
                "time": mean(times),
                "compression": mean(ratios),
            }
    return out


def update_pr_report(pr_branch: str, report_section: str) -> None:
    """Replace (or append) the managed compression + timing block in the PR.

    Looks up the open PR for ``pr_branch`` via ``gh``; if none exists, prints
    a warning and returns. ``report_section`` is the combined block: a
    ``## Compression vs`` section followed by ``## Timing and fixture
    regressions``. The managed region is matched as an optional compression
    section immediately followed by the timing section (which absorbs an older
    bare ``## Timing`` heading), up to the next ``## `` heading or EOF; if not
    found it's appended (separated by a blank line).
    """
    try:
        body = subprocess.check_output(
            ["gh", "pr", "view", pr_branch, "--json", "body", "-q", ".body"],
            cwd=ROOT, text=True, stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as e:
        print(f"\nbench_pr: no PR found for branch {pr_branch!r}, skipping PR update.\n  {e.stderr.strip()}")
        return
    body = body.rstrip("\n")
    import re
    pattern = re.compile(
        r"(?m)^(?:## Compression vs\b.*?(?=^## ))?## Timing\b.*?(?=^## |\Z)", re.DOTALL
    )
    if pattern.search(body):
        new_body = pattern.sub(lambda _: report_section.rstrip() + "\n\n", body).rstrip() + "\n"
    else:
        sep = "\n\n" if body else ""
        new_body = body + sep + report_section.rstrip() + "\n"
    res = subprocess.run(
        ["gh", "pr", "edit", pr_branch, "--body-file", "-"],
        cwd=ROOT, input=new_body, text=True, capture_output=True,
    )
    if res.returncode != 0:
        print(f"\nbench_pr: gh pr edit failed (exit {res.returncode}):\n{res.stderr}")
    else:
        print(f"\nbench_pr: updated Timing section on PR for {pr_branch}.")


def _speedup_emoji(speedup: float, band: float = 0.02) -> str:
    """Green above ``1+band``, red below ``1-band``, gray for the in-between band.

    ``band`` is the half-width of the gray "no meaningful change" zone: 0.02 (2%)
    for geomean rows, 0.05 (5%) for per-domain rows whose individual noise is
    larger.
    """
    if speedup > 1 + band:
        return "🟢"
    if speedup < 1 - band:
        return "🔴"
    return "⚪"


def _comp_leaves(obj, prefix=""):
    """Yield (key, ratio) for every ``compression_ratio`` in ``obj``.

    A dict carrying the key is one result (we don't recurse into it); any other
    dict is a container whose values are recursed into, extending the key path
    (DSR fixtures map per-benchmark names to result objects). A ``null`` ratio
    means no invention was applied, so cost is unchanged — normalised to 1.0, as
    ``src/main.rs`` computes ``original_size / final_cost`` and only emits None
    when no rewrite happened.
    """
    if isinstance(obj, dict):
        if "compression_ratio" in obj:
            ratio = obj["compression_ratio"]
            yield prefix, 1.0 if ratio is None else ratio
        else:
            for name, val in obj.items():
                yield from _comp_leaves(val, f"{prefix}::{name}" if prefix else name)


def _comp_leaves_at(ref: str, rel: str) -> dict:
    """Return ``{key: ratio}`` for every compression leaf of fixture ``rel`` at
    git ref ``ref`` (keyed by ``rel`` plus any in-file benchmark path). Empty if
    the file is absent there or unparseable."""
    text = git_show(ref, rel)
    if text is None:
        return {}
    try:
        obj = json.loads(text)
    except json.JSONDecodeError:
        return {}
    return {f"{rel}::{k}" if k else rel: r for k, r in _comp_leaves(obj)}


def compression_section(base: str, pr: str) -> str:
    """Build the markdown "Compression vs <base>" report comparing every output
    fixture's ``compression_ratio`` leaf on ``pr`` against ``base``.

    Fixtures are listed from ``pr``; each leaf is bucketed into regressed,
    improved, or new (absent on ``base``). Unchanged leaves are summarised but
    not listed.
    """
    listing = subprocess.check_output(
        ["git", "ls-tree", "-r", "--name-only", pr, "--", EXPECTED_OUTPUTS],
        cwd=ROOT, text=True,
    )
    fixtures = sorted(p for p in listing.splitlines() if p.endswith(".out.json"))

    higher, lower, new, n_equal = [], [], [], 0
    for rel in fixtures:
        cur = _comp_leaves_at(pr, rel)
        base_leaves = _comp_leaves_at(base, rel)
        for key, c in cur.items():
            if key not in base_leaves:
                new.append((key, c))
                continue
            delta = c - base_leaves[key]
            if delta > COMP_EPS:
                higher.append((key, c, base_leaves[key]))
            elif delta < -COMP_EPS:
                lower.append((key, c, base_leaves[key]))
            else:
                n_equal += 1
    higher.sort(); lower.sort(); new.sort()

    total = len(higher) + len(lower) + n_equal + len(new)
    lines = [f"## Compression vs `{base}`", ""]
    lines.append(
        f"Compared **{total}** compression values across **{len(fixtures)}** fixtures: "
        f"{len(lower)} regressed, {len(higher)} improved, {n_equal} unchanged, "
        f"{len(new)} new (no baseline)."
    )
    lines.append("")
    lines.append(
        "**✅ ALL OUTPUTS HAVE GREATER-OR-EQUAL COMPRESSION**"
        if not lower else f"**❌ {len(lower)} OUTPUT(S) REGRESSED**"
    )

    def delta_table(title, rows):
        if not rows:
            return
        lines.extend(["", f"### {title} ({len(rows)})", "",
                      f"| fixture | `{base}` | `{pr}` | delta |", "|---|---:|---:|---:|"])
        for key, c, b in rows:
            lines.append(f"| {key} | {b:.6f} | {c:.6f} | {c - b:+.6f} |")

    delta_table("⬇️ Lower compression (regressions)", lower)
    delta_table("⬆️ Higher compression (improvements)", higher)
    if new:
        lines.extend(["", f"### 🆕 New (not on `{base}`) ({len(new)})", "",
                      f"| fixture | `{pr}` |", "|---|---:|"])
        for key, c in new:
            lines.append(f"| {key} | {c:.6f} |")
    return "\n".join(lines)


def fmt_table(base_label: str, pr_label: str, base: dict, pr: dict, title: str,
              domains: list[str] = DOMAINS, unconverged: set = frozenset(),
              methods: tuple = ("enum", "smc")) -> str:
    """Return a GitHub-flavored markdown comparison table.

    Emits one row per entry in ``domains`` plus a trailing geomean row, for
    each method in ``methods`` (``enum``/``smc``; SMC omitted when ``DO_SMC``).

    Per-domain rows use a 5% gray band (their per-cell noise is larger); the
    geomean row keeps the tighter 2% band. A ``(domain, method)`` in
    ``unconverged`` — its rel-SEM stayed above the sampler's target at the end
    of the run — is flagged 🟡 in place of the speedup color.
    """
    lines = [
        f"### {title} — `{pr_label}` vs `{base_label}`",
        "",
        f"|   | domain | method | time `{base_label}` [s] | time `{pr_label}` [s] | speedup | comp `{base_label}` | comp `{pr_label}` |",
        "|---|---|---|---:|---:|---:|---:|---:|",
    ]

    def cell(b: dict, p: dict):
        """(time_base, time_pr, speedup, comp_base, comp_pr) for one domain/method."""
        return (b["time"], p["time"], b["time"] / p["time"], b["compression"], p["compression"])

    def geomean(rows):
        return np.prod(rows, axis=0) ** (1 / len(rows))

    for m in methods:
        rows = [cell(base[dom][m], pr[dom][m]) for dom in domains]
        labels = list(domains)
        rows.append(geomean(rows)); labels.append("geomean")
        for dom, (t_base, t_pr, speedup, c_base, c_pr) in zip(labels, rows):
            is_geomean = dom == "geomean"
            if not is_geomean and (dom, m) in unconverged:
                emoji = "🟡"
            else:
                emoji = _speedup_emoji(speedup, 0.02 if is_geomean else 0.05)
            comp_warn = " ‼️" if c_pr / c_base < 0.99 else ""
            lines.append(f"| {emoji}{comp_warn} | {dom} | {m} | {t_base:.3f} | {t_pr:.3f} | {speedup:.2f}x | {c_base:.3f} | {c_pr:.3f} |")
    return "\n".join(lines)


def fmt_single_table(label: str, data: dict, title: str,
                     domains: list[str] = DOMAINS, methods: tuple = ("enum", "smc")) -> str:
    """Return a single-run markdown table (no comparison): one row per domain
    plus a geomean row, per method, with raw time and compression."""
    lines = [
        f"### {title} — `{label}`",
        "",
        "| domain | method | time [s] | compression |",
        "|---|---|---:|---:|",
    ]

    def geomean(rows):
        return np.prod(rows, axis=0) ** (1 / len(rows))

    for m in methods:
        rows = [(data[dom][m]["time"], data[dom][m]["compression"]) for dom in domains]
        labels = list(domains)
        rows.append(geomean(rows)); labels.append("geomean")
        for dom, (t, c) in zip(labels, rows):
            lines.append(f"| {dom} | {m} | {t:.3f} | {c:.3f} |")
    return "\n".join(lines)


def main() -> None:
    """CLI entry point; see module docstring for the argument shape."""
    args = sys.argv[1:]
    base = args[0] if len(args) >= 1 else "main"
    pr = args[1] if len(args) >= 2 else subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
    session = time.strftime("%Y-%m-%d_%H-%M-%S")

    # Worktree/comparison prerequisites apply only in base-vs-PR mode.
    comp_section = None
    if WORKTREES:
        preflight(base)
        # Built up front so a fixture regression is reported even if the (long)
        # timing run is interrupted afterwards.
        comp_section = compression_section(base, pr)

    bf_summary = {
        t: (f"mfe{r.max_forced_expansion}" if r.max_forced_expansion is not None else f"{r.num_steps}steps")
        for t, r in bench_config.BF_RUNNERS.items()
    }
    mode = "worktrees (base-vs-PR)" if WORKTREES else "single-run (current, no comparison)"
    print(f"mode={mode}  base={base}  pr={pr}  reps=(min={MIN_RUNS}, max={MAX_RUNS})  warmup={WARMUP}  do_smc={DO_SMC}  "
          f"smc=({bench_config.SMC_STEPS} steps, {bench_config.SMC_PARTICLES} particles, T={bench_config.SMC_TEMP})  "
          f"bf={bf_summary}  session={session}")

    wt_root = Path(f"/tmp/bench_pr_{session}")
    wt_base = wt_root / "base"
    wt_pr = wt_root / "pr"
    try:
        if WORKTREES:
            # base-vs-PR: build both refs in ephemeral worktrees.
            branches = [("base", setup_worktree(base, wt_base)),
                        ("pr", setup_worktree(pr, wt_pr))]
        else:
            # single-run: build + run the current working tree, one branch.
            branches = [("current", build_current())]
        branch_labels = [label for label, _ in branches]

        # SMC is shared across targets; best-first is per-target (its cutoff —
        # forced-expansion cap vs step limit — depends on the domain). All knobs
        # live in scripts/bench_config.py.
        smc_runner = bench_config.smc_runner()
        methods = ["enum", "smc"] if DO_SMC else ["enum"]

        def runner_for(method: str, domain: str):
            """The runner for one (method, domain): per-target best-first, shared SMC."""
            return smc_runner if method == "smc" else bench_config.BF_RUNNERS[domain]

        conditions = [("with_dsrs", True), ("without_dsrs", False)]
        # Each cell is keyed by (dsr_label, domain, method); runner + use_dsrs
        # are recovered from these lookup tables.
        use_dsrs_for = dict(conditions)
        cell_keys = [(d, dom, m) for (d, _), dom, m in product(conditions, DOMAINS, methods)]
        # Molecule scramble families run only with DSRs (see MOL_FAMILIES).
        cell_keys += [("with_dsrs", fam, m) for fam in MOL_FAMILIES for m in methods]

        def run_rep_for(cell: tuple[str, str, str], rep_idx: int) -> None:
            """Run one rep of one cell across every branch (both refs in
            worktree mode; just the current tree otherwise)."""
            dsr_label, domain, method = cell
            runner = runner_for(method, domain)
            for label, binary in branches:
                if domain in MOL_FAMILIES:
                    time_mol_cell(binary, runner, domain,
                                  cache_path_for(session, label, dsr_label, method, domain, rep_idx))
                else:
                    use_dsrs = use_dsrs_for[dsr_label]
                    time_cell(binary, runner, domain, use_dsrs,
                              cache_path_for(session, label, dsr_label, method, domain, rep_idx))

        def cell_rel_sem(cell: tuple[str, str, str], num_reps: int) -> float:
            """Max rel-SEM across all branches for one cell at num_reps."""
            dsr_label, domain, method = cell
            return max(
                rel_sem(cell_per_rep_times(session, label, dsr_label, method, domain, num_reps))
                for label in branch_labels
            )

        # Phase 1: warmup + MIN_RUNS reps for every cell. Phase 2: keep
        # adding reps only to cells whose paired rel-SEM (max of base/PR)
        # is still above TARGET_REL_SEM, capped at MAX_RUNS per cell.
        reps_done: dict[tuple[str, str, str], int] = {c: 0 for c in cell_keys}

        def cell_done(cell: tuple[str, str, str]) -> bool:
            n = reps_done[cell]
            if n < MIN_RUNS:
                return False
            if n >= MAX_RUNS:
                return True
            return cell_rel_sem(cell, n) < TARGET_REL_SEM

        # Warmup rep (rep 0): an extra first run per cell, dropped from the
        # aggregate to absorb cold-start effects. Skipped when WARMUP is off.
        if WARMUP:
            wpbar = tqdm.tqdm(cell_keys, desc="warmup", unit="cell", leave=False)
            for cell in wpbar:
                wpbar.set_postfix_str("/".join(cell))
                run_rep_for(cell, 0)

        # Adaptive sampling: each outer iteration runs one more rep for
        # every still-unconverged cell. Stop when no cell needs more reps.
        round_idx = 0
        while True:
            pending = [c for c in cell_keys if not cell_done(c)]
            if not pending:
                break
            round_idx += 1
            pbar = tqdm.tqdm(pending, desc=f"round {round_idx} ({len(pending)} cells)", unit="cell", leave=False)
            for cell in pbar:
                reps_done[cell] += 1
                pbar.set_postfix_str(f"{'/'.join(cell)} rep{reps_done[cell]}")
                run_rep_for(cell, reps_done[cell])
            # rel-SEM is undefined for a single sample, so only report it once a
            # cell has >= 2 reps; with MAX_RUNS=1 this stays silent per round.
            for cell in pending:
                n = reps_done[cell]
                if n >= 2:
                    r = cell_rel_sem(cell, n)
                    print(f"  {'/'.join(cell)}: {n} reps, rel-SEM={r:.2%}{' ✓' if r < TARGET_REL_SEM else ''}", flush=True)

        # One quiet end-of-run summary instead of per-cell warning spam (and
        # silent under single-run, where rel-SEM is undefined at 1 sample).
        unconverged_cells = [
            c for c, n in reps_done.items()
            if n >= 2 and n >= MAX_RUNS and cell_rel_sem(c, n) >= TARGET_REL_SEM
        ]
        if unconverged_cells:
            print(f"{len(unconverged_cells)} cell(s) hit MAX_RUNS={MAX_RUNS} without reaching "
                  f"rel-SEM<{TARGET_REL_SEM:.0%}", flush=True)

        with_reps = {(d, dom, m): reps_done[(d, dom, m)] for d in ("with_dsrs",) for dom in DOMAINS for m in methods}
        without_reps = {(d, dom, m): reps_done[(d, dom, m)] for d in ("without_dsrs",) for dom in DOMAINS for m in methods}
        mol_reps = {("with_dsrs", fam, m): reps_done[("with_dsrs", fam, m)] for fam in MOL_FAMILIES for m in methods}

        def unconverged_set(dsr_label: str, domains: list[str]) -> set:
            """{(domain, method)} whose final rel-SEM stayed >= TARGET_REL_SEM,
            i.e. the adaptive sampler never drove the cell's noise under target
            (it hit MAX_RUNS first)."""
            return {
                (dom, m) for dom in domains for m in methods
                if cell_rel_sem((dsr_label, dom, m), reps_done[(dsr_label, dom, m)]) >= TARGET_REL_SEM
            }

        if WORKTREES:
            with_md = fmt_table(base, pr,
                                summarize(session, "base", "with_dsrs", methods, with_reps),
                                summarize(session, "pr", "with_dsrs", methods, with_reps),
                                "with DSRs",
                                unconverged=unconverged_set("with_dsrs", DOMAINS),
                                methods=methods)
            without_md = fmt_table(base, pr,
                                   summarize(session, "base", "without_dsrs", methods, without_reps),
                                   summarize(session, "pr", "without_dsrs", methods, without_reps),
                                   "without DSRs",
                                   unconverged=unconverged_set("without_dsrs", DOMAINS),
                                   methods=methods)
            mol_md = fmt_table(base, pr,
                               summarize(session, "base", "with_dsrs", methods, mol_reps, domains=MOL_FAMILIES),
                               summarize(session, "pr", "with_dsrs", methods, mol_reps, domains=MOL_FAMILIES),
                               "molecules (with DSRs)",
                               domains=MOL_FAMILIES,
                               unconverged=unconverged_set("with_dsrs", MOL_FAMILIES),
                               methods=methods)
            timing_section = ("## Timing and fixture regressions\n\n"
                              + with_md + "\n\n" + without_md + "\n\n" + mol_md + "\n")
            report_section = comp_section.rstrip() + "\n\n" + timing_section
            print()
            print(report_section)
            update_pr_report(pr, report_section)
        else:
            # Single-run: raw values for the current tree, no comparison / PR update.
            label = branch_labels[0]
            with_md = fmt_single_table(label, summarize(session, label, "with_dsrs", methods, with_reps),
                                       "with DSRs", methods=methods)
            without_md = fmt_single_table(label, summarize(session, label, "without_dsrs", methods, without_reps),
                                          "without DSRs", methods=methods)
            mol_md = fmt_single_table(label, summarize(session, label, "with_dsrs", methods, mol_reps, domains=MOL_FAMILIES),
                                      "molecules (with DSRs)", domains=MOL_FAMILIES, methods=methods)
            report_section = ("## Timing (single run, no comparison)\n\n"
                              + with_md + "\n\n" + without_md + "\n\n" + mol_md + "\n")
            print()
            print(report_section)
            # Persist the summary table alongside the per-cell JSON cache.
            report_path = ROOT / "viz" / "results" / "bench_pr" / session / "report.md"
            report_path.parent.mkdir(parents=True, exist_ok=True)
            report_path.write_text(report_section)
            print(f"report written to {report_path}")
    finally:
        if WORKTREES:
            teardown_worktree(wt_base)
            teardown_worktree(wt_pr)


if __name__ == "__main__":
    main()
