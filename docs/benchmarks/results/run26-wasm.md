outputs differ: none

## wasm: ms per pass (median of rounds), and vs M26

| workload | MB out | M26 | R26 | spread M26 |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 15.26 | 15.43 (+1.1%) | 4.0% |
| github-dict.frames | 7.5 | 8.55 | 8.31 (-2.8%) | 2.8% |
| github.frames | 7.5 | 25.32 | 25.85 (+2.1%) | 1.7% |
| mixed-2880x1800@2-200M | 491.3 | 663.86 | 669.52 (+0.9%) | 7.7% |
| silesia-nci.l3.zst | 33.6 | 31.75 | 32.22 (+1.5%) | 1.7% |
| silesia-x-ray.l1.zst | 8.5 | 12.55 | 12.09 (-3.6%) | 13.9% |
| silesia.tar.fast4.zst | 212.0 | 225.96 | 228.73 (+1.2%) | 1.3% |
| silesia.tar.l19.zst | 212.0 | 373.89 | 378.39 (+1.2%) | 1.1% |
| silesia.tar.l3.zst | 212.0 | 361.48 | 370.12 (+2.4%) | 1.8% |
| terminal-2880x1800@2 | 3.4 | 1.20 | 1.23 (+2.2%) | 6.3% |
| **geomean vs M26** | | +0.0% | +0.6% | |
