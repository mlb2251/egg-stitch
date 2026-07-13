#!/usr/bin/env python3
"""Fetch the vendored EPFL `.aig` circuits for the current benchmark set.

Downloads from the EPFL Combinational Benchmark Suite (github.com/lsils/benchmarks)
at a pinned commit, so the gitignored `scripts/epfl-circuits/<circuit>.aig` blobs
reproduce byte-for-byte. With no args, fetches the aig for every committed corpus
in data/domains/epfl-circuits/; pass circuit names to fetch a subset.

Usage:
    python3 scripts/epfl-circuits/fetch_aigs.py            # the committed set
    python3 scripts/epfl-circuits/fetch_aigs.py voter log2
"""
import os
import sys
import time
import urllib.error
import urllib.request

# Pinned commit (2018-07-25 "Benchmark files."), so the download is immutable.
REF = "52b26f0e2cf1e88298a8b76c5e68e75013ba3977"

HERE = os.path.dirname(os.path.abspath(__file__))
DOMAIN = os.path.join(os.path.dirname(os.path.dirname(HERE)), "data", "domains", "epfl-circuits")

# Every EPFL circuit -> its suite subdirectory.
SUBDIR = {c: "arithmetic" for c in
          ["adder", "bar", "div", "hyp", "log2", "max", "multiplier", "sin", "sqrt", "square"]}
SUBDIR.update({c: "random_control" for c in
               ["arbiter", "cavlc", "ctrl", "dec", "i2c", "int2float", "mem_ctrl", "priority", "router", "voter"]})


def fetch(name: str) -> None:
    """Download circuit `name` to scripts/epfl-circuits/<name>.aig.

    raw.githubusercontent.com rate-limits (HTTP 429) when the CI matrix hammers
    it, so retry transient failures with exponential backoff before giving up.
    """
    url = f"https://raw.githubusercontent.com/lsils/benchmarks/{REF}/{SUBDIR[name]}/{name}.aig"
    req = urllib.request.Request(url, headers={"User-Agent": "egg-stitch"})
    for attempt in range(6):
        try:
            with urllib.request.urlopen(req, timeout=120) as resp:
                data = resp.read()
            break
        except (urllib.error.HTTPError, urllib.error.URLError) as e:
            # HTTPError carries a status code; a bare URLError (DNS/connection)
            # is always worth retrying. Give up on non-transient HTTP codes.
            transient = not isinstance(e, urllib.error.HTTPError) or e.code in (429, 500, 502, 503, 504)
            if not transient or attempt == 5:
                raise
            delay = 2**attempt
            print(f"{name}: {e}; retrying in {delay}s", file=sys.stderr)
            time.sleep(delay)
    assert data.startswith(b"aig "), f"{name}: not an AIGER file"
    with open(os.path.join(HERE, f"{name}.aig"), "wb") as f:
        f.write(data)
    print(f"{name}: wrote {len(data)} bytes")


def main():
    names = sys.argv[1:] or sorted(f[:-5] for f in os.listdir(DOMAIN) if f.endswith(".json"))
    for name in names:
        if name not in SUBDIR:
            raise SystemExit(f"unknown circuit {name!r}")
        fetch(name)


if __name__ == "__main__":
    main()
