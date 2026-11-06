outputs differ: none

## wasm: ms per pass (median of rounds), and vs Q28

| workload | MB out | Q28 | WS5 | spread Q28 |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 16.05 | 15.46 (-3.7%) | 10.2% |
| github-dict.frames | 7.5 | 8.41 | 8.32 (-1.1%) | 1.6% |
| github.frames | 7.5 | 25.51 | 25.32 (-0.7%) | 4.9% |
| mixed-2880x1800@2-200M | 491.3 | 668.48 | 675.03 (+1.0%) | 6.7% |
| silesia-nci.l3.zst | 33.6 | 32.76 | 32.37 (-1.2%) | 8.2% |
| silesia-x-ray.l1.zst | 8.5 | 12.74 | 12.85 (+0.9%) | 10.3% |
| silesia.tar.fast4.zst | 212.0 | 231.56 | 226.23 (-2.3%) | 6.4% |
| silesia.tar.l19.zst | 212.0 | 378.11 | 375.15 (-0.8%) | 4.4% |
| silesia.tar.l3.zst | 212.0 | 372.15 | 365.53 (-1.8%) | 8.3% |
| terminal-2880x1800@2 | 3.4 | 1.30 | 1.29 (-0.8%) | 1.9% |
| **geomean vs Q28** | | +0.0% | -1.1% | |
