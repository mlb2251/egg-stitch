#!/usr/bin/env python3
"""Fetch the vendored EPFL `.aig` circuits, so the committed blobs have a
reproducible provenance rather than being trusted blindly.

Source: the EPFL Combinational Benchmark Suite, github.com/lsils/benchmarks.
The raw URLs are pinned to a fixed commit (not a branch) so re-running always
retrieves the exact same bytes as the vendored `scripts/epfl-circuits/*.aig`.
Each circuit is vendored under its short key (e.g. the EPFL `multiplier` is
`mult.aig`) so the key matches the corpus filename
`data/domains/epfl-circuits/<key>.json`.

Usage:
    python3 scripts/epfl-circuits/fetch_aigs.py            # all circuits
    python3 scripts/epfl-circuits/fetch_aigs.py mult bar   # only these
"""
import os
import sys
import urllib.request

# Pinned commit (2018-07-25 "Benchmark files."), so the download is immutable.
# Bump this ref to intentionally re-vendor.
REF = "52b26f0e2cf1e88298a8b76c5e68e75013ba3977"

# circuit key -> upstream EPFL `arithmetic/<name>.aig` basename.
CIRCUITS = {
    "mult": "multiplier",
    "square": "square",
    "bar": "bar",
}

HERE = os.path.dirname(os.path.abspath(__file__))


def fetch(key: str) -> None:
    """Download circuit `key` to scripts/epfl-circuits/<key>.aig."""
    url = f"https://raw.githubusercontent.com/lsils/benchmarks/{REF}/arithmetic/{CIRCUITS[key]}.aig"
    req = urllib.request.Request(url, headers={"User-Agent": "egg-stitch"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = resp.read()
    header = data.split(b"\n", 1)[0].decode("ascii", "replace")
    assert header.startswith("aig "), f"{key}: not an AIGER file (header: {header!r})"
    out = os.path.join(HERE, f"{key}.aig")
    with open(out, "wb") as f:
        f.write(data)
    print(f"{key}: wrote {len(data)} bytes -> {out}  ({header})")


def main():
    keys = sys.argv[1:] or list(CIRCUITS)
    for key in keys:
        if key not in CIRCUITS:
            raise SystemExit(f"unknown circuit {key!r}; known: {', '.join(CIRCUITS)}")
        fetch(key)


if __name__ == "__main__":
    main()
