# Run 34 on several threads: B ÷ M

threads.sh from each variant's tree, M, B, M, B, after a restart; each cell the better of two rounds. Made by a script of this run (not in the harness).

## native: B ÷ M, ms a decode (the better of two rounds each)

| workload | 1 thr | 2 thr | 3 thr | 4 thr | 6 thr | 8 thr |
|---|---:|---:|---:|---:|---:|---:|
| github-dict.frames | 1.012 | 0.854 | 0.942 | 1.099 | 1.134 | 1.089 |
| github.frames | 1.009 | 0.963 | 0.982 | 1.048 | 1.056 | 1.081 |
| silesia-nci.l3.zst | 0.983 | 1.037 | 1.092 | 0.996 | 0.991 | 0.981 |
| silesia-x-ray.l1.zst | 1.000 | 0.996 | 0.973 | 0.998 | 0.979 | 1.009 |
| silesia.tar.fast4.zst | 0.987 | 1.010 | 1.008 | 0.977 | 1.037 | 1.039 |
| silesia.tar.l19.zst | 1.005 | 1.046 | 0.969 | 1.042 | 1.034 | 1.041 |
| silesia.tar.l3.4M-frames.zst | 0.989 | 1.005 | 0.994 | 0.973 | 0.957 | 0.963 |
| silesia.tar.l3.zst | 0.993 | 1.023 | 1.073 | 0.986 | 1.055 | 1.055 |

geomean B ÷ M: 1.011
M's two rounds apart: median 1.8%, most 17.6%

## wasm: B ÷ M, ms a decode (the better of two rounds each)

| workload | 1 thr | 2 thr | 3 thr | 4 thr | 6 thr | 8 thr |
|---|---:|---:|---:|---:|---:|---:|
| github-dict.frames | 1.032 | 1.057 | 1.115 | 1.104 | 0.993 | 0.967 |
| github.frames | 1.008 | 1.038 | 1.062 | 1.026 | 1.144 | 1.052 |
| silesia-nci.l3.zst | 1.002 | 0.983 | 0.984 | 0.972 | 0.956 | 0.960 |
| silesia-x-ray.l1.zst | 1.008 | 0.986 | 1.006 | 1.007 | 1.006 | 0.996 |
| silesia.tar.fast4.zst | 1.008 | 0.996 | 1.054 | 0.987 | 0.979 | 0.984 |
| silesia.tar.l19.zst | 1.007 | 0.993 | 1.035 | 0.959 | 0.891 | 0.952 |
| silesia.tar.l3.4M-frames.zst | 1.012 | 0.880 | 0.873 | 0.873 | 0.896 | 0.913 |
| silesia.tar.l3.zst | 1.012 | 1.005 | 0.832 | 0.932 | 0.938 | 0.964 |

geomean B ÷ M: 0.986
M's two rounds apart: median 1.4%, most 10.8%
