# Where the decoder stands

The latest full benchmark of ruzstd-decompress against libzstd: run 34 of
[`README.md`](README.md) (2026-11-08, "outputs held to their room, frames to their content
size"), B34, the change that holds every frame to its header's content size and adds
`decode_each_parallel`, on `main`'s v0.9.2-alpha.14. lab2 was rebuilt for run 34, back on an
EPYC-Rome, so its figures are not comparable with earlier runs'. The README holds each
change's own run; this page is the state they add up to, and follows the latest full run.

## How it was measured

- **Machine**: lab2, a VM with an `AMD EPYC-Rome Processor` (AVX2, BMI2), 8 vCPUs (4 cores,
  2 threads), 15 GB, Debian 13 (kernel 6.12.116), swap off, restarted before the run,
  nothing else running; every run pinned to vCPU 2.
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
  the median of those. Every build decoded every workload to the same output, run 29's.

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

The corpora are made by [`harness/make-corpus.sh`](harness/make-corpus.sh), the same bytes on
any machine (checked against [`harness/corpus.sha256`](harness/corpus.sha256)); the tiles, and
the rest: "Running it again" in the README.

## Against libzstd

ms a pass. Natively, the ratio is ruzstd-decompress's time over libzstd's, so above 1 is
slower than libzstd. libzstd is not built for wasm here, so the wasm build is not compared
with it: its ratio is its time over ruzstd-decompress's own native time, what WebAssembly
costs.

| workload | libzstd | native | native ÷ libzstd | wasm | wasm ÷ native |
|---|---:|---:|---:|---:|---:|
| terminal's tiles | 1.71 | 0.84 | **0.49** | 1.10 | 1.32 |
| the video clip | 19.52 | 15.46 | **0.79** | 15.63 | 1.01 |
| the GitHub frames | 20.35 | 18.67 | **0.92** | 25.53 | 1.37 |
| `x-ray` | 9.12 | 9.90 | 1.09 | 11.56 | 1.17 |
| silesia `-19` | 211.89 | 233.41 | 1.10 | 322.03 | 1.38 |
| `nci` | 19.10 | 21.57 | 1.13 | 28.60 | 1.33 |
| silesia `--fast=4` | 130.49 | 147.19 | 1.13 | 197.54 | 1.34 |
| mixed's tiles | 425.44 | 479.86 | 1.13 | 619.64 | 1.29 |
| silesia `-3` | 206.53 | 236.82 | 1.15 | 328.76 | 1.39 |
| the GitHub frames with their dictionary | 3.77 | 4.38 | 1.16 | 6.92 | 1.58 |
| **geomean** | | | **0.98** | | **1.31** |

How far the rounds were apart (largest less smallest, over the median): libzstd 0.2–2.8%,
but the video clip 11.9%; ruzstd-decompress natively 0.5–1.9%, but silesia `-3` 5.6% and the
video clip 9.9%; in wasm 0.3–2.2%, but the video clip 6.1% and silesia `-3` 9.5%. silesia
`-3` again, five rounds alone ([`results/run34-l3.md`](results/run34-l3.md)): 232.64 ms
natively, 1.13 times libzstd's time, and 320.82 in wasm, 1.38 times the native time.
A wasm process runs in a faster or a slower mode for all of its passes, so a wasm figure
moves more from one run to the next than its rounds show.
Every build, every workload: [`results/run34-lab2-rebuilt.md`](results/run34-lab2-rebuilt.md),
the raw lines in the `.jsonl` beside it.

## What it says

- **Natively**, ruzstd-decompress takes 0.98 times libzstd's time by geomean: the
  sequences' and the Huffman streams' loops run by raw pointers, as libzstd's, in a BMI2
  build where the processor has it (run 26). On the EPYC-Milan of runs 29–33 it was 0.99;
  back on an EPYC-Rome, libzstd lost more on mixed's tiles (1.23 to 1.13) and `x-ray` (1.22
  to 1.09), and ruzstd-decompress more on the video clip (0.70 to 0.79) and the dictionary
  frames (1.11 to 1.16).
- **Ahead of libzstd natively**: terminal's tiles (0.49), the video clip's raw blocks (0.79)
  and the GitHub frames (0.92).
- **Behind most on the GitHub frames with their dictionary** (1.16) and silesia `-3` (1.13
  to 1.15 by run), then the rest of Silesia and mixed's tiles, 1.09–1.13. The sequences'
  loop runs as many instructions a cycle as libzstd's, with as many misses and
  mispredictions; it trails by about 7 cycles a sequence, with about 10 more stores of its
  spills (item 1 of the [roadmap](../roadmap.md)).
- **In wasm**, which runs the same raw loops since run 27, the workloads take 1.01–1.58
  times their native time, 1.31 by geomean; the dictionary frames most (1.58), the video
  clip's raw blocks about the same.
- **The content size checked** (run 34) cost nothing measurable on one thread: by geomean
  0.996 times v0.9.2-alpha.14's time natively and 0.995 in wasm.

## On several threads

`FrameDecoder::decode_all_parallel` (the `rayon` feature) decodes on a pool's threads, in
wasm on a pool of workers (`examples/wasm-threads`), to the output's spare capacity;
`decode_each_parallel` decodes many inputs side by side, each to its own output. Run 34 of
the [README](README.md) (lab2, 8 threads against 1):

- **Inputs of many frames, without a dictionary,** got 4.4–5.1 times as fast natively and in
  wasm: Silesia in frames of 4 MiB, and the GitHub frames.
- **The GitHub frames with their dictionary** got 2.5× natively and 2.9× in wasm.
- **One frame** got 1.5–2.8 times as fast natively, 1.7–2.8 in wasm: `x-ray` most, the
  Silesia tars 1.6–1.8× natively and 1.8–2.0× in wasm. Its sequences are executed on one
  thread, which is about half of a decode.

Against v0.9.2-alpha.14 the large frames are as before; the small frames move by their noise
on several threads, B34 1.5–5% behind in wasm on the GitHub frames, about what the checks
cost on one thread (run 34).

## Running it again

On any machine: "On any machine", under "Running it again" in the [README](README.md), which makes
the corpora, builds a commit and libzstd, and runs them, with
[`harness/env.sh`](harness/env.sh) for the settings; without the maintainer's files it runs 8
of the 10 workloads, all but the video clip and the tiles. A figure compares only within its
run, on its machine. This page follows the latest full run on lab2:
`harness/run.sh results/runN-NAME.jsonl VARIANT Z` after a restart, then
`harness/summary.py results/runN-NAME.jsonl VARIANT`.
