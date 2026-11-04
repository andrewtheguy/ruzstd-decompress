"""summary.py RUN.jsonl: per workload and platform, each variant's median of its round medians, relative to A."""
import json, statistics, sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1])]
med = defaultdict(list)
checks = defaultdict(set)
size = {}
for r in rows:
    k = (r["platform"], r["file"])
    med[k + (r["variant"],)].append(r["median_ms"])
    checks[r["file"]].add(r["check"])
    size[r["file"]] = (r["in"], r["out"])
variants = sorted({r["variant"] for r in rows})
bad = {f: c for f, c in checks.items() if len(c) > 1}
print("outputs differ:", bad if bad else "none")
for plat in [p for p in ("native", "wasm") if any(r["platform"] == p for r in rows)]:
    print(f"\n## {plat}: ms per pass (median of rounds), and vs A\n")
    print("| workload | MB out | " + " | ".join(variants) + " | spread A |")
    print("|---|---:|" + "---:|" * len(variants) + "---:|")
    geo = defaultdict(list)
    for f in sorted({r["file"] for r in rows}):
        a = statistics.median(med[(plat, f, "A")])
        cells = []
        for v in variants:
            m = statistics.median(med[(plat, f, v)])
            geo[v].append(m / a)
            cells.append(f"{m:.2f}" if v == "A" else f"{m:.2f} ({(m / a - 1) * 100:+.1f}%)")
        ra = med[(plat, f, "A")]
        spread = (max(ra) - min(ra)) / a * 100
        print(f"| {f.replace('-lossless', '').replace('.tiles', '')} | {size[f][1] / 1e6:.1f} | " + " | ".join(cells) + f" | {spread:.1f}% |")
    print("| **geomean vs A** | | " + " | ".join(f"{(statistics.geometric_mean(geo[v]) - 1) * 100:+.1f}%" for v in variants) + " | |")
