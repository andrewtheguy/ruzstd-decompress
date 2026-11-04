# Decode benchmarks

What the decoder costs on the data it is used for, and what each change to it
bought. The crate's main user is zDesk's page, which decodes wlshare's lossless
tiles in WebAssembly (`zdesk/frontend/wasm/egfx`); wlshare decodes the same
tiles natively in its tests and examples. The general corpora are measured too,
for use outside those.

## 2026-11-04: the safe ring buffer

The question: do the unsafe optimizations (the custom ring buffer and its
overshooting copy in `src/decoding/ringbuffer.rs`, `repeat`'s unchecked copy)
still pay against safe code, natively and in wasm?

### Variants

All from `master` at `119c1b9` (the decoder-only crate):

| | |
|---|---|
| **A** | `master` as it was: the unsafe ring buffer, copying in 16-byte chunks on SSE2 and NEON and in `usize` chunks elsewhere, wasm included |
| **B** | a safe ring buffer over a fully initialised `Vec<u8>`, copying in 16-byte `copy_within` chunks where there is room; no unsafe code ([`B-from-D.patch`](variants/B-from-D.patch) applied to D) |
| **C** | A with the 16-byte chunks on `simd128` too ([`C.patch`](variants/C.patch)) |
| **D** | B, and `repeat_in_chunks` copying an overlapping match in doubling runs instead of offset-sized pieces: what was committed, in `80d66ba` |
| **E** | A with D's doubling ([`E.patch`](variants/E.patch)) |
| **Z** | libzstd 1.5.7 (`zstd` crate 0.14, `zstd-sys 2.1.2+zstd.1.5.7`), native only: the reference |

### Workloads

- **wlshare's lossless tiles**: every planar and still body wlshare's
  motion-bench sends, in order, for the 120-frame samples of browser, drag,
  still, terminal, uncover, video and mixed at 1920x1080 and 2880x1800@2, with
  `--lossless --tiles-only --libx264`. They are replayed as zDesk's
  `WlshareTiles::planar` and `::still` decode them: a planar body by a
  `StreamingDecoder` read to the end, the still stream by one `FrameDecoder` a
  block at a time through `decode_from_to`, its newest bytes read with `peek`.
  The mixed sessions are cut to their first 200 MB of bodies. Flood is left out:
  it is a reference, not something a remote desktop runs. The held picture
  (without `--lossless`) sent no lossless bodies in 120 frames, so it has none.
- **General data**: `enwik9` and `silesia.tar`, each compressed by the `zstd`
  CLI at `-3` and `-19`, decoded whole a megabyte of blocks at a time to a sink.

Every result carries a hash of the output (an FNV hash of every tile's bitmap,
or the frame's checksum checked); every variant, libzstd included, decoded every
workload to the same output.

### Machine and build

- lab1: a Vultr VM, `AMD EPYC-Rome Processor`, 8 vCPUs (4 cores, 2 threads), 16
  GB, Debian 13, kernel 6.12.115, swap off, nothing else running; each run
  pinned to one vCPU (`taskset -c 2`).
- rustc `1.101.0-nightly (92b5b8c87 2026-10-29)` (`nightly-2026-10-31`, the pin
  of the time); the profile of zDesk's egfx crate: `opt-level = 3`, `lto =
  true`, `codegen-units = 1`, `panic = "abort"`.
- Native: `x86_64-unknown-linux-gnu`, the default CPU (SSE2).
- wasm: `wasm32-wasip1` with `+simd128,+bulk-memory`, then `wasm-opt -O3`
  (binaryen 120) with zDesk's flags, run by Node 26.13.0 (V8 14.6) through
  `node:wasi`. Not zDesk's `+atomics` shared memory: the decoder runs on one
  thread either way.
- A pass is timed in process, repeated until 2 s and at least 3 times; the
  figure is its median. Three rounds, the variants in a new random order each
  round; the tables give the median of the rounds' medians. "spread A" is how
  far A's rounds were apart.

### Results

Geomean over the 18 workloads, against A (lower is faster):

| | B | C | D | E |
|---|---:|---:|---:|---:|
| native | +0.1% | −0.6% | **−1.0%** | +1.5% |
| wasm | −5.8% | −5.0% | **−6.6%** | −1.6% |

The heaviest workloads, ms a pass:

| workload | MB out | native A | native D | wasm A | wasm D | libzstd |
|---|---:|---:|---:|---:|---:|---:|
| video 1920x1080 | 246.5 | 1181 | 1167 (−1.2%) | 1460 | 1441 (−1.3%) | 189 |
| video 2880x1800@2 | 686.0 | 3290 | 3232 (−1.8%) | 4094 | 4051 (−1.1%) | 603 |
| mixed 1920x1080, 200 MB | 383.9 | 1598 | 1539 (−3.7%) | 1959 | 1950 (−0.4%) | 303 |
| mixed 2880x1800@2, 200 MB | 491.3 | 1914 | 1866 (−2.5%) | 2434 | 2316 (−4.8%) | 448 |
| enwik9 `-3` | 1000.0 | 3465 | 3352 (−3.3%) | 5038 | 4279 (−15.1%) | 1127 |
| enwik9 `-19` | 1000.0 | 2553 | 2484 (−2.7%) | 3854 | 3143 (−18.4%) | 1112 |
| silesia `-3` | 212.0 | 659 | 639 (−2.9%) | 918 | 800 (−12.8%) | 215 |
| silesia `-19` | 212.0 | 599 | 567 (−5.4%) | 839 | 710 (−15.4%) | 216 |

Every workload, every variant: [`results/run1.md`](results/run1.md); libzstd
against A, timed beside it: [`results/run2-libzstd.md`](results/run2-libzstd.md).
The raw lines are the `.jsonl` beside them.

### What it showed

- Natively the unsafe ring buffer bought nothing: the safe one is within the
  rounds' spread of it (B +0.1%).
- In wasm the safe one is faster, 13–19% on the general corpora: the compiler's
  `copy_within` beats the hand-written copy, whose wasm path copied 4 bytes at a
  time. Giving that path 16-byte chunks (C) recovers only part of it.
- On wlshare's tiles the gain is smaller (0–5% on video and mixed, up to 12% on
  the small ones), as their time goes elsewhere.
- The doubling repeat adds about a point on top of the safe ring buffer (D
  against B), but slows the unsafe one natively (E).
- libzstd is 2.6 times as fast natively over all (geomean −62%), and 5–6 times
  on the video and mixed tiles, which are mostly planar frames of about 75 KB
  each: the gap is the decoder's throughput, not its start-up per tile.
- libzstd's harness zero-fills the most a bitmap can be for every still body,
  which ruzstd's does not: its figures for the small still-heavy samples at
  2880x1800@2 (still, terminal, drag) are pessimistic.

## 2026-11-04: the `peek-perf` decoder commits

The question: what do the safe decoder commits of the `peek-perf` branch buy
here? They are Peter Knego's, written against the old `ruzstd/` layout and
moved onto `master`:

- inlining `FSEDecoder::update_state` and `DecodeBuffer::repeat`;
- executing each sequence as it is decoded, with no `Vec` of sequences;
- Huffman literals decoded with one table lookup a symbol, the four streams
  interleaved in one loop;
- the reverse bit reader's refill inlined, its end-of-source case cold.

The branch's overshooting literal copy (unsafe, and reverted on the branch
itself) is left out, as is its doubling repeat, which `master` has had since
D.

| | |
|---|---|
| **M** | `master` at `7bcbd58` (D, as merged) |
| **P** | M with the four commits |
| **Z** | libzstd, as before |

The same workloads, machine, profile and method as above, but on rustc
`1.99.1 (58cf8f9c3 2026-10-13)`, stable, for both: the repository no longer
pins a toolchain. Every variant decoded every workload to the same output.

Geomean over the 18 workloads, against M:

| | P | libzstd |
|---|---:|---:|
| native | **−31.9%** | −62.4% |
| wasm | **−27.4%** | |

The heaviest workloads, ms a pass:

| workload | native M | native P | wasm M | wasm P | libzstd |
|---|---:|---:|---:|---:|---:|
| video 1920x1080 | 1252 | 471 (−62.4%) | 1434 | 588 (−59.0%) | 188 |
| video 2880x1800@2 | 3521 | 1492 (−57.6%) | 4087 | 1963 (−52.0%) | 602 |
| mixed 1920x1080, 200 MB | 1668 | 688 (−58.8%) | 1946 | 869 (−55.3%) | 305 |
| mixed 2880x1800@2, 200 MB | 1995 | 970 (−51.4%) | 2363 | 1262 (−46.6%) | 446 |
| enwik9 `-3` | 3357 | 2623 (−21.9%) | 4227 | 3653 (−13.6%) | 1130 |
| enwik9 `-19` | 2504 | 2218 (−11.4%) | 3229 | 2938 (−9.0%) | 1099 |
| silesia `-3` | 637 | 481 (−24.5%) | 799 | 657 (−17.7%) | 213 |
| silesia `-19` | 570 | 477 (−16.3%) | 716 | 672 (−6.2%) | 219 |

Every workload: [`results/run3-peek-perf.md`](results/run3-peek-perf.md)
(`summary.py results/run3-peek-perf.jsonl M`).

- The video and mixed tiles, where the time goes in zDesk, decode in 38–49% of
  M's time natively and 41–53% in wasm. The general corpora gain 6–25%.
- libzstd's lead over ruzstd falls from 2.6 times to 1.8 times natively over
  all, and on the video and mixed tiles from 4.5–6.7 times to 2.2–2.5 times.
- `decode` and `interop` fuzzing, four minutes each, found nothing.

## Running it again

The scripts assume `../devtools`' layout, everything under
`~/codes/staging-area`; `zstd-rs-tuned-scratch/` holds the corpora, variants
and results.

1. **The tiles** (on any machine; the bytes do not hang on the processor): an
   rsync of `../wlshare` at `aff1945` with
   [`wlshare-tile-dump.patch`](harness/wlshare-tile-dump.patch) applied, its
   motion-bench built into `wlshare-tiledump-target/`, then
   [`dump-tiles.sh`](harness/dump-tiles.sh), which writes
   `SCENARIO-SIZE-PICTURE.tiles`: a byte for the kind (0 planar, 1 still), `w`
   and `h` as u16, the length as u32, little endian, then the body.
2. **The general corpora**: `enwik9` (mattmahoney.net) and the Silesia corpus as
   one tar, each `zstd -3` and `zstd -19` into `corpus/`.
3. **The variants**: a tree each in `variants/A`… (a `git archive` of the
   commit, with the patches above), then [`build.sh`](harness/build.sh) `A B …`
   on the timing machine; [`libzstd/`](harness/libzstd) is built the same way
   into `bin/Z-native`.
4. [`run.sh`](harness/run.sh) `OUT.jsonl VARIANT…` (wasm through
   [`run-wasi.mjs`](harness/run-wasi.mjs); `PLATFORMS=native` for native alone),
   on a quiet machine, then [`summary.py`](harness/summary.py) `OUT.jsonl
   [BASE]` for the tables against BASE, A by default (`uv run --python 3.13
   python -I summary.py OUT.jsonl M`). `build.sh` uses a variant's
   `rust-toolchain.toml` if it has one, else the default toolchain.
