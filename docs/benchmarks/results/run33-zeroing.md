# Run 33: B, and B without the zeroing ahead of the writes (B0, an experiment), against main (A)

lab2, unpinned, three rounds, the variants alternating. Output sizes differ: none.

## native: B against A, ms a decode (median of rounds) and B ÷ A

| workload | MB out | 1 thr | 4 thr | 8 thr |
|---|---:|---:|---:|---:|
| silesia-x-ray.l1.zst | 8.5 | 9.87 → 9.09 (0.921) | 5.18 → 5.22 (1.008) | 4.05 → 3.89 (0.960) |
| silesia.tar.fast4.zst | 212.0 | 145.21 → 140.48 (0.967) | 129.51 → 123.12 (0.951) | 94.49 → 92.32 (0.977) |
| silesia.tar.l3.4M-frames.zst | 212.0 | 217.71 → 208.85 (0.959) | 98.26 → 82.33 (0.838) | 47.64 → 49.29 (1.035) |

## native: B0 against A, ms a decode (median of rounds) and B0 ÷ A

| workload | MB out | 1 thr | 4 thr | 8 thr |
|---|---:|---:|---:|---:|
| silesia-x-ray.l1.zst | 8.5 | 9.87 → 9.02 (0.914) | 5.18 → 5.31 (1.025) | 4.05 → 3.80 (0.938) |
| silesia.tar.fast4.zst | 212.0 | 145.21 → 135.38 (0.932) | 129.51 → 117.08 (0.904) | 94.49 → 84.38 (0.893) |
| silesia.tar.l3.4M-frames.zst | 212.0 | 217.71 → 200.48 (0.921) | 98.26 → 82.65 (0.841) | 47.64 → 49.12 (1.031) |

## wasm: B against A, ms a decode (median of rounds) and B ÷ A

| workload | MB out | 1 thr | 4 thr | 8 thr |
|---|---:|---:|---:|---:|
| silesia-x-ray.l1.zst | 8.5 | 11.66 → 10.74 (0.921) | 5.96 → 6.19 (1.039) | 4.66 → 4.94 (1.060) |
| silesia.tar.fast4.zst | 212.0 | 187.37 → 195.60 (1.044) | 147.71 → 138.90 (0.940) | 113.30 → 121.00 (1.068) |
| silesia.tar.l3.4M-frames.zst | 212.0 | 286.20 → 294.06 (1.027) | 103.10 → 111.06 (1.077) | 64.57 → 69.31 (1.073) |

## wasm: B0 against A, ms a decode (median of rounds) and B0 ÷ A

| workload | MB out | 1 thr | 4 thr | 8 thr |
|---|---:|---:|---:|---:|
| silesia-x-ray.l1.zst | 8.5 | 11.66 → 10.72 (0.919) | 5.96 → 6.27 (1.052) | 4.66 → 5.06 (1.086) |
| silesia.tar.fast4.zst | 212.0 | 187.37 → 190.64 (1.017) | 147.71 → 134.44 (0.910) | 113.30 → 117.68 (1.039) |
| silesia.tar.l3.4M-frames.zst | 212.0 | 286.20 → 288.20 (1.007) | 103.10 → 109.79 (1.065) | 64.57 → 75.42 (1.168) |
