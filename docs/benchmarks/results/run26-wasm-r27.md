outputs differ: none

## wasm: ms per pass (median of rounds), and vs M26

| workload | MB out | M26 | R27 | spread M26 |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 15.20 | 15.17 (-0.2%) | 9.6% |
| github-dict.frames | 7.5 | 8.51 | 8.69 (+2.1%) | 7.7% |
| github.frames | 7.5 | 25.56 | 25.37 (-0.7%) | 2.2% |
| mixed-2880x1800@2-200M | 491.3 | 666.40 | 665.75 (-0.1%) | 3.9% |
| silesia-nci.l3.zst | 33.6 | 31.97 | 32.53 (+1.8%) | 1.2% |
| silesia-x-ray.l1.zst | 8.5 | 12.86 | 12.10 (-5.9%) | 6.2% |
| silesia.tar.fast4.zst | 212.0 | 226.09 | 225.68 (-0.2%) | 4.0% |
| silesia.tar.l19.zst | 212.0 | 367.67 | 376.36 (+2.4%) | 5.2% |
| silesia.tar.l3.zst | 212.0 | 377.90 | 367.09 (-2.9%) | 3.0% |
| terminal-2880x1800@2 | 3.4 | 1.18 | 1.22 (+3.0%) | 3.5% |
| **geomean vs M26** | | +0.0% | -0.1% | |
