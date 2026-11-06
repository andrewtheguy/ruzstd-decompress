# Where the decoder stands

The latest full benchmark of ruzstd-decompress against libzstd: run 28 of
[`README.md`](README.md) (2026-11-06, "the sequences in V8, on the dictionary frames"), B28,
the branch `seq-one-table` at `bd3b6d9`. The README holds each change's own run; this page is the state they add up
to, and is replaced by the next full run.

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

ms a pass. Natively, the ratio is ruzstd-decompress's time over libzstd's, so above 1 is
slower than libzstd. libzstd is not built for wasm here, so the wasm build is not compared
with it: its ratio is its time over ruzstd-decompress's own native time, what WebAssembly
costs.

| workload | libzstd | native | native ÷ libzstd | wasm | wasm ÷ native |
|---|---:|---:|---:|---:|---:|
| terminal's tiles | 1.65 | 0.79 | **0.48** | 1.05 | 1.33 |
| the video clip | 19.97 | 15.04 | **0.75** | 15.30 | 1.02 |
| the GitHub frames | 19.45 | 18.43 | **0.95** | 24.52 | 1.33 |
| `x-ray` | 8.68 | 9.45 | 1.09 | 11.16 | 1.18 |
| the GitHub frames with their dictionary | 3.69 | 4.03 | 1.09 | 6.56 | 1.63 |
| silesia `-19` | 205.09 | 225.76 | 1.10 | 341.31 | 1.51 |
| mixed's tiles | 411.06 | 456.85 | 1.11 | 582.75 | 1.28 |
| `nci` | 18.38 | 20.66 | 1.12 | 28.15 | 1.36 |
| silesia `--fast=4` | 124.61 | 141.54 | 1.14 | 190.59 | 1.35 |
| silesia `-3` | 203.59 | 232.99 | 1.14 | 319.64 | 1.37 |
| **geomean** | | | **0.97** | | **1.33** |

How far the rounds were apart (largest less smallest, over the median): libzstd 0.8–5.7%,
but terminal's tiles 10.2%; ruzstd-decompress natively 1.4–6.7%, but the video clip 10.1%;
in wasm 1.4–7.6%, but the video clip 14.9%. A wasm process runs in a faster or a slower mode
for all of its passes, so a wasm figure moves more from one run to the next than its rounds
show: silesia `-19` took 1.38 times its native time in run 27 and 1.51 here, where its wasm
figure is 3.5% over M28's (`main`), within M28's 5.6% spread.
Every build, every workload: [`results/run28-dict-copy.md`](results/run28-dict-copy.md), the
raw lines in the `.jsonl` beside it.

## What it says

- **Natively**, ruzstd-decompress takes 0.97 times libzstd's time by geomean (0.96 in run
  27, the same within the rounds' spread): the
  sequences' and the Huffman streams' loops run by raw pointers, as libzstd's, in a BMI2
  build where the processor has it (run 26).
- **Ahead of libzstd natively**: terminal's tiles (0.48), the video clip's raw blocks (0.75)
  and the GitHub frames (0.95).
- **Behind most on Silesia**, 1.09–1.14 times natively, `x-ray` included, the GitHub frames
  with their dictionary 1.09 and mixed's tiles 1.11. The sequences' loop runs as many instructions a cycle
  as libzstd's, with as many misses and mispredictions; it trails by about 7 cycles a
  sequence, with about 10 more stores of its spills (item 1 of the [roadmap](../roadmap.md)).
- **In wasm**, which runs the same raw loops since run 27 (10.8% faster than before by
  geomean), the workloads take 1.18–1.63 times their native time, 1.33 by geomean (1.50 in
  run 26); the dictionary frames most, 1.71 in run 27 before their matches from the
  dictionary were copied inline (run 28, 10.3% faster in wasm), the video clip's raw blocks
  about the same.

## Running it again

`harness/run.sh results/runN-NAME.jsonl VARIANT Z` on lab2 after a restart, a variant being
a commit's tree built by `harness/build.sh`, then
`uv run --python 3.13 python -I harness/summary.py results/runN-NAME.jsonl VARIANT`; the
README's "Running it again" has the layout and the corpora. Then this page, from the run's
`.jsonl`, with every full run.
