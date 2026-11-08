"""threads-summary.py OUT.jsonl: threads.sh's lines as tables, natively and in wasm: ms a decode
(the median) on each number of threads, and the speed-up over one thread."""
import json
import os
import sys

rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
for platform in ("native", "wasm"):
    lines = [r for r in rows if r["platform"] == platform]
    if not lines:
        continue
    counts = sorted({r["threads"] for r in lines})
    files = list(dict.fromkeys(r["file"] for r in lines))
    print(f"\n## {platform}: ms a decode (median), and the speed-up over 1 thread\n")
    print("| workload | MB out | " + " | ".join(f"{n} thr" for n in counts) + " |")
    print("|---|---:|" + "---:|" * len(counts))
    for f in files:
        by = {r["threads"]: r for r in lines if r["file"] == f}
        one = by[counts[0]]["decode_ms"]
        cells = []
        for n in counts:
            ms = by[n]["decode_ms"]
            cells.append(f"{ms:.2f}" if n == counts[0] else f"{ms:.2f} ({one / ms:.2f}×)")
        mb = by[counts[0]]["bytes_out"] / 1e6
        print(f"| {os.path.basename(f)} | {mb:.1f} | " + " | ".join(cells) + " |")
