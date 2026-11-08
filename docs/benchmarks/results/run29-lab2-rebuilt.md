outputs differ: none

## native: ms per pass (median of rounds), and vs M29

| workload | MB out | M29 | Z | spread M29 |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 13.69 | 19.47 (+42.2%) | 1.2% |
| github-dict.frames | 7.5 | 3.49 | 3.14 (-10.0%) | 6.5% |
| github.frames | 7.5 | 15.28 | 16.48 (+7.8%) | 1.1% |
| mixed-2880x1800@2-200M | 491.3 | 426.10 | 345.54 (-18.9%) | 3.1% |
| silesia-nci.l3.zst | 33.6 | 19.27 | 17.09 (-11.3%) | 1.6% |
| silesia-x-ray.l1.zst | 8.5 | 8.79 | 7.23 (-17.8%) | 0.3% |
| silesia.tar.fast4.zst | 212.0 | 132.81 | 116.25 (-12.5%) | 0.8% |
| silesia.tar.l19.zst | 212.0 | 208.97 | 187.56 (-10.2%) | 0.5% |
| silesia.tar.l3.zst | 212.0 | 203.09 | 176.50 (-13.1%) | 0.7% |
| terminal-2880x1800@2 | 3.4 | 0.66 | 1.36 (+104.8%) | 0.8% |
| **geomean vs M29** | | +0.0% | +1.3% | |

## wasm: ms per pass (median of rounds), and vs M29

| workload | MB out | M29 | spread M29 |
|---|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 13.87 | 2.8% |
| github-dict.frames | 7.5 | 5.43 | 1.7% |
| github.frames | 7.5 | 21.69 | 1.7% |
| mixed-2880x1800@2-200M | 491.3 | 524.85 | 2.6% |
| silesia-nci.l3.zst | 33.6 | 25.30 | 1.2% |
| silesia-x-ray.l1.zst | 8.5 | 9.99 | 0.9% |
| silesia.tar.fast4.zst | 212.0 | 173.25 | 4.3% |
| silesia.tar.l19.zst | 212.0 | 278.37 | 1.8% |
| silesia.tar.l3.zst | 212.0 | 276.64 | 3.9% |
| terminal-2880x1800@2 | 3.4 | 0.88 | 1.6% |
| **geomean vs M29** | | +0.0% | |
