#!/usr/bin/env python3
"""Run the equivalence oracle on every `*.out.json` under `data/expected_outputs/`.

Each fixture's oracle is declared in `tests/snapshots.toml` (the same manifest
that drives the Rust snapshot suite), via each case's `oracle` field:

  * "beta"            — β-only equivalence (the default)
  * "beta-flat"       — β-only, but parsing the flat TypeScript dialect
                        (`lam{n}` binder groups, variadic `app`, `define` as a
                        let). Required for `--language typescript` runs.
  * { rules = "..." } — β + the given DSR file (β alone can't bridge e.g.
                        `(* 0 ?x) ≡ 0`)
  * "circuit"         — exhaustive boolean truth-table equivalence
                        (check_circuit_equiv.py), definitive for the EPFL cones
  * { skip = "..." }  — oracle-intractable / intentionally-unsound fixture,
                        pinned by its snapshot test instead

The Rust `coverage` trial guarantees the manifest and the on-disk fixture tree
are in exact correspondence, so every `*.out.json` here has exactly one entry.

Exit 0 iff every applicable file checks out.
"""

import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CHECKER = HERE / "check_equiv.py"
CIRCUIT_CHECKER = HERE / "check_circuit_equiv.py"
ROOT = REPO / "data" / "expected_outputs"
MANIFEST = REPO / "tests" / "snapshots.toml"


def fixture_rel(case):
    """The fixture path a case owns, relative to ROOT (matches Case::fixture_path)."""
    if "fixture" in case:
        return f"{case['fixture']}.out.json"
    rel = case["input"]
    for prefix in ("data/domains/", "data/"):
        if rel.startswith(prefix):
            rel = rel[len(prefix):]
            break
    return rel.removesuffix(".json") + ".out.json"


def _is_typescript_case(case):
    """True iff the case runs `--language typescript` (the only dialect the
    flat oracle understands)."""
    args = case.get("args", [])
    return any(a == "--language" and args[i + 1] == "typescript" for i, a in enumerate(args[:-1]))


def load_oracles():
    """Map each fixture rel-path to its oracle spec from the manifest."""
    manifest = tomllib.loads(MANIFEST.read_text())
    oracles = {}
    for case in manifest.get("case", []):
        # dreamcoder cases use `glob` (no single input) but always set `fixture`.
        if "fixture" not in case and "input" not in case:
            raise SystemExit(f"manifest case {case['name']!r} has neither fixture nor input")
        oracle = case.get("oracle", "beta")
        # A TypeScript case checked with plain `beta` degrades silently (its
        # `lam{n}`/`app` heads read as opaque symbols); a `beta-flat` case that
        # isn't TypeScript is equally suspect. Enforce the CLAUDE.md "must" in
        # both directions so this can't fail open.
        is_ts = _is_typescript_case(case)
        if is_ts and oracle != "beta-flat":
            raise SystemExit(f"manifest case {case['name']!r} runs --language typescript but oracle is {oracle!r}, not \"beta-flat\"")
        if oracle == "beta-flat" and not is_ts:
            raise SystemExit(f"manifest case {case['name']!r} sets oracle = \"beta-flat\" but does not run --language typescript")
        oracles[fixture_rel(case)] = oracle
    return oracles


def main():
    oracles = load_oracles()
    paths = sorted(ROOT.rglob("*.out.json"))
    if not paths:
        print(f"no *.out.json under {ROOT}", file=sys.stderr)
        sys.exit(1)

    circuits = []
    # check_equiv batches, keyed by (rules file or None, flat?) so each distinct
    # `check_equiv.py` invocation (rules set × dialect) is one subprocess call.
    batches = {}
    for p in paths:
        rel = p.relative_to(ROOT).as_posix()
        oracle = oracles.get(rel)
        if oracle is None:
            print(f"no manifest case owns fixture {rel} (add a [[case]] to tests/snapshots.toml)", file=sys.stderr)
            sys.exit(1)
        if oracle == "circuit":
            circuits.append(p)
        elif isinstance(oracle, dict) and "skip" in oracle:
            print(f"skip ({oracle['skip']}): {rel}")
        elif isinstance(oracle, dict) and "rules" in oracle:
            batches.setdefault((oracle["rules"], False), []).append(p)
        elif oracle == "beta-flat":
            batches.setdefault((None, True), []).append(p)
        elif oracle == "beta":
            batches.setdefault((None, False), []).append(p)
        else:
            print(f"unknown oracle {oracle!r} for {rel}", file=sys.stderr)
            sys.exit(1)

    overall = 0
    if circuits:
        print(f"$ check_circuit_equiv.py <{len(circuits)} files>")
        res = subprocess.run([sys.executable, str(CIRCUIT_CHECKER), *map(str, circuits)], cwd=REPO)
        if res.returncode != 0:
            overall = res.returncode
    for (rules, flat), group in batches.items():
        cmd = [sys.executable, str(CHECKER), *[str(p) for p in group]]
        if rules:
            cmd += ["--rewrites", rules]
        if flat:
            cmd += ["--flat"]
        label = f"(rules={rules})" if rules else ("(β-only, flat)" if flat else "(β-only)")
        print(f"$ check_equiv.py {label} <{len(group)} files>")
        res = subprocess.run(cmd, cwd=REPO)
        if res.returncode != 0:
            overall = res.returncode
    sys.exit(overall)


if __name__ == "__main__":
    main()
