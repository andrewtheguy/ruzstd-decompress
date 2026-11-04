"""summary.py RUN.jsonl [BASE]: per workload and platform, each variant's median of its round medians, relative to BASE (A by default)."""
import json, statistics, sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1])]
base = sys.argv[2] if len(sys.argv) > 2 else "A"
med = defaultdict(list)
checks = defaultdict(set)
size = {}
for r in rows:
    k = (r["platform"], r["file"])
    med[k + (r["variant"],)].append(r["median_ms"])
    checks[r["file"]].add(r["check"])
    size[r["file"]] = (r["in"], r["out"])
bad = {f: c for f, c in checks.items() if len(c) > 1}
print("outputs differ:", bad if bad else "none")
for plat in [p for p in ("native", "wasm") if any(r["platform"] == p for r in rows)]:
    # A variant without a build for this platform (libzstd has no wasm) is left out of it
    variants = sorted({r["variant"] for r in rows if r["platform"] == plat})
    print(f"\n## {plat}: ms per pass (median of rounds), and vs {base}\n")
    print("| workload | MB out | " + " | ".join(variants) + f" | spread {base} |")
    print("|---|---:|" + "---:|" * len(variants) + "---:|")
    geo = defaultdict(list)
    for f in sorted({r["file"] for r in rows}):
        a = statistics.median(med[(plat, f, base)])
        cells = []
        for v in variants:
            m = statistics.median(med[(plat, f, v)])
            geo[v].append(m / a)
            cells.append(f"{m:.2f}" if v == base else f"{m:.2f} ({(m / a - 1) * 100:+.1f}%)")
        ra = med[(plat, f, base)]
        spread = (max(ra) - min(ra)) / a * 100
        print(f"| {f.replace('-lossless', '').replace('.tiles', '')} | {size[f][1] / 1e6:.1f} | " + " | ".join(cells) + f" | {spread:.1f}% |")
    print(f"| **geomean vs {base}** | | " + " | ".join(f"{(statistics.geometric_mean(geo[v]) - 1) * 100:+.1f}%" for v in variants) + " | |")
