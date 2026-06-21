#!/usr/bin/env python3
"""Fetch the vendored `multiplier.aig` from its upstream source, so the committed
blob has a reproducible provenance rather than being trusted blindly.

Source: the `multiplier` benchmark (a 64x64 multiplier: 128 inputs, 128 outputs,
27062 AND gates) from the EPFL Combinational Benchmark Suite, github.com/lsils/
benchmarks. The raw URL is pinned to a fixed commit (not a branch) so re-running
this always retrieves the exact same bytes as the vendored
scripts/epfl-circuits/multiplier.aig.

Usage:
    python3 scripts/epfl-circuits/fetch_multiplier_aig.py          # -> scripts/epfl-circuits/multiplier.aig
    python3 scripts/epfl-circuits/fetch_multiplier_aig.py OUT.aig  # -> explicit path
"""
import os
import sys
import urllib.request

# Pinned to the commit that last touched arithmetic/multiplier.aig (2018-07-25),
# so the download is immutable. Bump this ref to intentionally re-vendor.
REF = "52b26f0e2cf1e88298a8b76c5e68e75013ba3977"
URL = f"https://raw.githubusercontent.com/lsils/benchmarks/{REF}/arithmetic/multiplier.aig"

# The vendored .aig lives alongside the generators that consume it.
DEFAULT_OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "multiplier.aig")


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_OUT
    os.makedirs(os.path.dirname(out), exist_ok=True)
    req = urllib.request.Request(URL, headers={"User-Agent": "egg-stitch"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = resp.read()
    with open(out, "wb") as f:
        f.write(data)
    # AIGER header sanity check: `aig M I L O A`.
    header = data.split(b"\n", 1)[0].decode("ascii", "replace")
    assert header.startswith("aig "), f"not an AIGER file (header: {header!r})"
    print(f"wrote {len(data)} bytes -> {out}")
    print(f"  source: {URL}")
    print(f"  header: {header}")


if __name__ == "__main__":
    main()
