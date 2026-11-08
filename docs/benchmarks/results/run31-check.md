outputs differ: none

## native: ms per pass (median of rounds), and vs A

| workload | MB out | A | B | spread A |
|---|---:|---:|---:|---:|
| github-dict.frames | 7.5 | 3.49 | 3.55 (+1.7%) | 1.4% |
| github.frames | 7.5 | 15.33 | 15.23 (-0.7%) | 1.0% |
| silesia-nci.l3.zst | 33.6 | 18.95 | 19.10 (+0.8%) | 4.0% |
| silesia-x-ray.l1.zst | 8.5 | 8.79 | 9.06 (+3.0%) | 1.3% |
| **geomean vs A** | | +0.0% | +1.2% | |

## wasm: ms per pass (median of rounds), and vs A

| workload | MB out | A | B | spread A |
|---|---:|---:|---:|---:|
| github-dict.frames | 7.5 | 5.44 | 5.50 (+1.0%) | 1.5% |
| github.frames | 7.5 | 21.67 | 21.88 (+1.0%) | 1.2% |
| silesia-nci.l3.zst | 33.6 | 25.49 | 25.29 (-0.8%) | 0.5% |
| silesia-x-ray.l1.zst | 8.5 | 10.06 | 10.13 (+0.7%) | 1.1% |
| **geomean vs A** | | +0.0% | +0.5% | |
