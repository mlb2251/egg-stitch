#!/usr/bin/env python3
"""Convert an existing `sN`-leaved circuit corpus to per-cone free De Bruijn
variables (`$0..$k`). Generation already emits this form (see aig_to_egg.py);
this is for re-renaming corpora produced before the switch.

Usage: rename_vars.py <in.json> <out.json>"""
import sys, os, json

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from aig_cones import free_vars


def main():
    src, dst = sys.argv[1], sys.argv[2]
    out = [free_vars(p) for p in json.load(open(src))]
    json.dump(out, open(dst, "w"), indent=0)
    print(f"wrote {len(out)} programs ({len(set(out))} distinct) -> {dst}")


if __name__ == "__main__":
    main()
