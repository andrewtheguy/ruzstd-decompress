outputs differ: none

## native: ms per pass (median of rounds), and vs A

| workload | MB out | A | C | spread A |
|---|---:|---:|---:|---:|
| github-dict.frames | 7.5 | 3.48 | 3.54 (+1.6%) | 2.4% |
| github.frames | 7.5 | 15.24 | 15.32 (+0.5%) | 1.0% |
| silesia-nci.l3.zst | 33.6 | 18.94 | 19.12 (+0.9%) | 5.5% |
| silesia-x-ray.l1.zst | 8.5 | 8.80 | 9.05 (+2.8%) | 38.5% |
| **geomean vs A** | | +0.0% | +1.5% | |

x-ray's spread for A is one round, round 3 at 12.17 ms; A's other four took 8.78–8.84 ms, C's
9.01–9.09 ms, so its median and +2.8% do not hang on that round.

## wasm: ms per pass (median of rounds), and vs A

| workload | MB out | A | C | spread A |
|---|---:|---:|---:|---:|
| github-dict.frames | 7.5 | 5.41 | 5.56 (+2.9%) | 2.1% |
| github.frames | 7.5 | 21.77 | 21.86 (+0.4%) | 1.7% |
| silesia-nci.l3.zst | 33.6 | 25.46 | 25.30 (-0.6%) | 8.6% |
| silesia-x-ray.l1.zst | 8.5 | 10.05 | 10.14 (+0.9%) | 3.0% |
| **geomean vs A** | | +0.0% | +0.9% | |
