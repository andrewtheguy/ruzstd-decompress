# Run 33: D (C in place only where one job decodes the whole input: the final code) against main (A), natively

lab2, unpinned, three rounds, the variants alternating. Output sizes differ: none.

## native: D against A, ms a decode (median of rounds) and D ÷ A

| workload | MB out | 1 thr | 4 thr | 8 thr |
|---|---:|---:|---:|---:|
| silesia.tar.l3.4M-frames.zst | 212.0 | 219.12 → 206.03 (0.940) | 98.23 → 97.17 (0.989) | 47.74 → 48.05 (1.006) |
| silesia.tar.l3.zst | 212.0 | 220.19 → 206.55 (0.938) | 163.91 → 153.70 (0.938) | 127.29 → 117.00 (0.919) |
