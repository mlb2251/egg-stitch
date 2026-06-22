"""Compare no-DSR / at-start / live egg-stitch outputs to see whether keeping
DSRs live unlocks structure beyond one-shot canonicalization."""
import json

res = {t: json.load(open(f"/tmp/p.{t}.json")) for t in ["nodsr", "atstart", "live"]}

print("mode      final   compression  after_dsr")
for t, d in res.items():
    print(f"{t:8} {d['final_cost']:6}   {d['initial_cost']/d['final_cost']:.2f}x       {d['cost_after_rewrites']}")

# Bodies only (drop the fn_N: prefix); intra-library cross-refs make exact set
# diffing noisy, so also strip references to other fn_N when comparing.
def bodies(t):
    return [a["pattern"].split(": ", 1)[1] for a in res[t]["library"]]

live = bodies("live")
others = set(bodies("atstart")) | set(bodies("nodsr"))
live_only = [p for p in live if p not in others]

print(f"\nlive library size={len(live)}  at-start={len(bodies('atstart'))}  no-DSR={len(bodies('nodsr'))}")
print(f"abstractions in LIVE but in neither at-start nor no-DSR: {len(live_only)}")
for p in live_only:
    tag = "  <-- algebraic (+./*.)" if ("+." in p or "*." in p or "/." in p) else ""
    print(f"   {p[:120]}{tag}")
