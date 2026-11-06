# Where the decoder stands

The latest snapshot of ruzstd-decompress against libzstd: **v0.9.2-alpha.9**, from run 25
of [`README.md`](README.md) (2026-11-06, "the table of pairs' checks in wasm"). The run
measured the decoder as of `57a696d`; the release has the same code, the commits after it
being documentation and the version. The README holds each change's own run; this page is
the state they add up to, and is replaced by the next release's full run.

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
| terminal's tiles | 1.70 | 0.96 | **0.56** | 1.19 | **0.70** |
| the video clip | 18.89 | 15.11 | **0.80** | 15.38 | **0.81** |
| the GitHub frames | 19.43 | 18.52 | **0.95** | 25.18 | 1.30 |
| `x-ray` | 8.87 | 9.73 | 1.10 | 12.76 | 1.44 |
| mixed's tiles | 412.30 | 481.47 | 1.17 | 676.93 | 1.64 |
| the GitHub frames with their dictionary | 3.60 | 4.58 | 1.27 | 8.73 | 2.42 |
| silesia `-19` | 207.96 | 264.46 | 1.27 | 372.46 | 1.79 |
| silesia `-3` | 197.09 | 254.40 | 1.29 | 367.40 | 1.86 |
| `nci` | 18.43 | 23.95 | 1.30 | 31.98 | 1.74 |
| silesia `--fast=4` | 125.18 | 165.38 | 1.32 | 224.79 | 1.80 |
| **geomean** | | | **1.07** | | **1.46** |

How far the rounds were apart (largest less smallest, over the median): libzstd 0.5–4.2%,
but terminal's tiles 11.8%; ruzstd-decompress natively 0.3–5.7%, but the video clip 12.1%
and the dictionary frames 9.9%; in wasm 0.2–5.1%, but `x-ray` 18.3%. A wasm process runs
in a faster or a slower mode for all of its passes, so a wasm figure moves more from one
run to the next than its rounds show here. Every build, every workload:
[`results/run25-tail.md`](results/run25-tail.md), the raw lines in
[`results/run25-tail.jsonl`](results/run25-tail.jsonl).

## What it says

- **Ahead of libzstd natively**: terminal's tiles (0.56), the video clip's raw blocks (0.80)
  and the GitHub frames (0.95); the first two in wasm too.
- **Behind most on Silesia**, 1.27–1.32 times natively. Nearly all of it is the sequences'
  decoding and execution, and it is the instruction count, not cache misses: on silesia
  `-3`, 3,600 M instructions a pass (lab2, 2026-11-06) to libzstd's 2,408 M (run 22). That
  is item 1 of the [roadmap](../roadmap.md).
- **The Huffman literals** (`x-ray`, mixed's tiles) are 1.10–1.17 times natively. The four
  streams' loop waits on each stream's chain from one lookup to the next; libzstd's
  assembly loop takes about 9 instructions a lookup to ruzstd-decompress's 15.8 (run 22);
  a pass on `x-ray` has run 3.6% fewer since run 25.
- **In wasm** the workloads take 1.24–1.44 times their native time, and the dictionary
  frames 1.91, where the fast copy takes no match into the dictionary; the video clip's raw
  blocks take the same. zDesk's tiles in wasm: mixed's 1.64 times native libzstd's time,
  terminal's 0.70.
- **Since the last snapshot** (run 24: 1.06 by geomean natively) the native ratios are as
  they were, within the runs' spread; run 25's change was for wasm, and is measured over
  16 rounds in its README section.

## Running it again

`harness/run.sh results/runN-NAME.jsonl VARIANT Z` on lab2 after a restart, a variant being
`main`'s tree built by `harness/build.sh`, then
`uv run --python 3.13 python -I harness/summary.py results/runN-NAME.jsonl VARIANT`; the
README's "Running it again" has the layout and the corpora. Then this page, from the run's
`.jsonl`.
