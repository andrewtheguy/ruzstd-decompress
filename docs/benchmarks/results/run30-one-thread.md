outputs differ: none

## native: ms per pass (median of rounds), and vs A

| workload | MB out | A | B | spread A |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 13.95 | 13.88 (-0.4%) | 0.5% |
| github-dict.frames | 7.5 | 3.50 | 3.59 (+2.4%) | 1.3% |
| github.frames | 7.5 | 15.24 | 15.23 (-0.1%) | 0.5% |
| mixed-2880x1800@2-200M | 491.3 | 421.07 | 421.59 (+0.1%) | 0.8% |
| silesia-nci.l3.zst | 33.6 | 19.07 | 19.30 (+1.2%) | 3.3% |
| silesia-x-ray.l1.zst | 8.5 | 8.79 | 9.10 (+3.5%) | 1.7% |
| silesia.tar.fast4.zst | 212.0 | 133.94 | 131.41 (-1.9%) | 3.6% |
| silesia.tar.l19.zst | 212.0 | 210.56 | 209.95 (-0.3%) | 2.3% |
| silesia.tar.l3.zst | 212.0 | 202.88 | 203.44 (+0.3%) | 1.0% |
| terminal-2880x1800@2 | 3.4 | 0.67 | 0.66 (-0.4%) | 3.7% |
| **geomean vs A** | | +0.0% | +0.4% | |

## wasm: ms per pass (median of rounds), and vs A

| workload | MB out | A | B | spread A |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 14.08 | 13.98 (-0.7%) | 1.9% |
| github-dict.frames | 7.5 | 5.42 | 5.49 (+1.3%) | 5.7% |
| github.frames | 7.5 | 21.77 | 21.70 (-0.4%) | 0.7% |
| mixed-2880x1800@2-200M | 491.3 | 528.33 | 531.48 (+0.6%) | 1.7% |
| silesia-nci.l3.zst | 33.6 | 25.40 | 25.36 (-0.2%) | 0.6% |
| silesia-x-ray.l1.zst | 8.5 | 10.12 | 10.29 (+1.7%) | 1.9% |
| silesia.tar.fast4.zst | 212.0 | 171.78 | 174.48 (+1.6%) | 2.4% |
| silesia.tar.l19.zst | 212.0 | 279.11 | 278.30 (-0.3%) | 8.2% |
| silesia.tar.l3.zst | 212.0 | 278.72 | 274.60 (-1.5%) | 5.4% |
| terminal-2880x1800@2 | 3.4 | 0.89 | 0.88 (-0.5%) | 0.9% |
| **geomean vs A** | | +0.0% | +0.2% | |
