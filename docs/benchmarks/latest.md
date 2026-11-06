# Where the decoder stands

The latest full benchmark of ruzstd-decompress against libzstd: run 26 of
[`README.md`](README.md) (2026-11-06, "unsafe code in the native loops"), of the branch
`unsafe-fast` at `dc41c17`: natively R26, the branch at `e56ebae`, whose native code is the
same; in wasm R27, at `dc41c17`, measured after the run on the same machine. The README holds
each change's own run; this page is the state they add up to, and is replaced by the next
full run.

## How it was measured

- **Machine**: lab2, a VM with an `AMD EPYC-Rome Processor`, 4 vCPUs (2 cores, 2 threads),
  8 GB, Debian 13 (kernel 6.12.115), swap off, restarted before the run, nothing else
  running; every run pinned to vCPU 2.
- **ruzstd-decompress**: rustc 1.99.1 (stable), the profile of zDesk's egfx crate
  (`opt-level = 3`, `lto = true`, `codegen-units = 1`, `panic = "abort"`).
  - Native: `x86_64-unknown-linux-gnu`, the default CPU (SSE2).
  - wasm: `wasm32-wasip1` with `+simd128,+bulk-memory`, then `wasm-opt -O3` (binaryen 120)
    with zDesk's flags, run by Node 26.13.0 (V8 14.6) through `node:wasi`.
- **libzstd**: 1.5.7, bundled by the `zstd` crate 0.14, native only (its C is not built for
  wasm here), through a harness of the same shape
  ([`harness/libzstd/`](harness/libzstd)).
- **A figure**: a pass over the workload, timed in the process and repeated for 2 s and at
  least 3 times, gives its median; three rounds, the builds in a new random order each, give
  the median of those. Every build decoded every workload to the same output.

## The workloads

| workload | in | out | mode | what it puts the decoder through |
|---|---:|---:|---|---|
| silesia `-3` | 66.2 MB | 212.0 MB | frame | 128 KB blocks, FSE tables, most of the output from matches |
| silesia `-19` | 52.8 MB | 212.0 MB | frame | the same, with repeated tables |
| silesia `--fast=4` | 98.8 MB | 212.0 MB | frame | raw literals |
| Silesia's `x-ray`, `-1` | 6.8 MB | 8.5 MB | frame | nearly all Huffman literals, four streams |
| Silesia's `nci`, `-3` | 2.8 MB | 33.6 MB | frame | the longest matches in Silesia |
| the GitHub users sample set, `-3` | 2.5 MB | 7.5 MB | frames | 9,114 frames of 0.8 KB: one-stream Huffman, predefined tables |
| the same, with its dictionary | 0.7 MB | 7.5 MB | frames | the dictionary's tables, matches into the dictionary |
| the video clip, `-3` | 129.9 MB | 129.9 MB | frame | raw blocks, already compressed |
| wlshare's mixed tiles 2880x1800@2 (200 MB of bodies) | 199.8 MB | 491.3 MB | tiles | zDesk's planar frames, mostly Huffman literals, and its still stream |
| wlshare's terminal tiles 2880x1800@2 | 0.2 MB | 3.4 MB | tiles | the still stream: small flushed blocks, predefined tables |

- **frame**: one frame decoded whole, by a new decoder each pass, its checksum checked.
- **frames**: each small frame read whole into one reused buffer, by one decoder reset for
  each, with the dictionary when there is one.
- **tiles**: replayed as zDesk's page decodes them: a planar body by a `StreamingDecoder`
  read to its end, the still stream by one `FrameDecoder` a block at a time
  (`decode_from_to`), both decoders kept for all the passes, as zDesk keeps them.

How the corpora and tiles are made: "Running it again" in the README.

## Against libzstd

ms a pass; the ratio is ruzstd-decompress's time over native libzstd's, so above 1 is
slower than libzstd. In wasm the same comparison is with native libzstd, the cost of
WebAssembly included.

| workload | libzstd | native | ÷ libzstd | wasm | ÷ libzstd |
|---|---:|---:|---:|---:|---:|
| terminal's tiles | 1.65 | 0.79 | **0.48** | 1.22 | **0.74** |
| the video clip | 20.30 | 15.38 | **0.76** | 15.17 | **0.75** |
| the GitHub frames | 19.51 | 17.49 | **0.90** | 25.37 | 1.30 |
| `x-ray` | 8.75 | 9.62 | 1.10 | 12.10 | 1.38 |
| silesia `--fast=4` | 127.34 | 140.50 | 1.10 | 225.68 | 1.77 |
| the GitHub frames with their dictionary | 3.63 | 4.06 | 1.12 | 8.69 | 2.39 |
| silesia `-19` | 203.74 | 228.26 | 1.12 | 376.36 | 1.85 |
| silesia `-3` | 199.74 | 224.07 | 1.12 | 367.09 | 1.84 |
| mixed's tiles | 413.15 | 471.39 | 1.14 | 665.75 | 1.61 |
| `nci` | 18.38 | 21.25 | 1.16 | 32.53 | 1.77 |
| **geomean** | | | **0.97** | | **1.45** |

How far the rounds were apart (largest less smallest, over the median): libzstd 0.6–5.8%;
ruzstd-decompress natively 0.6–5.0%, but the video clip 13.4%; in wasm 1.3–5.7%, but the
video clip 8.8% and `x-ray` 9.1%. A wasm process runs in a faster or a slower mode for all of
its passes, so a wasm figure moves more from one run to the next than its rounds show. Every
build, every workload: [`results/run26-raw.md`](results/run26-raw.md) (natively, and wasm
before `dc41c17`) and [`results/run26-wasm-r27.md`](results/run26-wasm-r27.md) (wasm), the
raw lines in the `.jsonl` beside them.

## What it says

- **Natively**, ruzstd-decompress takes 0.97 times libzstd's time by geomean (1.07 in run
  25): the sequences' and the Huffman streams' loops run by raw pointers, as libzstd's, in a
  BMI2 build where the processor has it.
- **Ahead of libzstd natively**: terminal's tiles (0.48), the video clip's raw blocks (0.76)
  and the GitHub frames (0.90); the first two in wasm too.
- **Behind most on Silesia**, 1.10–1.16 times natively, `x-ray` included, and the GitHub
  frames with their dictionary 1.12. The sequences' loop runs as many instructions a cycle
  as libzstd's, with as many misses and mispredictions; it trails by about 7 cycles a
  sequence, with about 10 more stores of its spills (item 1 of the [roadmap](../roadmap.md)).
- **In wasm**, which keeps the safe loops of run 25, the workloads take 1.26–1.65 times
  their native time, the dictionary frames 2.14, the video clip's raw blocks the same; 1.45
  times native libzstd's by geomean. The raw loops in wasm are not yet tried.

## Running it again

`harness/run.sh results/runN-NAME.jsonl VARIANT Z` on lab2 after a restart, a variant being
a commit's tree built by `harness/build.sh`, then
`uv run --python 3.13 python -I harness/summary.py results/runN-NAME.jsonl VARIANT`; the
README's "Running it again" has the layout and the corpora. Then this page, from the run's
`.jsonl`, with every full run.
