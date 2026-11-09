# Run 33: the one-thread harness, L (the ring and decode buffer of B, C and D) against main (M)

lab2, pinned to vCPU 2, three rounds.

outputs differ: none

## native: ms per pass (median of rounds), and vs M

| workload | MB out | L | M | spread M |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 13.93 (+0.1%) | 13.91 | 0.6% |
| github-dict.frames | 7.5 | 3.55 (+0.9%) | 3.52 | 0.5% |
| github.frames | 7.5 | 15.25 (-0.8%) | 15.37 | 14.9% |
| mixed-2880x1800@2-200M | 491.3 | 420.31 (-0.5%) | 422.38 | 1.7% |
| silesia-nci.l3.zst | 33.6 | 19.03 (-0.6%) | 19.15 | 2.1% |
| silesia-x-ray.l1.zst | 8.5 | 9.13 (+0.7%) | 9.07 | 0.7% |
| silesia.tar.fast4.zst | 212.0 | 130.74 (-1.5%) | 132.73 | 1.6% |
| silesia.tar.l19.zst | 212.0 | 211.01 (+1.5%) | 207.82 | 3.2% |
| silesia.tar.l3.zst | 212.0 | 201.18 (+0.3%) | 200.56 | 0.9% |
| terminal-2880x1800@2 | 3.4 | 0.66 (+1.5%) | 0.65 | 1.5% |
| **geomean vs M** | | +0.2% | +0.0% | |

## wasm: ms per pass (median of rounds), and vs M

| workload | MB out | L | M | spread M |
|---|---:|---:|---:|---:|
| explosions-4k-60s.webm.l3.zst | 129.9 | 14.09 (+0.5%) | 14.02 | 1.9% |
| github-dict.frames | 7.5 | 5.48 (-0.5%) | 5.51 | 2.1% |
| github.frames | 7.5 | 22.10 (+0.5%) | 22.00 | 1.4% |
| mixed-2880x1800@2-200M | 491.3 | 529.78 (-2.2%) | 541.49 | 3.1% |
| silesia-nci.l3.zst | 33.6 | 25.34 (+0.3%) | 25.27 | 0.5% |
| silesia-x-ray.l1.zst | 8.5 | 10.15 (-0.7%) | 10.23 | 2.5% |
| silesia.tar.fast4.zst | 212.0 | 174.82 (+0.2%) | 174.54 | 2.6% |
| silesia.tar.l19.zst | 212.0 | 284.28 (-0.7%) | 286.31 | 0.9% |
| silesia.tar.l3.zst | 212.0 | 273.75 (+0.3%) | 272.93 | 186.6% |
| terminal-2880x1800@2 | 3.4 | 0.88 (-0.5%) | 0.89 | 4.2% |
| **geomean vs M** | | -0.3% | +0.0% | |
