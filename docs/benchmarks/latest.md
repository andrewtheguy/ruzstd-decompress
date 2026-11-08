# Where the decoder stands

The latest full benchmark of ruzstd-decompress against libzstd: run 29 of
[`README.md`](README.md) (2026-11-07, "a new baseline, on a rebuilt lab2 and reproducible
corpora"), M29, `main` at `ff75fa7` (v0.9.2-alpha.12). lab2 has a new processor since run
29, so its figures are not comparable with earlier runs'. The README holds each change's own run; this page is the state they add up
to, and is replaced by the next full run.

## How it was measured

- **Machine**: lab2, a VM with an `AMD EPYC-Milan Processor` (AVX2, BMI2), 8 vCPUs (4 cores,
  2 threads), 15 GB, Debian 13 (kernel 6.12.115), swap off, restarted before the run,
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
| terminal's tiles | 1.36 | 0.66 | **0.49** | 0.88 | 1.33 |
| the video clip | 19.47 | 13.69 | **0.70** | 13.87 | 1.01 |
| the GitHub frames | 16.48 | 15.28 | **0.93** | 21.69 | 1.42 |
| the GitHub frames with their dictionary | 3.14 | 3.49 | 1.11 | 5.43 | 1.55 |
| silesia `-19` | 187.56 | 208.97 | 1.11 | 278.37 | 1.33 |
| `nci` | 17.09 | 19.27 | 1.13 | 25.30 | 1.31 |
| silesia `--fast=4` | 116.25 | 132.81 | 1.14 | 173.25 | 1.30 |
| silesia `-3` | 176.50 | 203.09 | 1.15 | 276.64 | 1.36 |
| `x-ray` | 7.23 | 8.79 | 1.22 | 9.99 | 1.14 |
| mixed's tiles | 345.54 | 426.10 | 1.23 | 524.85 | 1.23 |
| **geomean** | | | **0.99** | | **1.29** |

How far the rounds were apart (largest less smallest, over the median): libzstd 0.4–1.6%,
but `x-ray` 19.9% (6.84, 7.23 and 8.28 ms: its ratio is 1.06–1.29 by round);
ruzstd-decompress natively 0.3–3.1%, but the dictionary frames 6.5%; in wasm 0.9–4.3%.
A wasm process runs in a faster or a slower mode for all of its passes, so a wasm figure
moves more from one run to the next than its rounds show.
Every build, every workload: [`results/run29-lab2-rebuilt.md`](results/run29-lab2-rebuilt.md),
the raw lines in the `.jsonl` beside it.

## What it says

- **Natively**, ruzstd-decompress takes 0.99 times libzstd's time by geomean: the
  sequences' and the Huffman streams' loops run by raw pointers, as libzstd's, in a BMI2
  build where the processor has it (run 26). On the old lab2's EPYC-Rome it was 0.97 (run
  28, the same code); the new processor sped libzstd up more on mixed's tiles and `x-ray`,
  ruzstd-decompress more on the video clip.
- **Ahead of libzstd natively**: terminal's tiles (0.49), the video clip's raw blocks (0.70)
  and the GitHub frames (0.93).
- **Behind most on mixed's tiles** (1.23) and `x-ray` (1.22, less surely), then Silesia,
  1.11–1.15 times, and the GitHub frames with their dictionary, 1.11. The sequences' loop
  runs as many instructions a cycle as libzstd's, with as many misses and mispredictions; it
  trails by about 7 cycles a sequence, with about 10 more stores of its spills (item 1 of
  the [roadmap](../roadmap.md); measured on the EPYC-Rome).
- **In wasm**, which runs the same raw loops since run 27, the workloads take 1.01–1.55
  times their native time, 1.29 by geomean; the dictionary frames most (1.55), the GitHub
  frames next (1.42), the video clip's raw blocks about the same.

## On several threads

`FrameDecoder::decode_all_parallel` (the `rayon` feature) decodes on a pool's threads, in
wasm on a pool of workers (`examples/wasm-threads`). Run 32 of the [README](README.md)
(2026-11-08, lab2, 8 threads against 1):

- **Inputs of many frames, without a dictionary,** got 4.5–5.0 times as fast natively and in
  wasm: Silesia in frames of 4 MiB, and the GitHub frames.
- **The GitHub frames with their dictionary** got 1.9× natively and 2.5× in wasm.
- **One frame** got 1.4–2.5 times as fast: `x-ray` most, the Silesia tars 1.5–1.7×. Its
  sequences are executed on one thread, which is about half of a decode.

On one thread it decodes as before (run 30, geomean +0.4% natively and +0.2% in wasm).

## Running it again

On any machine: "On any machine", under "Running it again" in the [README](README.md), which makes
the corpora, builds a commit and libzstd, and runs them, with
[`harness/env.sh`](harness/env.sh) for the settings; without the maintainer's files it runs 8
of the 10 workloads, all but the video clip and the tiles. A figure compares only within its
run, on its machine. This page follows the latest full run on lab2:
`harness/run.sh results/runN-NAME.jsonl VARIANT Z` after a restart, then
`harness/summary.py results/runN-NAME.jsonl VARIANT`.
