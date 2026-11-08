# Decode benchmarks

What the decoder costs, and what each change to it bought. The crate is a
general zstd decoder, so the workloads are standard corpora first: Silesia,
whole and by file, at several levels, and zstd's own small-frame and dictionary
set. A workload of its own is added only for what none of them covers. wlshare's
lossless tiles, which zDesk's page decodes in WebAssembly
(`zdesk/frontend/wasm/egfx`), are kept as a check on a real user, two of them
since 2026-11-05 (see "the workloads, for a generic decoder" below); the runs
before that date measured 14.

Where the decoder stands now, against libzstd, natively and in wasm, from the latest full
run: [`latest.md`](latest.md). The sections below are each change's own run, oldest first.

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
  which ruzstd-decompress's does not: its figures for the small still-heavy samples at
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
- libzstd's lead over ruzstd-decompress falls from 2.6 times to 1.8 times natively over
  all, and on the video and mixed tiles from 4.5–6.7 times to 2.2–2.5 times.
- `decode` and `interop` fuzzing, four minutes each, found nothing.

## 2026-11-04: libzstd-style sequence tables

The question: do the sequences decode faster from tables laid out as libzstd's
(`ZSTD_seqSymbol`), each state holding the code's value and extra bits and the
next state's base and bit count (`src/decoding/sequence_table.rs`)? Before,
each sequence's codes were turned into values with a lookup apiece, and RLE
blocks had a loop of their own. Predefined tables are now also kept from one
block, and frame, to the next.

| | |
|---|---|
| **N** | `master` at `cfb8f3d` (v0.9.2-alpha.3) |
| **T** | N with the tables |

The same workloads, machine and build as the `peek-perf` run. Every variant
decoded every workload to the same output.

A profile first (`perf`, native) said where the time goes:
- On the video tiles, 64% is Huffman literals (`decode_4_streams`) and about
  3% sequences.
- On mixed, 34% is literals and about 10% sequences.
- On silesia `-3`, about 38% is sequences.

The tables could only pay on the general corpora and the smaller tiles, and
that is where they did. Geomean over the 18 workloads, against N:

| | T |
|---|---:|
| native | **−6.1%** |
| wasm | **−9.1%** |

| workload | native N | native T | wasm N | wasm T |
|---|---:|---:|---:|---:|
| video 1920x1080 | 465 | 462 (−0.6%) | 594 | 587 (−1.1%) |
| video 2880x1800@2 | 1495 | 1459 (−2.4%) | 1975 | 1879 (−4.9%) |
| mixed 1920x1080, 200 MB | 675 | 671 (−0.6%) | 882 | 851 (−3.5%) |
| mixed 2880x1800@2, 200 MB | 968 | 969 (+0.1%) | 1293 | 1201 (−7.1%) |
| enwik9 `-3` | 2656 | 2433 (−8.4%) | 3653 | 3205 (−12.3%) |
| enwik9 `-19` | 2164 | 1987 (−8.2%) | 2980 | 2647 (−11.2%) |
| silesia `-3` | 476 | 440 (−7.6%) | 664 | 581 (−12.5%) |
| silesia `-19` | 469 | 431 (−8.1%) | 679 | 586 (−13.6%) |

The smaller tiles (browser, drag, still, terminal, uncover) gain 3–15% natively
and 5–15% in wasm. Every workload:
[`results/run4-seq-tables.md`](results/run4-seq-tables.md) (native and wasm)
and [`results/run5-seq-tables-native-pinned.md`](results/run5-seq-tables-native-pinned.md)
(native, glibc's thresholds fixed, which the native figures above are from).

**glibc's malloc and the small native workloads.** In the first run, native
still 1920x1080 took 1.15 ms for T against N's 0.50 ms (+129%), every round.
The cause was page faults, about 320 a pass for T and almost none for N, not
the decoding. The harness makes a new decoder every pass. glibc moves its mmap
threshold as blocks are freed, so whether that decoder's buffers come from the
heap or as fresh pages to fault in turns on the build's allocation pattern.
With the thresholds fixed (`GLIBC_TUNABLES=glibc.malloc.mmap_threshold=67108864:glibc.malloc.trim_threshold=268435456`),
T takes 0.47 ms and N 0.51 ms. The same effect is in the earlier runs' small
native workloads:
- drag 1920x1080: 0.92 ms for N then, 0.46 ms with the thresholds fixed;
- terminal 1920x1080: 2.55 ms then, 1.12 ms fixed;
- still 2880x1800@2: 2.38 ms then, 1.19 ms fixed;
- the 1.45 ms of v0.9.2-alpha.2 on still 1920x1080 in the `peek-perf` run.

So those earlier figures, and their libzstd ratios, say as much about glibc as
about the decoders. The large workloads (video, mixed, the corpora) moved by
under 4%. Fixing either threshold alone made it worse for both, 1.4–1.8 ms; only
the pair holds. Both harnesses now set the same pair themselves at start
(`mallopt`, on glibc: a 64 MiB mmap threshold and a 256 MiB trim threshold), so
no setting of the environment is needed. wasm's allocator does
not give memory back, so its runs are unaffected.

Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
nothing.

A native run with the thresholds set by the harnesses, no environment
needed, on the corpora of "Running it again" (enwik9 traded for the video):
[`results/run6-native-mallopt.md`](results/run6-native-mallopt.md). It agrees
with the run under `GLIBC_TUNABLES` to within 3.3% on every workload both have,
most within 2%. It also gives libzstd's lead without glibc's noise: 35% by
geomean over alpha.3 (N), so about 1.5 times, not the 2.6 the first run gave.

| workload | N | T | libzstd | libzstd's lead over T |
|---|---:|---:|---:|---:|
| video 1920x1080 | 465 | 464 | 191 | 2.4× |
| video 2880x1800@2 | 1494 | 1470 | 619 | 2.4× |
| mixed 1920x1080, 200 MB | 684 | 666 | 302 | 2.2× |
| mixed 2880x1800@2, 200 MB | 975 | 946 | 447 | 2.1× |
| silesia `-3` | 474 | 445 | 214 | 2.1× |
| silesia `-19` | 469 | 433 | 219 | 2.0× |
| video clip, raw blocks | 16.2 | 16.4 | 20.3 | 0.8× |

The small tiles are closer: libzstd leads by 1.1–1.2 times on browser and drag,
1.2–1.5 on terminal and 1.6–1.9 on uncover. On still, ruzstd-decompress is ahead (0.95 and
0.83 times), but libzstd's harness zero-fills a whole bitmap for every still body
and ruzstd-decompress's does not.
On the video clip ruzstd-decompress is ahead: there is nothing to decode, only blocks to
copy, so that compares how the two harnesses take the output, not the
decoders.

## 2026-11-04: the Huffman literals' loop

The question: on video and mixed tiles, where ruzstd-decompress was furthest behind
libzstd, most of the time is Huffman literals. A `perf` profile put 62% of
video and 34% of mixed in `decode_4_streams`. Can that loop be made faster
without unsafe code?

| | |
|---|---|
| **U** | `master` at `2e3fb1d` (the sequence tables) |
| **F** | U with the new loop (`src/huff0/huff0_decoder.rs`) |
| **Z** | libzstd |

The loop, in steps, each kept only if the quick checks on lab1 (video,
mixed, silesia, native and wasm) said so:

1. **Local readers and fixed chunks.** It runs on copies of the four stream
   readers, so their state can stay in registers; before, every symbol stored
   its reader's bit count back to memory. The output is taken `N` bytes at a
   time as arrays, `N` a constant chosen from the codes' longest, so the stores
   need no bounds check.
2. **A fixed 11-bit table.** The table always has 2048 entries, a shorter
   code's entry repeated for every value of the bits after it. The index is
   then a fixed shift, and needs no bounds check.
3. **A left-aligned window, libzstd's chain.** Each symbol comes off the top
   of a register holding the reader's bits: the index is a fixed shift of it,
   and the next window one shift by the entry's length. A sentinel bit at the
   bottom counts the bits used, so a stream's state in the loop is that one
   register.
4. **The fast range.** The loop runs only while every stream has 8 bytes of
   source left, so a refill cannot reach the end. Then the slow refill, which
   takes the reader by reference, is not in the loop, and the readers stay in
   registers. In wasm, the store that reference forced had been the loop's
   hottest instruction. The last symbols are decoded on the readers
   themselves.
5. **Tables allocated on their first build.** zDesk makes a decoder for every
   planar tile. Before this, each new decoder zeroed its tables: the Huffman
   table's 4 KB and the three sequence tables' 12 KB.

Tried and left out:
- **Two streams at a time** instead of four: slower on both, worse natively
  (+7% on video against `master`).
- **A 32-bit bit count** to save wasm's 8-bit masks: no change.
- **The cold refill taking the reader by value.** It got the readers out of
  memory in wasm (−7% video), but cost the sequence loop natively, which
  small, very compressible tiles are mostly made of (drag +8%, terminal +5%).
  Step 4 gets the same in wasm without that cost.
- **Narrower tables for short codes** (7, 8 and 9 bits wide): no help to the
  small tiles, and slower video in wasm.
- **Per-tile functions in the harness**, to keep the timing loop out of V8's
  unoptimized tier: wasm-opt inlines a function with one caller, whatever Rust
  asks. Running with `--no-liftoff` showed the tier was not costing anything.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against U:

| | F |
|---|---:|
| native | **−6.2%** |
| wasm | **−5.6%** |

| workload | native U | native F | wasm U | wasm F | libzstd |
|---|---:|---:|---:|---:|---:|
| video 1920x1080 | 462 | 361 (−21.9%) | 595 | 490 (−17.7%) | 190 |
| video 2880x1800@2 | 1472 | 1226 (−16.7%) | 1895 | 1612 (−14.9%) | 609 |
| mixed 1920x1080, 200 MB | 674 | 550 (−18.4%) | 856 | 717 (−16.3%) | 302 |
| mixed 2880x1800@2, 200 MB | 946 | 852 (−10.0%) | 1227 | 1058 (−13.8%) | 444 |
| silesia `-3` | 444 | 447 (+0.8%) | 580 | 564 (−2.8%) | 218 |
| silesia `-19` | 436 | 443 (+1.6%) | 593 | 570 (−3.9%) | 223 |

The smaller tiles are between −9% and +2%, the gains on browser and still and
the losses (drag, uncover 1920x1080 in wasm) within their rounds' spread.
Every workload: [`results/run7-huffman.md`](results/run7-huffman.md).

- libzstd's lead natively falls from 2.4 times to 1.9–2.0 on video, and from
  2.1–2.2 to 1.8–1.9 on mixed. Over all, by geomean, it falls from 1.46 to 1.37
  times.
- The wasm build is 237 KB, from 215: the loop is compiled once for each of
  four `N`.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-04: table builds and the ring buffer

The question: after the Huffman loop, the small tiles (uncover, terminal,
drag, still, browser) were where ruzstd-decompress was furthest behind libzstd for
their size. zDesk makes a decoder for every planar tile, and every tile is a
frame of its own, with its own tables. Where does their time go?

A `perf` profile (native, uncover 1920x1080) said:
- 27% was building FSE tables, a division for every state;
- the forward bit reader, which reads the tables' descriptions, read a byte at
  a time;
- on terminal tiles, a third of the time was the ring buffer growing. A new
  decoder's buffer started empty and doubled as it filled, each time zeroing a
  buffer of the new size and copying the old one in: 4.8 MB zeroed a pass for
  1.7 MB decoded.

| | |
|---|---|
| **H** | `master` at `6621152` (the Huffman loop) |
| **J** | H with the three commits |
| **Z** | libzstd |

The commits, each kept only if the quick checks on lab1 said so:

1. **Tables built as libzstd builds them.** A symbol of probability `p` has
   the states `p..2p`, so each state's bits and base come from a counter and a
   leading-zero count. The symbols are laid out in a row 8 bytes at a time
   before they are spread. The sequence tables are built straight from the
   distribution; the FSE table in between is no longer built. A test checks the
   tables against the old build for 3,000 random distributions.
2. **The forward bit reader reads 8 bytes at a time.**
3. **The ring buffer is initialised as it is written.** Growing copies only
   the data, and the ring is initialised 16 KB ahead of where the tail writes,
   so a buffer is never zeroed whole. A frame that gives its content size
   reserves it at once, up to one block (128 KB), which a block of a few bytes
   can make a decoder allocate anyway; a header that claims more costs nothing
   until the data comes.

Tried and left out:
- **Each code's value looked up once per table**, not for each state, and the
  spread written without bounds checks: no change.
- **The initialised length checked on its own** before each copy: silesia 7%
  slower in wasm. The check is folded into the bounds the copies already
  test.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against H:

| | J | libzstd |
|---|---:|---:|
| native | **−12.1%** | −26.4% |
| wasm | **−9.4%** | |

| workload | native H | native J | wasm H | wasm J | libzstd |
|---|---:|---:|---:|---:|---:|
| terminal 1920x1080 | 1.01 | 0.80 (−21.5%) | 1.15 | 0.96 (−16.7%) | 0.69 |
| uncover 1920x1080 | 2.79 | 2.25 (−19.5%) | 3.24 | 2.84 (−12.5%) | 1.49 |
| still 1920x1080 | 0.43 | 0.35 (−19.4%) | 0.52 | 0.44 (−15.8%) | 0.52 |
| drag 1920x1080 | 0.43 | 0.35 (−18.7%) | 0.49 | 0.45 (−9.9%) | 0.39 |
| browser 1920x1080 | 5.17 | 4.52 (−12.5%) | 6.04 | 5.42 (−10.2%) | 4.76 |
| video 1920x1080 | 365 | 346 (−5.2%) | 488 | 422 (−13.6%) | 191 |
| video 2880x1800@2 | 1224 | 1138 (−7.0%) | 1611 | 1424 (−11.6%) | 603 |
| mixed 1920x1080, 200 MB | 556 | 527 (−5.3%) | 710 | 651 (−8.4%) | 306 |
| mixed 2880x1800@2, 200 MB | 833 | 783 (−5.9%) | 1053 | 994 (−5.6%) | 451 |
| silesia `-3` | 441 | 421 (−4.6%) | 583 | 571 (−2.2%) | 215 |
| silesia `-19` | 445 | 429 (−3.5%) | 564 | 562 (−0.4%) | 222 |

ms a pass. The 2880x1800@2 small tiles gain 11–18% natively and 7–12% in
wasm. Every workload: [`results/run9-tables.md`](results/run9-tables.md).

- Natively, J now decodes still, drag and browser tiles faster than libzstd,
  and terminal within 1.2 times. Over all, by geomean, libzstd's lead falls
  from 1.36 to 1.19 times. On video, mixed and silesia it is still 1.7–2.0
  times: there libzstd decodes Huffman literals two symbols to a lookup (its
  X2 tables), which ruzstd-decompress does not.
- The wasm build is the same size, 237 KB.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-04: Huffman literals two symbols to a lookup

The question: on video and mixed tiles, the four-stream Huffman loop was
44–59% of the time, and libzstd still 1.7–1.9 times faster. libzstd decodes
the literals of most such blocks two symbols to a lookup (its X2 tables).
Would ruzstd-decompress gain by doing the same, natively and in wasm?

What the literals look like, counted over every compressed literals block:
- Video and mixed: 98–99% of the literals' bytes come from blocks of codes up
  to 11 bits long, compressed to 0.57–0.67 of their size, mostly in blocks
  over 64 KB. A table indexed by 11 bits holding two symbols wherever both
  codes fit in them takes 0.60–0.63 lookups a symbol.
- The small tiles: 0.85–0.90 of their size, in blocks under 16 KB; 0.79–0.86
  lookups a symbol, too few saved to pay for building the table.
- Silesia: its literals are 12 MB of the 212 MB, 0.85–0.89 of their size.

| | |
|---|---|
| **K** | `main` at `f295533` (the table builds and the ring buffer) |
| **X** | K with the table of pairs (`fa966df`) |
| **Z** | libzstd |

The commit:
- **A table of pairs**, made from the single symbols' table: an index's first
  symbol is the single symbols' entry for it, and the bits after that code,
  shifted up, index the same table for the second. Four bytes an entry: the
  two symbols, their bits, and whether there is a second.
- **The four streams decode from it** five lookups a stream a refill (55 of
  the 56 bits a refill leaves), each writing two bytes and keeping one or
  both. The window is reloaded without a branch. A stream's last few bytes
  (fewer than 10), and the symbols in the last 8 bytes of its source, are
  decoded from the single symbols' table.
- **libzstd's choice, block by block** (`HUF_selectDecoder`): its estimates
  of each table's build and decode, from the literals' size and ratio. The
  small tiles and Silesia mostly keep the single symbols' table. A block that
  repeats the previous table chooses again, building the pairs if it needs
  them.
- A test decodes random streams, under 2,000 random complete codes, from
  both tables, and checks they give the same symbols and read the same bits.

Tried and left out, each in the quick checks on lab1:
- **The refill's branch kept**, as in the single symbols' loop: video 7–8%
  faster natively, not 12%.
- **The table of pairs' functions kept out of line**, and **the single
  symbols' four-stream loop with the same branchless reload**: no gain on
  any workload.
- **The four streams' output positions packed into one `u32`.** For wasm
  (`+simd128`), rustc 1.99 and zDesk's `nightly-2026-10-31` vectorize the
  four positions' additions: every lookup moves the four entries into a
  vector and the positions back out (`i32x4.replace_lane`, `extract_lane`).
  Packed, the loop has no SIMD, and is slower: video 1% slower in wasm than K,
  and 7.5% faster natively instead of 12%. The vectorized loop is not what
  holds wasm back.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against K:

| | X | libzstd |
|---|---:|---:|
| native | **−1.6%** | −15.5% |
| wasm | **−1.7%** | |

| workload | native K | native X | wasm K | wasm X | libzstd |
|---|---:|---:|---:|---:|---:|
| video 1920x1080 | 345 | 306 (−11.4%) | 420 | 391 (−6.9%) | 190 |
| video 2880x1800@2 | 1142 | 1050 (−8.1%) | 1442 | 1354 (−6.1%) | 605 |
| mixed 1920x1080, 200 MB | 523 | 490 (−6.4%) | 650 | 618 (−5.0%) | 304 |
| mixed 2880x1800@2, 200 MB | 774 | 737 (−4.7%) | 986 | 946 (−4.0%) | 445 |
| silesia `-3` | 426 | 428 (+0.5%) | 577 | 581 (+0.7%) | 218 |
| silesia `-19` | 431 | 427 (−0.9%) | 580 | 578 (−0.3%) | 224 |

ms a pass. The small tiles and the video corpus are within 2.2% of K either
way, inside the spread of the runs. Every workload:
[`results/run10-huffman-x2.md`](results/run10-huffman-x2.md).

- On video and mixed tiles, libzstd's lead falls from 1.7–1.9 times to
  1.6–1.7 times natively. By geomean over all workloads, libzstd is 1.16
  times faster (1.18 against K).
- Wasm gains about half of what native does. A profile of video in V8 puts
  `decode_literals` at 52% of the time, from 60%: V8's code for the loop
  moves registers around every variable shift, and spills the streams'
  state.
- The wasm build grows by 3.8 KB, to 241 KB.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-04: the sequence loop

The question: on uncover tiles, decoding and executing the sequences was about
40% of the time, and libzstd's loop took about half as long. Where did it go?

A `perf` profile (native, uncover 1920x1080) said:
- 12% was the bit reader: two refills a sequence, each with its branches, and
  a check of the bits left;
- 8% was the ring buffer, which made room three times a sequence (the
  literals, the match, and the match's copy), and about 6% more was in
  `memcpy`, `extend` and `repeat_in_chunks`;
- a sequence's literals and match took two trips through the buffer, each
  with its own checks of the data's length and wrapping.

| | |
|---|---|
| **X** | `main` at `6ec54e8` (the table of pairs) |
| **S** | X with the commit (`81a25fe`) |
| **Z** | libzstd |

The commit:
- **A sequence in one copy** (`RingBuffer::push_sequence`): the literals and
  then the match, with one check of the room from the tail, initialised, and
  no reserve. The match's source must be in the data that runs on to where it
  is written, whether the data wraps or not, and at least 16 bytes back;
  pieces up to 32 bytes are copied in 16-byte chunks, longer ones in one copy.
  The literals are followed by 16 bytes of padding for the chunks. It takes
  89–98% of the tiles' sequences; the rest go the old way.
- **The sequence loop reads a window of the reader's bits**, as the Huffman
  loop does: a branchless refill a sequence, and a second only when the three
  lengths' extra bits are more than 30, as libzstd's does on 64-bit. It runs
  while the source has room for two refills, so it needs no check of the bits
  left; the last sequences go the old way.
- **The offset history** is updated by a pick and selects, not branches. A
  test checks it against the old update in every case.
- The ring buffer's test against a `Vec` takes sequences too, wrapped and
  close ones included.

Tried and left out, each in the quick checks on lab1:
- **The fast copy only where the data does not wrap**: on 41% of uncover's
  sequences, 67% of browser's and 81% of mixed's, it does. Mixed was 2–3%
  slower than X natively.
- **Every piece in 16-byte chunks**, however long: silesia 2–3% slower in wasm
  than with the long ones in one copy.
- **A match from less than 16 bytes back** as a repeated pattern, written a
  chunk at a time (1–8% of the tiles' sequences): no faster, and 2 KB more
  wasm.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against X:

| | S | libzstd |
|---|---:|---:|
| native | **−10.4%** | −14.5% |
| wasm | **−11.6%** | |

| workload | native X | native S | wasm X | wasm S | libzstd |
|---|---:|---:|---:|---:|---:|
| terminal 1920x1080 | 0.78 | 0.73 (−6.8%) | 0.94 | 0.88 (−7.2%) | 0.72 |
| uncover 1920x1080 | 2.25 | 1.98 (−12.0%) | 2.81 | 2.44 (−13.1%) | 1.45 |
| still 1920x1080 | 0.34 | 0.30 (−12.8%) | 0.44 | 0.37 (−15.6%) | 0.49 |
| drag 1920x1080 | 0.35 | 0.29 (−17.9%) | 0.44 | 0.36 (−17.2%) | 0.40 |
| browser 1920x1080 | 4.50 | 4.17 (−7.4%) | 5.46 | 4.98 (−8.9%) | 4.81 |
| video 1920x1080 | 306 | 298 (−2.6%) | 392 | 383 (−2.4%) | 190 |
| video 2880x1800@2 | 1027 | 955 (−6.9%) | 1346 | 1196 (−11.2%) | 611 |
| mixed 1920x1080, 200 MB | 488 | 461 (−5.6%) | 625 | 589 (−5.8%) | 302 |
| mixed 2880x1800@2, 200 MB | 737 | 668 (−9.4%) | 959 | 873 (−8.9%) | 445 |
| silesia `-3` | 426 | 338 (−20.6%) | 574 | 448 (−21.9%) | 216 |
| silesia `-19` | 428 | 355 (−17.0%) | 583 | 459 (−21.3%) | 225 |

ms a pass. The 2880x1800@2 small tiles gain 9–16% natively and 11–18% in
wasm. The video corpus, raw blocks, which the commit does not change, is 1.3%
slower natively and 3.0% in wasm, about the spread of its runs. Every
workload: [`results/run11-sequences.md`](results/run11-sequences.md).

- By geomean over all workloads, libzstd is now 1.05 times faster natively
  (1.17 against X). ruzstd-decompress decodes still, drag, browser and the
  2880x1800@2 terminal tiles faster than libzstd, and the 1920x1080 terminal
  tiles about as fast; uncover is 1.2–1.4 times slower, and video, mixed and
  silesia 1.5–1.6 times (from 1.6–2.0).
- The wasm build grows by 3.1 KB, to 244 KB.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-04: what was left of the table builds

The question: on uncover tiles, building the sequences' tables was 14% of the
time natively, reading their descriptions 6%, and filling the Huffman tables
about 4%. What can be cut?

What the profiles and counts said, natively unless they say wasm:
- uncover builds 847 sequence tables a pass, of 32 to 128 states each, 63,200
  states in all, at 4.5 ns a state (a microbenchmark rebuilding them, which
  agrees with the profile). About two thirds of that is the loop working out
  each state's bits and base, and a tenth the walk that spreads the symbols.
  The loop is bound by its count of instructions, about 34 a state with the
  rest, at 2.6 a cycle. Each symbol's count is a chain through memory, but
  breaking it did not help (below).
- Only 1% of uncover's tables are the same as the one before them (69% of
  terminal's), so keeping a table between blocks would do little for it.
- Reading a table's description spends most of its time on a chain from one
  value to the next: the count of what is left, the leading zeros of that
  count for the bits to read, then the read, each through a `Result`.
- uncover builds 88 Huffman tables a pass, most of their codes at most 9 bits
  long, and every build fills all 2048 entries: 180,000 entries for 119,000
  literals. The fill went a symbol at a time, its length changing from one
  symbol to the next.

The harness changed first. zDesk's page now keeps one decoder for its planar
tiles and one for the still stream, reset for each tile or stream, and the
harness had gone on making a new one for each. Both harnesses now keep theirs
for a pass; libzstd's already kept one context for the planar tiles. Every
tile decodes to the same bitmap as before. Against the old harness, ruzstd-decompress's
times fell by the setup the page no longer does: terminal 14% natively and in
wasm, video 5%, still 2%; libzstd's moved within the noise. So this run's
times are lower than run 11's for the same code, and are not to be set beside
them.

| | |
|---|---|
| **S** | `main` at `0168447` (the sequence loop, released as `v0.9.2-alpha.4`) |
| **TB** | S with the commits (`a6506ad`) |
| **Z** | libzstd |

Both S and TB were built with the new harness (`cb43531`). The commits:
- **The sequence tables' symbols without the walk.** With no "less than one"
  probability the walk round the table skips no state, so its `k`th step is
  `k * step` round the table, and state `u` holds the symbol laid out at `u`
  times the inverse of the step (odd, so there is one). The states take their
  symbols from there: no walk, and no array of the symbols. A table with "less
  than one" symbols, about a fifth of uncover's, walks as before. The codes'
  values come from arrays padded to 64, with no match on the kind or bounds
  check a state. The microbenchmark went from 4.50 to 3.62 ns a state.
- **The descriptions read as libzstd reads them** (`FSE_readNCount`): the bits
  a value takes from a threshold that only falls, not a count of leading
  zeros, read from a window of the source, the end checked once after the last
  value. A test reads 200,000 random sources against the old reader.
- **The Huffman table filled a code length at a time**: the symbols sorted by
  their codes' lengths, each length in a loop of its own, on a fixed number of
  entries each, which stores them as wide as they are.
- **At most 255 weights.** The fuzzer found that a tree's FSE-coded weights
  can run to 256 or 257, past a byte's symbols, which the new fill indexed out
  of bounds; the old fill wrote symbol 256 as 0. They are now refused, with a
  test. The check is one comparison a tree, made after the run.

Tried and left out, in the microbenchmark (uncover's 847 tables, 4.18 ns a
state with the padded values alone) and the quick checks on lab1:
- **The next state's count loaded before this state's is stored**, so that two
  states of a symbol in a row wait on no store: 4.41 ns. Three in a row of two
  symbols still chain.
- **The table's two halves side by side**, each with counts of its own, the
  second's from a count of the first half's symbols: 4.75 ns.
- **An entry built as one `u64`** to store it once: LLVM stored it in two
  halves with shifts, 4.53 ns. libzstd's walk two at a time, without the
  inverse: 4.30.
- **The tables with "less than one" symbols without the walk too**, the
  laid-out row given a gap at each step that comes to a top state: no faster
  than walking them.

In wasm the sequence tables' change did nothing: in V8's code the build is
12% of uncover with it and without (the microbenchmark, run under V8, had it
15% faster, and took twice the time the decoder spends in it). TB's gains in wasm
are the descriptions' and the Huffman tables'.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against S:

| | TB | libzstd |
|---|---:|---:|
| native | **−3.2%** | −2.7% |
| wasm | **−2.2%** | |

| workload | native S | native TB | wasm S | wasm TB | libzstd |
|---|---:|---:|---:|---:|---:|
| terminal 1920x1080 | 0.62 | 0.60 (−3.7%) | 0.75 | 0.72 (−3.5%) | 0.70 |
| uncover 1920x1080 | 2.00 | 1.87 (−6.6%) | 2.45 | 2.35 (−3.8%) | 1.47 |
| still 1920x1080 | 0.30 | 0.30 (−1.0%) | 0.37 | 0.36 (−0.8%) | 0.53 |
| drag 1920x1080 | 0.30 | 0.29 (−3.3%) | 0.36 | 0.36 (+0.8%) | 0.42 |
| browser 1920x1080 | 4.16 | 3.93 (−5.6%) | 5.01 | 4.79 (−4.3%) | 4.84 |
| video 1920x1080 | 285 | 283 (−0.7%) | 363 | 367 (+1.1%) | 189 |
| video 2880x1800@2 | 933 | 912 (−2.2%) | 1170 | 1159 (−1.0%) | 604 |
| mixed 1920x1080, 200 MB | 447 | 442 (−1.2%) | 593 | 564 (−4.9%) | 305 |
| mixed 2880x1800@2, 200 MB | 647 | 637 (−1.6%) | 827 | 819 (−0.9%) | 449 |
| silesia `-3` | 338 | 331 (−2.1%) | 447 | 444 (−0.7%) | 216 |
| silesia `-19` | 338 | 332 (−1.9%) | 457 | 456 (−0.2%) | 218 |

ms a pass. The 2880x1800@2 small tiles gain 3.5–7.3% natively and 2.9–5.9%
in wasm. Video and the corpora build few tables for what they decode, and
move by about the spread of their runs; mixed in wasm (−4.9%) by about its
spread (4.1%). Every workload:
[`results/run12-table-builds.md`](results/run12-table-builds.md).

- By geomean over all workloads, libzstd and ruzstd-decompress are now even natively
  (libzstd 0.995 times TB's time, 1.03 times S's with this harness). ruzstd-decompress
  decodes still, drag, browser and terminal tiles faster than libzstd
  (libzstd's still figures are pessimistic, as before: its harness zero-fills
  a bitmap for each still body); uncover is 1.1–1.3 times slower, and video,
  mixed and silesia 1.4–1.5 times.
- In TB's profile of uncover the sequence tables' build is still 13% natively
  and 12% in wasm, and reading their descriptions 6%; the Huffman tables'
  build is about 4% natively, half of it sorting the symbols by length.
- The wasm build grows by 3.7 KB, to 248 KB: the Huffman fill's loop for each
  of the eleven lengths.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  the weights' overrun above, and nothing after the fix.

## 2026-11-04: the table of pairs' loop in wasm

The question: the table of pairs (run 10) made video 8–11% faster natively but
only 6–7% in wasm, where the literals were still 60% of video's time in V8's
profile. What in V8's code for the loop costs, and can it be avoided?

What V8's code for the four-stream loop showed, on video 1920x1080:
- **Every variable shift's count goes through `cl`.** x86-64 takes the count
  of a shift by a register in `cl` alone (V8's code has no BMI2 `shlx`, which
  the processor has), and
  `rcx` also held the entry or the index, so each lookup's shift came with up
  to three moves between registers.
- **Two of the four streams' windows were in memory**, loaded and stored at
  each lookup: the four windows, four output pointers, the table and the
  memory's base are more than V8 has registers for.
- **The four output positions in one vector**, as in run 10: each round of
  four lookups moves their entries into a vector and the positions back out,
  and V8 builds each vector constant again where it is used.
- **The streams' ends.** The four streams decode together while all of them
  have 8 bytes of source left; the first to near its end stops them all, and
  the other three went on a symbol at a time from the single symbols' table.
  `decode_stream` was 4% of video.
- **The table's build** (3% of video) wrote each entry as four byte stores.

| | |
|---|---|
| **A5** | `main` at `0e3496f` (collected as decoded, released as `v0.9.2-alpha.5`) |
| **HW** | A5 with the commit (`4869e70`) |
| **Z** | libzstd |

The commit:
- **In wasm, the window moves on by a multiplication**: by two to the power of
  the entry's bits, which is the shift modulo 2^64, and a multiplication takes
  any registers. Natively the shift stays; the multiplication made video 5–7%
  slower there.
- **The entry is one `u32`**: the two symbols, whether there is a second, and
  from bit 20 the bits (natively) or two to their power (wasm, at most 2^11).
  The build writes it in one store.
- **Each stream finishes from the table of pairs** after the four stop
  together, a stream at a time, and only its last bytes from the single
  symbols' table. In the quick checks this was worth as much as the
  multiplication on video in wasm, and nothing without it.

Tried and left out, in the quick checks on lab1 (video, mixed and silesia,
native and wasm):
- **Two streams at a time**, so that their state fits in V8's registers:
  video 15% slower in wasm and 25–40% natively.
- **The multiplication natively too**: video 5–7% slower natively, mixed 3–6%.
- **The entry holding how many symbols it has, masked with 3**, which saves
  an addition and one of the vector constants a round: slower on both (video
  2.5% natively against the entry alone), the masked count also needing 14
  bytes of output a refill, not 10.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against A5:

| | HW | libzstd |
|---|---:|---:|
| native | −0.3% | +2.2% |
| wasm | **−0.8%** | |

| workload | native A5 | native HW | wasm A5 | wasm HW | libzstd |
|---|---:|---:|---:|---:|---:|
| video 1920x1080 | 270 | 267 (−1.1%) | 354 | 337 (**−4.6%**) | 191 |
| video 2880x1800@2 | 877 | 874 (−0.3%) | 1145 | 1074 (**−6.2%**) | 613 |
| mixed 1920x1080, 200 MB | 419 | 416 (−0.9%) | 548 | 514 (**−6.3%**) | 302 |
| mixed 2880x1800@2, 200 MB | 637 | 618 (−3.0%) | 793 | 780 (−1.6%) | 446 |
| silesia `-3` | 339 | 334 (−1.7%) | 449 | 459 (+2.3%) | 215 |
| silesia `-19` | 333 | 339 (+1.6%) | 460 | 463 (+0.8%) | 220 |
| uncover 1920x1080 | 1.87 | 1.90 (+1.9%) | 2.34 | 2.36 (+0.9%) | 1.50 |

ms a pass. The small tiles mostly decode their literals from the single
symbols' table, which this does not change, and are within 2.2% of A5 either
way. Silesia's +2.3% in wasm is inside its rounds' spread (5.2%). Every
workload: [`results/run13-huffman-wasm.md`](results/run13-huffman-wasm.md).

- The gain is smaller than the quick checks had it (video 1920x1080 −7.7% in
  wasm, mixed −5.4%), and mixed 2880x1800@2 gains a quarter of what mixed
  1920x1080 does.
- In wasm, video now takes 1.23–1.26 times its native time, from 1.31.
- A5 collects as it decodes (`v0.9.2-alpha.5`), so its planar frames are no
  longer hashed: its video is 5% faster natively than run 12's TB. By geomean
  natively, HW takes 0.97 times libzstd's time. ruzstd-decompress is still
  1.37–1.43 times slower on video and mixed, 1.5 times on silesia, and
  1.1–1.3 times on uncover.
- The wasm build grows by 0.7 KB, to 248 KB: the single stream's loop.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-04: hashing only frames with a checksum

The question: a native profile of `main` on uncover 1920x1080 put XXH64 at
5.8% of the time. The decoder never checks a checksum itself; what was it
hashing?

Since the bytes are collected as they are decoded (`v0.9.2-alpha.5`), the
decode buffer hashes each byte as it drops it past the window, so that
`get_calculated_checksum` can give the frame's checksum at its end. zDesk's
still stream is one frame with a 1 MiB window and no checksum, and in the
tiles it runs on for far more than a MiB: one stream a pass of 454 bodies in
uncover, 447 in browser, 134 in terminal, 1,430 in video. Every byte of it
past the first MiB was hashed, for nothing. libzstd makes no hash for a frame
without a checksum. (The still tiles' two streams are under a MiB, and drop
nothing.)

| | |
|---|---|
| **B0** | `main` at `d3b4aaa` (the table of pairs' loop in wasm) |
| **CK** | B0 with the commit (`10a15e5`) |
| **Z** | libzstd |

The commit hashes a frame's bytes only when its header has the checksum flag;
`get_calculated_checksum` is `None` for a frame without one. A test takes
frames with and without a checksum in turn on one decoder, collected a block
at a time or at the end, and checks both checksums and the bytes.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against B0:

| | CK | libzstd |
|---|---:|---:|
| native | **−5.0%** | +1.8% |
| wasm | **−3.8%** | |

| workload | native B0 | native CK | wasm B0 | wasm CK | libzstd |
|---|---:|---:|---:|---:|---:|
| browser 1920x1080 | 3.99 | 3.29 (**−17.5%**) | 4.81 | 4.14 (**−13.9%**) | 4.79 |
| browser 2880x1800@2 | 9.71 | 8.47 (**−12.7%**) | 11.92 | 10.66 (**−10.5%**) | 13.06 |
| terminal 1920x1080 | 0.58 | 0.53 (−8.6%) | 0.71 | 0.66 (−7.6%) | 0.70 |
| terminal 2880x1800@2 | 1.41 | 1.20 (**−14.7%**) | 1.72 | 1.53 (**−10.7%**) | 1.77 |
| uncover 1920x1080 | 1.88 | 1.77 (−5.4%) | 2.35 | 2.26 (−3.9%) | 1.50 |
| uncover 2880x1800@2 | 3.24 | 2.99 (−7.7%) | 4.11 | 3.86 (−5.9%) | 2.80 |
| video 1920x1080 | 268 | 260 (−2.8%) | 342 | 325 (−5.1%) | 190 |
| video 2880x1800@2 | 883 | 847 (−4.1%) | 1071 | 1069 (−0.2%) | 600 |
| mixed 1920x1080, 200 MB | 420 | 405 (−3.6%) | 523 | 531 (+1.5%) | 303 |
| mixed 2880x1800@2, 200 MB | 621 | 597 (−3.9%) | 777 | 759 (−2.4%) | 446 |
| silesia `-3` | 331 | 336 (+1.7%) | 454 | 449 (−1.2%) | 220 |
| silesia `-19` | 341 | 348 (+2.2%) | 463 | 473 (+2.2%) | 220 |

ms a pass. The still and drag tiles and the video corpus are within 3.7% either
way. Silesia's
frames have checksums (the `zstd` command's default), so they hash as before;
their +1.7–2.2% is a little over the spread of their rounds natively
(0.7–4.3%), with nothing in the commit on their path but a test of whether to
hash. Every workload: [`results/run14-checksum.md`](results/run14-checksum.md).

- By geomean natively, ruzstd-decompress now takes 0.93 times libzstd's time. It is
  1.46–1.54 times faster on browser tiles and 1.3–1.5 times on terminal; uncover is
  1.07–1.18 times slower, video and mixed 1.3–1.4 times, silesia 1.5–1.6.
- The profile that found this also shows what the roadmap had wrong: the
  harness's `replay`, 15% of uncover natively, is the sequences' decoding
  inlined into it (14.7% of the 15.3%), not a copy of the output.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-05: the sequences executed inline, to the stream's end

The question: decoding and executing the sequences were 44% of uncover
1920x1080 natively, and uncover the one kind of tile libzstd still decoded
faster (1.07–1.18 times). Where in the loop did the time go?

What a native profile of `main` said:
- **A call a sequence.** The loop handed each sequence to a closure, and LLVM
  had not inlined it: 24% of uncover was in the closure's own code, which
  saved six registers, loaded the executor's state from memory, and read and
  wrote the offset history through a pointer. The loop around it kept the bit
  reader's state on the stack, its address being taken by the refills of the
  loop after it.
- **The stream's last bytes.** The fast loop stops with fewer than 15 bytes of
  the stream left, room for two refills, and the rest went a sequence at a
  time through the reader's checked refills: uncover's blocks are small
  (about 7 KB out each), so these were a good share of its sequences. With the
  first change, their out-of-line execution alone was 3.9% of uncover.

| | |
|---|---|
| **D0** | `main` at `7bae314` (hashing only frames with a checksum) |
| **SQ** | D0 with the commit (`5531084`) |
| **Z** | libzstd |

The commit:
- **The executor inline in the fast loop.** `decode_sequences` takes the
  executor, not a closure, and the fast loop runs on a copy of the reader
  whose address nothing takes. The sequences after it go through an
  out-of-line `execute`.
- **The fast loop to the stream's end.** Near the stream's start, the bytes
  left are copied after 16 zero bytes (`BitReaderReversed::padded`), and the
  same loop goes on there, to every sequence but the last, which reads no
  states. On a bad stream it can read past the start; the bits remaining,
  checked after it, then fail the block, which is undone as before. A test
  reads a padded copy against the reader, past the start too, at every step
  where one can be made.

Tried and left out, in the quick checks on lab1:
- **The executor's state in registers**: the offset history kept by value and
  written back at the end, the repeat picked by selects, not an index into an
  array, and the slow copy taking the buffer alone, so that nothing takes the
  executor's address. No faster on the tiles, and silesia 2–3% slower
  natively than the inline loop alone.
- **One loop that switches to the padded copy** as it goes: terminal 6–9%
  slower natively (V8 did well with it). Calling the loop as it was a second
  time, on the copy, kept the native gains.

Every variant decoded every workload to the same output. Geomean over the 17
workloads, against D0:

| | SQ | libzstd |
|---|---:|---:|
| native | **−3.0%** | +8.6% |
| wasm | **−3.4%** | |

| workload | native D0 | native SQ | wasm D0 | wasm SQ | libzstd |
|---|---:|---:|---:|---:|---:|
| uncover 1920x1080 | 1.76 | 1.71 (−2.7%) | 2.24 | 2.23 (−0.6%) | 1.45 |
| uncover 2880x1800@2 | 2.97 | 2.90 (−2.5%) | 3.88 | 3.76 (−2.9%) | 2.84 |
| terminal 2880x1800@2 | 1.21 | 1.16 (−3.6%) | 1.52 | 1.48 (−2.1%) | 1.94 |
| still 1920x1080 | 0.29 | 0.28 (−5.2%) | 0.37 | 0.35 (−4.4%) | 0.49 |
| drag 1920x1080 | 0.29 | 0.27 (−6.5%) | 0.36 | 0.35 (−3.6%) | 0.41 |
| browser 2880x1800@2 | 8.44 | 8.19 (−3.0%) | 10.82 | 10.44 (−3.5%) | 12.98 |
| video 1920x1080 | 259 | 263 (+1.3%) | 316 | 314 (−0.7%) | 189 |
| video 2880x1800@2 | 841 | 822 (−2.3%) | 1060 | 1038 (−2.0%) | 609 |
| mixed 2880x1800@2, 200 MB | 601 | 576 (−4.2%) | 755 | 730 (−3.2%) | 455 |
| silesia `-3` | 333 | 320 (−3.9%) | 442 | 404 (**−8.6%**) | 216 |
| silesia `-19` | 330 | 321 (−2.9%) | 464 | 400 (**−13.7%**) | 222 |

ms a pass. Every small tile is 2.1–6.5% faster natively and 0.6–4.4% in wasm.
Video 1920x1080 natively (+1.3%) is a little over its rounds' spread (0.7%).
Silesia gained less natively than in the quick checks (6–8% there). Every
workload: [`results/run15-sequences-inline.md`](results/run15-sequences-inline.md).

- By geomean natively, ruzstd-decompress now takes 0.89 times libzstd's time. uncover
  is 1.02–1.18 times slower than libzstd, video, mixed and silesia 1.3–1.5
  times; the other tiles 1.4–2.1 times faster.
- In wasm, silesia now takes 1.25–1.26 times its native time, from 1.33–1.40.
- The wasm build grows by 0.3 KB, to 249 KB: the loop twice.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing.

## 2026-11-05: the workloads, for a generic decoder

The question: the crate is a general zstd decoder, but 14 of the 17 workloads
were wlshare's tiles. Which of them does a standard corpus cover, and which are
redundant? Two kinds of evidence, both from this machine, not lab1: what each
workload puts the decoder through, read from its block, literals and sequences
headers, and how the workloads moved together over the 42 variant pairs of runs
1–15.

What they exercise (share of the output from Huffman literals and from
matches, and the bytes a sequence's match copies):

| workload | from literals | from matches | match B/seq | |
|---|---:|---:|---:|---|
| video, mixed: planar frames | 88–99% | 1–12% | 7–9 | 70–175 KB frames, 4-stream Huffman |
| video, mixed: still stream | 10–79% | 21–90% | 10–20 | |
| browser, drag, still, terminal, uncover | 3–15% | 85–97% | 50–235 | the still stream: small flushed blocks, mostly predefined tables |
| silesia `-3`, `-19` | 8–15% | 84–92% | 12–14 | 128 KB blocks, FSE tables; `-19` repeat tables too |
| Silesia's `x-ray`, `-1` | 99.7% | 0.3% | 7 | like the planar frames |
| Silesia's `nci`, `-3` | 2% | 98% | 30 | the longest matches in Silesia |
| silesia `--fast=4` | 32% raw | 64% | 14 | raw literals, which nothing else has |
| GitHub users sample set, `-3` | 28% | 72% | 17 | 9,114 frames of 0.8 KB: 1-stream Huffman, predefined tables |
| the same with a dictionary | 1% | 99% | 34 | the dictionary's tables (repeat mode), treeless literals |

How they moved: video and mixed at both sizes moved together (r 0.98–0.99,
1.6–4.2 points apart on average), and so did the small still-stream tiles
(r 0.8–0.93, 2–4 points), with terminal 2880x1800@2 closest to all of them.
Silesia `-3` and `-19` agree less (r 0.93, but 8.6% against 13.7% in wasm in
run 15). still 1920x1080 is a 0.3 ms pass, as noisy as it is redundant (14.8%
spread in wasm in run 15).

The workloads from now on, 10 for 17:

- **Standard**: silesia `-3`, `-19` and `--fast=4`; Silesia's `x-ray` at `-1`
  and `nci` at `-3`; the GitHub users sample set (zstd's own small-data and
  dictionary set), each file a frame, without and with a dictionary trained on
  it (`zstd --train`, 110 KB); the video clip in raw blocks.
- **wlshare's tiles**, as a check on a real user: mixed 2880x1800@2 (200 MB),
  for video and mixed, and terminal 2880x1800@2, for the small still-stream ones.
  The others are kept on the backup drive.

The small frames go through a mode of their own, `frames FILE [DICT]`, in both
harnesses: each frame read whole into one reused buffer, by one decoder reset
for each. ruzstd-decompress and libzstd gave the same hash over all 9,114 frames,
with and without the dictionary. ### Checked on lab2 (run 16)

lab2 replaces lab1 from here: the same `AMD EPYC-Rome Processor`, 4 vCPUs (2
cores, 2 threads), 8 GB, Debian 13, kernel 6.12.115, swap off, rustc 1.99.1
(stable), Node 26.13.0; each run pinned to vCPU 2 as before. Three pairs of
earlier variants, all through the new harness, over the old and new workloads
together: X→S (run 11, the sequence loop), A5→HW (run 13, the Huffman loop in
wasm) and D0→SQ (run 15, the sequences inlined), and libzstd beside them.

X and S predate A5's collecting every byte as it is decoded, so on the tiles the
harness collects none of the still stream's bytes from them (their tile hashes
differ from the rest, both alike); the pair still compares like with like.
Every other workload decoded to the same output in every variant, libzstd
included.

Each pair's change, native / wasm:

| workload | X→S | A5→HW | D0→SQ | SQ ÷ libzstd |
|---|---:|---:|---:|---:|
| video 1920x1080 *(dropped)* | −0.5 / −1.2% | +0.9 / −7.7% | −0.3 / −5.5% | 1.37× |
| video 2880x1800@2 *(dropped)* | −8.3 / −9.8% | −2.5 / +0.1% | −2.9 / −4.2% | 1.35× |
| mixed 1920x1080 *(dropped)* | −3.7 / −8.0% | +1.6 / −2.9% | −3.5 / −2.0% | 1.32× |
| mixed 2880x1800@2 | −12.2 / −11.2% | −1.4 / −1.2% | +1.0 / −2.4% | 1.35× |
| `x-ray` `-1` | −0.3 / −1.0% | −0.2 / −5.1% | −1.2 / +2.3% | 1.20× |
| the 9 small tiles *(dropped)* | −4.3…−18.2 / −8.2…−15.6% | −0.8…+0.8 / −3.5…+0.9% | −1.8…−6.8 / −2.2…−6.4% | 0.52–1.19× |
| terminal 2880x1800@2 | −11.4 / −13.3% | +0.6 / +0.6% | −5.6 / −3.5% | 0.64× |
| `nci` `-3` | −16.8 / −14.5% | −0.5 / −0.3% | −4.1 / −8.9% | 1.41× |
| GitHub, small frames | −5.6 / −5.1% | +1.0 / +0.9% | −1.2 / −2.3% | 1.26× |
| GitHub, with the dictionary | +1.5 / +7.2% | +2.5 / +0.1% | +0.1 / −3.8% | **7.84×** |
| silesia `-3` | −20.5 / −22.6% | −1.3 / +0.4% | −5.7 / −11.3% | 1.45× |
| silesia `-19` | −20.3 / −17.3% | −1.0 / +2.6% | −4.8 / −11.3% | 1.46× |
| silesia `--fast=4` | −23.4 / −29.0% | +0.8 / +0.4% | −3.2 / −10.9% | 1.52× |
| video clip, raw blocks | −2.5 / −3.3% | −1.3 / −8.2% | −0.4 / −10.0% | 0.76× |

SQ ÷ libzstd is SQ's time over libzstd's, natively: over 1, ruzstd-decompress is slower. Every workload:
[`results/run16-workloads.md`](results/run16-workloads.md).

- The small tiles are covered: in every pair and platform their changes lie
  between the GitHub frames' and `nci`'s, or within a point or two of
  terminal 2880x1800@2's, which stays.
- Video and mixed do not move together as tightly as runs 1–15 suggested: the
  sequence loop (X→S) took 0.5% off video 1920x1080 and 12% off mixed
  2880x1800@2, as the first is nearly all literals and the second has 3 times
  the sequences. `x-ray` and mixed 2880x1800@2 bracket them in X→S, and stay
  within 2–3 points of them elsewhere; the Huffman change in wasm (A5→HW) was
  largest on video 1920x1080 (−7.7%), and `x-ray` (−5.1%) is the workload
  nearest it.
- The new workloads find what the old ones did not: with a dictionary,
  ruzstd-decompress takes 7.7 times libzstd's time (28 ms against 3.6 ms for
  the 9,114 frames), where it takes 1.25 times without one. Not profiled
  yet; the likely cost is `init_from_dict`, run for every frame, which sets the
  dictionary's content and tables up again each time.
- `--fast=4` gained the most from the sequence loop (−23 / −29%), and libzstd's
  lead over ruzstd-decompress is largest there among the Silesia levels.
- Noisy: `x-ray` in wasm (7–11% between rounds), silesia `-3` natively (8–10%)
  and the video clip natively (8–11%); the medians of three rounds held.
- A round of a two-variant run, with libzstd, times 102 s of passes on the 10
  workloads against 171 s on the 17.

## 2026-11-05: the dictionary kept between frames

The question: why does ruzstd-decompress take 7.7 times libzstd's time on small
frames with a dictionary (run 16), and only 1.25 times without one?

A profile on lab2 (perf, `github-dict.frames`, `main` at `b46cfc9`): 72% of the
time in `DecoderScratch::init_from_dict`, which every frame using the dictionary
runs on reset. 65.5% copying the dictionary's content into the decode buffer
(110 KB, for frames of 0.8 KB), and most of the rest copying its tables: the
sequence tables (17% once the content was kept) and the Huffman table (9.5%).
Most of these frames never change them: their blocks repeat the dictionary's
tables (repeat mode, treeless literals).

| | |
|---|---|
| **M** | `main` at `b46cfc9` |
| **DC** | M with the dictionary kept between frames |

What DC keeps:

- **The content**: the decode buffer keeps its copy of the dictionary's content
  through a reset, a flag saying whether the frame may use it. The scratch
  records which dictionary's content it holds, and copies only for another one.
- **The tables**: each sequence table and the Huffman table records which
  dictionary's table it holds, as `SeqTable` already recorded holding the
  predefined one. A reset leaves the entries; any build in them clears the
  record. The Huffman table keeps its table of pairs with it, which was made
  again on every frame.
- **`add_dict`** forgets all of them, since a dictionary may replace another
  under its id.

A test (`test_dict_kept_between_frames`) decodes frames with and without the
dictionary in turn, then replaces it under its id, first with other content,
then with another dictionary's tables (`dict_tests/dictionary-small`, with two
frames made with it in `dict_tests/files-small`). It fails if any one of the
content, the sequence tables, the Huffman table or `add_dict` is not
invalidated.

The workloads of "the workloads, for a generic decoder", on lab2. Every variant
decoded every workload to the same output.

| workload | native M | native DC | wasm M | wasm DC | libzstd |
|---|---:|---:|---:|---:|---:|
| GitHub, with the dictionary | 27.88 | **6.50 (−76.7%)** | 31.86 | **9.73 (−69.4%)** | 3.63 |
| GitHub, small frames | 24.40 | 24.30 (−0.4%) | 31.95 | 31.90 (−0.2%) | 19.25 |
| silesia `-3` | 285.75 | 288.02 (+0.8%) | 364.30 | 363.72 (−0.2%) | 196.25 |
| silesia `-19` | 288.62 | 291.35 (+0.9%) | 392.18 | 369.44 (−5.8%) | 201.54 |
| video clip, raw blocks | 14.98 | 15.61 (+4.2%) | 15.71 | 16.60 (+5.7%) | 19.68 |

ms a pass. The others are within 2% either way, inside their rounds' spread.
The video clip's raw blocks do not reach the code changed. Run again alone, M
and DC in turn six times each, natively it was 15.15 and 15.04 ms: noise, as
its wasm spread (13.3%) says too. Every workload:
[`results/run17-dict.md`](results/run17-dict.md).

- With the dictionary, ruzstd-decompress now takes 1.8 times libzstd's time,
  where it took 7.7 times, against 1.25 times without a dictionary. Of what is left, most is the sequences' decoding and execution,
  the copies from the dictionary's content among them.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found
  nothing, both on nightly with AddressSanitizer and on stable without a
  sanitizer, which is how `AGENTS.md` asks for it from here.

## 2026-11-05: the sequences with the buffer's state in locals, natively

The question: natively, ruzstd-decompress takes 1.45–1.52 times libzstd's time on silesia
(run 16). Where does the difference go?

Profiles on lab2 (perf, silesia `-3`, `main` at `20f34d1`, against libzstd 1.5.7):

| | ruzstd-decompress | libzstd |
|---|---:|---:|
| ms a pass | 294 | 206 |
| instructions a pass | 4.0 G | 2.4 G |
| of them, the sequences' (`decode_sequences`, `ZSTD_decompressSequences`) | 3.1 G | 1.7 G |
| instructions a cycle | 3.79 | 3.28 |

The gap is the instructions, not stalls, and most of it is the sequences'. In the fast loop's
code, about 30 loads and stores a sequence go to the stack and to the decoder's state: the
ring buffer's pointer, length, head and tail are loaded again for each sequence, and its tail,
the output counter and the executor's counters stored, since any store of the sequences' bytes
could, for LLVM, be to them.

What was tried, in quick checks (three rounds, a few workloads):

| | | native silesia `-3` / `--fast=4` / `nci` | wasm, the same |
|---|---|---:|---:|
| R1 | the ring's and the executor's state in locals for the fast loop, written back at its end and before a sequence goes the slow way | −4…−6 / −8 / −6…−9% | +7 / +12 / +8…+12% |
| R2 | R1, the executor's counters through references | −6…−8 / −8 / −7% | +6…+12 / +7…+10 / +6…+10% |
| R3, R4 | R2 with only the tail in a local, or only the slice | −1 / −5 / −4% | +9…+12 / +7 / +7…+8% |
| R5 | R2, the slow sequence executed outside the inner loop | −5 / −6 / −3% | +6 / +13 / +12% |
| R6 | only a local slice of the ring, within its copy | 0% | +4 / 0 / +3% |
| R7 | R2, the fast loop not inlined | −9 / −10 / −7% | +16 / +10 / +14% |

Natively, holding the state in locals is what helps. In V8, every form of it was slower: its code
spills either way, and the values the loop loaded from memory when it needed them, which is
cheap there, took registers for the whole loop. So the two targets now differ:

| | | |
|---|---|---|
| **M** | `main` at `20f34d1` |
| **SR** | M with the commit |

- **Natively**, the fast loop executes its sequences through a run (`SequenceRun`): the ring's
  buffer as a slice and its tail in locals (`SeqWriter`), with the executor's counters, written
  back at the loop's end, and before a sequence the fast copy does not take goes the slow way.
  A writer dropped on an error leaves the buffer as it was.
- **In WebAssembly**, the run is the executor itself, and the buffer's state is read from it as
  before.
- The two share the sequence's checks and the copy. The checks first took the writer's copy
  as a closure, which LLVM left out of line: 36% of silesia's instructions. A trait's method,
  which can be marked to be inlined, replaced it.
- **The dictionary**: the native writer also copies a match that reaches into the
  dictionary's content, which would otherwise leave the run for each such sequence: the
  GitHub frames with the dictionary went from 7% slower to 4% faster. A test checks it against
  a byte at a time.
- **The WebAssembly path is tested**: `cargo test --target wasm32-wasip1` runs the tests under
  Node, without libzstd and criterion, which do not build there, and the three tests that
  compress with libzstd. Fuzzing never reaches that path.

The full run, on lab2 after a restart (run 18). Every variant decoded every workload to the
same output.

| workload | native M | native SR | wasm M | wasm SR | libzstd |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 288.30 | **276.12 (−4.2%)** | 374.18 | 364.61 (−2.6%) | 201.86 |
| silesia `-19` | 293.99 | **279.35 (−5.0%)** | 370.21 | 394.78 (+6.6%) | 205.20 |
| silesia `--fast=4` | 185.10 | **174.81 (−5.6%)** | 227.01 | 226.33 (−0.3%) | 125.20 |
| `nci` `-3` | 25.92 | **24.42 (−5.8%)** | 32.21 | 32.87 (+2.0%) | 18.28 |
| mixed 2880x1800@2 | 541.47 | **524.56 (−3.1%)** | 675.03 | 678.82 (+0.6%) | 413.20 |
| GitHub, with the dictionary | 6.42 | **6.18 (−3.7%)** | 9.70 | 9.94 (+2.5%) | 3.71 |
| GitHub, small frames | 24.37 | 24.79 (+1.7%) | 31.89 | 32.00 (+0.4%) | 20.01 |
| video clip, raw blocks | 14.99 | 16.48 (+10.0%) | 15.57 | 15.23 (−2.2%) | 19.07 |

ms a pass. `x-ray` and terminal are within 1% natively, and within 2.1% in wasm. Geomean
against M: natively −1.8%, in wasm +0.9%. Every workload:
[`results/run18-sequences-run.md`](results/run18-sequences-run.md).

- The video clip's raw blocks do not reach the code changed; its rounds' spread natively is
  9.3%.
- Run again in wasm alone, M and SR in turn (six times on silesia `-19`, four on `-3`),
  SR was +0.6% and +3.1%, each pass ranging over 7–8%: the wasm build is unchanged within
  its noise, as its code is.
- Natively, ruzstd-decompress now takes 1.36–1.40 times libzstd's time on silesia, from
  1.43–1.48.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on stable) found
  nothing.

## 2026-11-05: the Huffman tables' build

The question: natively, ruzstd-decompress took 1.24 times libzstd's time on the GitHub
sample set's small frames (run 18), where nearly every frame builds a Huffman table: 8,857
tables a pass for 9,114 frames, every one with FSE-coded weights (125 of them, 38 not 0) and
codes of up to 8 bits.

Profiles on lab2 (perf, `github.frames`, `main` at `6f431ba`, against libzstd 1.5.7), ms a
pass:

| | ruzstd-decompress | libzstd |
|---|---:|---:|
| the Huffman tables' build, their weights read | 11.5 | 8.3 |
| the literals' decoding | 4.8 | 5.6 |
| the sequences, their tables and descriptions | 5.8 | 3.9 |
| the pass | 24.5 | 19.6 |

In the build:
- **The fill**, 11% of the pass: LLVM stored the entries, a struct of two bytes, a byte at a
  time, 128 single-byte stores for each code of 5 bits. Aligned to 2 bytes, or as a `u16`,
  they were stored two bytes at a time wherever LLVM unrolled a code's copies (codes of 5 to 9
  bits).
- **The FSE-coded weights**, 15%: two decoders and a general reader, their state reloaded
  from and stored to the stack for each weight, and the weights pushed to a `Vec`.
- **Counting and sorting the weights**, 6% and more: each over every symbol, a chain of
  increments through memory of the count of weight 0, most symbols', each waiting on the one
  before; and three more passes over them, for their sum, their codes' lengths and those
  lengths' counts.

The variants, natively (quick checks, three rounds, a few workloads):

| | | `github.frames` |
|---|---|---:|
| N | `main` at `6f431ba` | 24.5 |
| H1 | N, the entries `[u8; 2]`, filled four at a time as a `u64` (16-byte stores where LLVM merges them) | 22.5 |
| H2 | H1, the weights read from a local copy of their stream by bit position, one load a read; the weights, lengths and ranks locals of the build, in one pass for their counts | 21.7 |
| H3 | H2, the counts and the sort in four lanes of the symbols, four chains through memory | 22.3 |
| H4 | H2, the symbols with a code listed first, the list's end in a register, then counted and sorted | 20.4 |
| H5 | H4, the weights read from a window in a register, reloaded once fewer than two reads' bits are left | 18.7 |

H3's lanes made the chains four times shorter, but their loop took about twice the
instructions a symbol, and was no faster. H5 is the commit, **HT**:

- The table's entries are `[u8; 2]`, whose bytes (`as_flattened_mut`) take a code's entries
  as `u64`s.
- The weights are decoded into an array on the stack, from a window that the two states read
  from in turn, over a copy of their stream after 8 zero bytes, which read as the zeros past
  its start. A test checks them against the two decoders on a `BitReaderReversed`, for random
  tables and streams.
- The weights, their counts and the sorted symbols are locals of the build. Before, the table
  kept them, and a frame using a dictionary copied them in with its table.
- `FSEDecoder` and the FSE table's own table of states served only the weights, and are gone;
  the FSE table is now the description its users build their tables from.

The full run, on lab2 after a restart (run 19). Every variant decoded every workload to the
same output.

| workload | native N | native HT | wasm N | wasm HT | libzstd |
|---|---:|---:|---:|---:|---:|
| GitHub, small frames | 24.63 | **18.76 (−23.8%)** | 32.09 | **25.50 (−20.6%)** | 19.57 |
| GitHub, with the dictionary | 6.17 | **5.74 (−6.9%)** | 9.94 | **9.44 (−5.0%)** | 3.65 |
| terminal 2880x1800@2 | 1.09 | **1.01 (−6.6%)** | 1.40 | **1.32 (−5.4%)** | 1.67 |
| `x-ray` `-1` | 10.46 | 10.72 (+2.5%) | 13.34 | 12.55 (−5.9%) | 8.85 |
| silesia `-3` | 273.82 | 274.61 (+0.3%) | 372.07 | 365.61 (−1.7%) | 197.47 |
| silesia `-19` | 291.14 | 294.38 (+1.1%) | 378.03 | 391.07 (+3.5%) | 204.61 |
| video clip, raw blocks | 15.10 | 15.23 (+0.9%) | 15.06 | 16.45 (+9.2%) | 19.03 |

ms a pass. Mixed, `nci` and `--fast=4` are within 1.5% both ways. Geomean against N: natively
−3.5%, in wasm −2.8%. Every workload:
[`results/run19-huffman-build.md`](results/run19-huffman-build.md).

- silesia `-19` in wasm, run again alone, N and HT in turn five times: HT 375.9 ms to N's
  388.9 (medians), each ranging over 6–12%. It builds at most one table a 128 KB block. The video clip's raw blocks reach no Huffman table, and its spread in wasm is 8.6%.
- Natively, ruzstd-decompress now takes 0.96 times libzstd's time on the small frames, from
  1.24, and 1.57 times with the dictionary, from 1.67. What is left of the latter is mostly
  the sequences (roadmap item 1).
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on stable) found
  nothing.

## 2026-11-05: the matches into the dictionary

The question: natively, ruzstd-decompress took 1.57 times libzstd's time on the GitHub
frames with their dictionary (run 19), mostly in the sequences. Where does it go?

Profiles on lab2 (perf, `github-dict.frames`, `main` at `627e0e1`, against libzstd 1.5.7):
both ran at 3.7–3.9 instructions a cycle, and the difference was the instructions, about
83 M a pass to libzstd's 52 M. 44% of the time was in `decode_sequences` and 14% in
`copy_from_dict`, with its `memcpy` 6% more; 4% in `execute_out_of_line`. Counted over
the frames (a pass: 9,114 frames, one block each, about 24 sequences a block):

- **59% of the sequences** match into the dictionary, all of them wholly within it, about
  50 bytes each, with less than one literal. Each one failed the fast copy's checks first,
  then called `copy_from_dict` with arguments on the stack, which checked again and copied
  by `memcpy`, and by `memmove` the empty rest from the data's start.
- **The last 2.9 sequences of each block** went one at a time through
  `execute_out_of_line`, about 146 instructions each: the padded loop stopped once its
  window reached a byte before the stream's start, with up to 56 bits of the stream left.
  The first loop took about 12 of a block's sequences, the padded one 9.
- **The tables**: 92–97% of the blocks repeat the dictionary's, but 3–6% use the predefined
  table and 2% (literal lengths) an RLE one. Each was built in the same entries as the
  dictionary's, so that the next frame copied the dictionary's table back in, and the next
  predefined block built that one again.

The variants, natively (quick checks, five rounds; the dictionary frames' ms a pass):

| | | `github-dict.frames` |
|---|---|---:|
| HT | `main` at `627e0e1` | 5.75 |
| E1–E3 | HT, `copy_from_dict` still out of line, its piece of the dictionary in chunks (up to 2, 4 or 8), no `memmove` for the empty rest | 5.8 |
| F1 | HT, the match into the dictionary taken in the writer inline, the room checked once, then whether the source is in the data or past it | 5.5 |
| F2 | F1, the executor's literals position, offset history and sum in locals of the run | 5.75, silesia `-3` 3% slower |
| F3 | F1, 32 zero bytes before the padded copy, not 16: the padded loop reads on to the block's last sequence | 5.17 |
| F4 | F3, the last sequence in the fast loop too | 5.2, terminal 25% and silesia 5% slower |
| W1 | F3, in wasm the dictionary's matches by the writer too (wasm: 9.3 to 8.46) | 5.16 |
| F5a | W1, the piece of the dictionary in one or two copies of 32 bytes | 5.10 |
| F5b | F5a, the literals and short matches in one or two copies of 16 bytes | 5.3, silesia 4% slower |
| F6 | F5a, the predefined and RLE tables in entries of their own | 4.96, silesia 12% slower |
| G4 | F6, the entries in use swapped in, and the match's chunks copied across a split | 4.75 |

- **F6's silesia** ran 12% more instructions: LLVM no longer inlined `copy_within` into the
  loop, whose 16-byte chunks of the match then called it, 6.7% of the instructions. Copied
  through an array instead, the `try_into` kept a length check (F7, F8); copied across
  `split_at_mut`, each chunk's source wholly before it since the offset is at least a
  chunk, it inlines whatever LLVM's budget (G4). F5a with the array copy alone was 3.5%
  slower on silesia: inlined, `copy_within` makes the better code.
- **G4 in wasm** (DM) was 2–7% slower on silesia. Taking out its changes one at a time and
  adding them to `main` one at a time (wasm, six rounds, silesia `-3` and `--fast=4`): the
  tables' entries cost nothing, the padding up to 3% on `--fast=4`, and both the check for
  the dictionary after the ring's fast copy failed and the fast copy written as the two
  functions the writer uses cost about 2% each. **DN** is G4 with `copy_sequence`'s own body
  in wasm, as before, and the dictionary's matches taken in the slow path, out of the loop.

DN is the commit:

- **Natively**, the writer takes a match into the dictionary inline (`SeqWriter`), its
  piece of the dictionary in at most two copies of 32 bytes where the dictionary has them.
  `copy_from_dict`'s call is gone. The match's chunks are copied across a split, not by
  `copy_within`.
- **In wasm**, the slow path (`execute_slow`) takes them by the same writer.
- The padded copy of the stream's end has 32 zero bytes before it, so that the fast loop
  takes every sequence but the block's last.
- A sequence table keeps the table built from a description, the predefined one and the RLE
  one in entries of their own, the one in use swapped in. A test checks that a predefined
  or RLE block leaves the dictionary's table for the next frame.

The full run, on lab2 after a restart (run 20). Every variant decoded every workload to the
same output.

| workload | native HT | native DN | wasm HT | wasm DN | libzstd |
|---|---:|---:|---:|---:|---:|
| GitHub, with the dictionary | 5.74 | **4.67 (−18.7%)** | 9.40 | **8.57 (−8.9%)** | 3.64 |
| GitHub, small frames | 18.85 | 18.83 (−0.1%) | 25.64 | 25.80 (+0.7%) | 19.74 |
| terminal 2880x1800@2 | 1.05 | 1.05 (+0.2%) | 1.33 | 1.31 (−1.4%) | 1.67 |
| mixed 2880x1800@2 | 523.28 | 518.93 (−0.8%) | 671.92 | 666.59 (−0.8%) | 429.92 |
| silesia `-3` | 276.93 | 286.96 (+3.6%) | 377.87 | 370.39 (−2.0%) | 204.80 |
| silesia `-19` | 286.32 | 286.33 (+0.0%) | 372.33 | 380.64 (+2.2%) | 207.93 |
| video clip, raw blocks | 15.39 | 15.15 (−1.6%) | 15.52 | 16.58 (+6.9%) | 19.97 |

ms a pass. `nci`, `x-ray` and `--fast=4` are within 3% both ways. Geomean against HT:
natively −1.7%, in wasm −0.6%. Every workload:
[`results/run20-dict.md`](results/run20-dict.md).

- Run again alone, HT and DN in turn six times each: natively silesia `-3` took 283.0 ms to
  HT's 279.7 (1.2% slower, DN's runs over 3%), and in wasm silesia `-19` 390.6 to 381.6
  (2.4%, each over 6–9%). These stay. The video clip's raw blocks reach none of the code
  changed, and its spread in wasm was 5.7%.
- Natively, ruzstd-decompress now takes 1.28 times libzstd's time on the dictionary frames,
  from 1.57.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each) found nothing, both
  on nightly with AddressSanitizer and on stable without a sanitizer.

## 2026-11-05: the short sequences' copy

The question: natively, Silesia took 1.35–1.47 times libzstd's time (run 20), the largest
gap left. Where does it go?

On lab2 (`perf stat`, silesia `-3`, `main` at `70ddb0a`, against libzstd 1.5.7): 3.94 G
instructions a pass to libzstd's 2.41 G, and 1.03 G cycles to its 0.73 G. The sequences'
decoding and execution were 79% of the instructions (3.12 G) to libzstd's 71% (1.72 G):
1.4 G of the 1.53 G between them. A pass has 14.46 M sequences. Counted over one (an
instrumented copy) and from callgrind's exact count of each instruction of one, decoded as
the harness's `frame` mode decodes:

- **About 215 instructions a sequence**, 44 of them loads and stores of the stack: the
  loop's state (the reader's, three states, three tables, the literals', the ring's, the
  offset history and the block's sum) does not fit the registers. The three table pointers
  are loaded from the stack for each sequence, and most of the entries' fields stored there
  and loaded again.
- **Taking the bits**: 8 instructions for each of the six fields a sequence, 48 in all (the
  count's `mov` to `cl` twice, its mask and `not`). `wrapping_shr` of `!n` and a `u128`
  shift both compiled to the same code.
- **The copies**: the match's chunks cost 17 instructions each, four of them bounds checks,
  and the literals' 16. 55% of the sequences have no literals; most have at most 16 of
  them and a match of at most 16 bytes, at least 16 back.
- **The slow path** took 4% of the sequences (586 K a pass), at about 197 instructions
  each: 212 K with matches from less than 16 bytes back, and 372 K matches back across the
  ring's wrap, past its start into its end.

The variants (callgrind, one pass of silesia `-3`, the whole process's instructions; the
first column decoded by `read_to_end` from a `StreamingDecoder`, the second as the harness
decodes, blocks up to 1 MB then all collected, whose ring wraps differently):

| | | `read_to_end` | harness |
|---|---|---:|---:|
| B0 | `main` at `70ddb0a` | 5,211 M | 4,087 M |
| S1 | B0, a sequence of at most 16 literals and a match of at most 16 bytes, at least 16 back, in one copy of 16 bytes each, no loops | 5,064 M | |
| S2 | S1, the ring split at the tail, the room checked on the free part's length, and the short copy's indices relative to the two parts: of its seven bounds checks, two left | 4,872 M | |
| S3 | S2, the same for up to 32 literals and 32 bytes of match, and longer ones in one copy where the source is wholly before the tail | 5,106 M | |
| S4 | S2, the block's sum out of the fast loop: its writer takes no sequence past the block's most, which the slow path then reports | 4,840 M | 3,766 M |
| S7 | S4, the matches across the wrap in the writer, the data's first part as its dictionary | 4,743 M | 3,778 M |
| S8 | S4, those matches in the slow path, by a writer of its own | | 3,794 M |
| B2 | S4, 32 sequences decoded into an array, then executed, by a function each | | 4,015 M |
| P1 | S4, a table's entry packed into a `u64` | | 3,889 M |
| B3 | B2 and P1 | | 4,086 M |

- **S3** made more of the loop's values live: decoded by `read_to_end`, its stack loads and
  stores went from 45 a sequence to 53.
- **S7** took the matches across the wrap, and the slow path took half the instructions
  (115.8 M to 55.7 M), but the loop's registers went to the case: 6 instructions a sequence
  more, all of them loads and stores of the stack. **S8** took them out of the loop, at the
  cost of the push and the repeat it saves: the writer's setup, the dictionary's copy path
  and a `memmove`.
- **B2** split the loop to free the registers. Decoding alone, `decode_batch` still kept the
  12 fields of the three entries live, which LLVM loads at once, and spilled, at 117
  instructions a sequence; with P1's entries (**B3**) it spilled hardly at all and still
  took 119: the six fields' 48, extracting the fields, the refill, the batch's stores.
- **x86-64-v3** (S4 built with `-C target-cpu=x86-64-v3`, BMI2's shifts by any register,
  `perf stat` on lab2): 7% fewer instructions on Silesia, and 0.5–2% more cycles. libzstd's
  `_bmi2` functions are not where its lead is.

S4 is the commit:

- **Natively**, the ring's writer (`SeqWriter`) takes the buffer split at the tail. A
  sequence of at most 16 literals and a match of at most 16 bytes, its source wholly before
  the tail, is two copies of 16 bytes, from slices the room check bounds: `copy_short`.
  The literals must have `CHUNK` bytes after them, not `CHUNK - 1`.
- The writer takes no sequence that would take the block past its most; the slow path adds
  each of its sequences to the block's sum and reports one past it, and a run adds what its
  writer wrote. In wasm, the sum is checked for each sequence as before.

On lab2 (`perf stat`, one run of each, a pass, natively):

| workload | B0 instructions | S4 instructions | B0 cycles | S4 cycles | libzstd cycles |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 3,936 M | 3,625 M (−7.9%) | 1,017 M | 950 M (−6.6%) | 722 M |
| silesia `-19` | 3,725 M | 3,427 M (−8.0%) | 1,058 M | 1,008 M (−4.8%) | 746 M |
| silesia `--fast=4` | 2,432 M | 2,241 M (−7.9%) | 658 M | 585 M (−11.0%) | 465 M |
| GitHub, with the dictionary | 69.4 M | 67.3 M (−2.9%) | 17.9 M | 17.6 M (−1.8%) | 13.3 M |
| GitHub, small frames | 204.6 M | 198.0 M (−3.2%) | 69.4 M | 67.7 M (−2.4%) | 73.1 M |
| terminal 2880x1800@2 | 11.73 M | 11.19 M (−4.6%) | 3.92 M | 3.77 M (−3.8%) | 6.72 M |

Its branch mispredictions on silesia `-3` went from 7.67 M a pass to 5.56 M, near
libzstd's 5.39 M: much of the cycles saved.

The full run, on lab2 after a restart (run 21; S4 is `SF` there, with the tests' changes).
Every variant decoded every workload to the same output.

| workload | native B0 | native S4 | wasm B0 | wasm S4 | libzstd |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 295.78 | **264.76 (−10.5%)** | 373.29 | 377.08 (+1.0%) | 202.61 |
| silesia `-19` | 293.92 | **280.07 (−4.7%)** | 378.85 | 382.37 (+0.9%) | 211.78 |
| silesia `--fast=4` | 181.54 | **169.09 (−6.9%)** | 229.09 | 231.58 (+1.1%) | 126.42 |
| `nci` | 25.34 | 24.23 (−4.4%) | 33.19 | 32.72 (−1.4%) | 18.52 |
| GitHub, with the dictionary | 4.79 | 4.88 (+1.9%) | 8.60 | 8.82 (+2.5%) | 4.01 |
| GitHub, small frames | 18.74 | 18.50 (−1.3%) | 25.50 | 25.45 (−0.2%) | 19.48 |
| terminal 2880x1800@2 | 1.05 | 1.03 (−1.3%) | 1.30 | 1.30 (+0.2%) | 1.66 |
| mixed 2880x1800@2 | 520.49 | 510.05 (−2.0%) | 673.17 | 679.78 (+1.0%) | 417.30 |

ms a pass. `x-ray` and the video clip are within 3% both ways, but `x-ray` in wasm
(−4.6%, over a 12% spread). Geomean against B0:
natively −3.1%, in wasm +0.1%. Every workload:
[`results/run21-seq.md`](results/run21-seq.md). B0's spread on silesia `-3` was 10%.

- Run again alone, B0 and S4 in turn six times each: natively, the dictionary frames took
  4.69 ms to B0's 4.77 (1.7% faster), silesia `-3` 271.9 to 282.2. In wasm the dictionary
  frames took 8.59 to 8.49 and silesia `-3` 376.9 to 368.5, each over 8–12%; by `perf stat`
  under Node, S4's wasm ran the same instructions as B0's on silesia `-3` (5,550 M a pass),
  with the same mispredictions (8.1 M), in 1–3% fewer or more cycles from one run to the
  next. The wasm build's fast loop is as it was.
- Natively, ruzstd-decompress now takes 1.31 times libzstd's time on silesia `-3`, from 1.46.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on nightly with
  AddressSanitizer) found nothing.

## 2026-11-05: the table of pairs' rounds

The question: after run 21, where is the rest of the gap to libzstd, and is any of it in
the longer sequences' copies or the table builds, the roadmap's next items?

Sampled on lab2 (`perf record`, `main` at `4b694db`, against libzstd 1.5.7), a pass:

- **silesia `-3`**: 236 M cycles more than libzstd, 225 M of them the sequences' decoding
  and execution (726 M to 501 M). That is their instructions (3,625 M to 2,408 M for the
  whole pass, `perf stat`), at more of them a cycle (3.9 to 3.2); the cache misses, the
  loads stalled on a store and the TLB misses are much the same as libzstd's. The longer
  sequences' copies are about 10 of the 245 instructions a sequence (callgrind).
- **mixed's tiles**: 2,408 M cycles to 2,037 M. The sequences 767 M to 615 M; the Huffman
  decoding 754 M to 643 M, nearly all of it the four streams' loop from the table of pairs
  (about 650 M, libzstd's assembly loop for its pairs about 540 M); the Huffman tables
  108 M to 64 M; the sequence tables 68 M to 59 M.
- **the GitHub frames**: 70 M to 73 M; the Huffman tables 25 M to 31 M, the sequence tables
  4 M to 5 M. With their dictionary, 18 M to 14 M, the gap the sequences'.

So the table of pairs' loop. On `x-ray`, which it decodes nearly all of, one pass is
7.84 M lookups, and the loop (`decode_4_streams_sized`, callgrind) 145.0 M instructions,
18.5 a lookup: about 11 the lookup's own, a third of those loads and stores of the stack,
since the four windows, the four output chunks, their offsets and the table are more than
the registers hold; and a round of 5 lookups a stream cost about 45 more checking each
stream's output room, 11 checking its source's bytes, and 16 a refill, with its bounds
checks. libzstd's assembly loop takes about 9 a lookup.

The variants (lab2, `perf stat`, a pass on `x-ray`, natively; mixed's counts move by up to
4% from one run to the next, more than most of these differ):

| | | instructions | cycles |
|---|---|---:|---:|
| K0 | `main` at `4b694db` | 159.3 M | 38.3 M |
| K1 | the entry's bits at its lowest, for the shift's count as loaded | 156.1 M | 38.6 M |
| K2 | K1, each round's lookups into an array on the stack, then copied out | 155.4 M | 41.3 M |
| K3 | K1, a round two refills, its checks once | 153.6 M | 37.2 M |
| K4 | K3, the entry in bytes of their own, each loaded on its own | 153.8 M | 42.7 M |
| K5 | K3 without K1 | | |

- **K1** made a window live on the stack, its shifts read and written in memory.
- **K2** freed the output chunks' registers and stalled on its copies' loads, each over
  several of the round's stores.
- **K4** took 1 instruction a lookup fewer than K3 (11.5, callgrind) and its tables' build
  7 M more; it ran 14% more cycles on `x-ray`.
- **K5** was as fast as K3 (`quick.sh`, four rounds), so K1 is not kept.

Then the round's refills as a constant (`quick.sh`, ms a pass, median of four rounds,
natively; two runs):

| refills a round | `x-ray` | mixed | `x-ray` | mixed |
|---|---:|---:|---:|---:|
| 1 (K0) | 10.37 | 502 | 10.42 | 516 |
| 2 | 9.91 | 494 | | |
| 3 | 9.73 | 498 | | |
| 4 | 9.60 | 487 | 9.68 | 488 |
| 6 | | | 9.75 | 508 |
| 8 | | | 9.66 | 494 |

In wasm 2 or more were 10–17% slower in V8 (with 2, mixed 728 ms to 664, `x-ray` 16.0 to
13.7), though K3's two refills written out were not.

The commit (HR): natively, the four streams' loop from the table of pairs takes 4 refills a
round, 5 lookups a stream after each, checking each stream's output room and its source's
bytes (`can_refill_fast_times`) once a round; in wasm 1, as before. The lookup and the
room take the chunk's size as a parameter. Its wasm ran the same instructions as K0's
under Node (194 M a pass on `x-ray`, 43.26 G over the whole run on mixed), in the same
cycles to within a run's spread.

On lab2 (`perf stat`, one run of each, a pass, natively):

| workload | K0 instructions | HR instructions | K0 cycles | HR cycles | libzstd cycles |
|---|---:|---:|---:|---:|---:|
| `x-ray` | 159.3 M | 137.9 M (−13.4%) | 38.1 M | 35.0 M (−8.0%) | 31.8 M |
| mixed 2880x1800@2 | 7,068 M | 6,576 M (−7.0%) | 2,223 M | 2,105 M (−5.3%) | 1,838 M |
| silesia `-3` | 3,625 M | 3,604 M (−0.6%) | 987 M | 946 M (within its spread) | 727 M |
| `nci` | 293.6 M | 293.3 M | 88.2 M | 88.8 M | 70.6 M |
| GitHub, small frames | 198.0 M | 198.0 M | 69.0 M | 70.2 M | 71.5 M |
| terminal 2880x1800@2 | 11.19 M | 11.19 M | 3.73 M | 3.76 M | 6.25 M |

On `x-ray` (callgrind) the loop went from 145.0 M instructions to 123.5 M, 15.8 a lookup,
and the pass from 187.6 M to 166.2 M. The GitHub frames and terminal's tiles, whose
literals are small, run the same instructions.

The full run, on lab2 after a restart (run 22). Every variant decoded every workload to the
same output.

| workload | native K0 | native HR | wasm K0 | wasm HR | libzstd |
|---|---:|---:|---:|---:|---:|
| `x-ray` | 10.99 | **9.68 (−11.9%)** | 13.32 | 13.19 (−1.0%) | 8.91 |
| mixed 2880x1800@2 | 501.79 | 494.55 (−1.4%) | 672.25 | 703.03 (+4.6%) | 414.92 |
| silesia `-3` | 261.04 | 254.91 (−2.3%) | 368.71 | 368.05 (−0.2%) | 197.12 |
| `nci` | 25.14 | 23.83 (−5.2%) | 34.28 | 32.53 (−5.1%) | 19.05 |
| GitHub, with the dictionary | 4.71 | 4.70 (−0.4%) | 8.63 | 8.56 (−0.8%) | 3.75 |
| GitHub, small frames | 18.48 | 18.87 (+2.1%) | 25.38 | 26.12 (+2.9%) | 19.58 |
| terminal 2880x1800@2 | 1.03 | 1.02 (−0.8%) | 1.30 | 1.33 (+2.3%) | 1.65 |

ms a pass. Silesia `-19` and `--fast=4` and the video clip are within 2.5% natively; in wasm
`-19` −5.6% and the video clip −7.3%, over spreads of 7–13%. Geomean against K0: natively
−1.6%, in wasm −1.1%. Every workload: [`results/run22-huf.md`](results/run22-huf.md).
K0's spread on `x-ray` was 16%.

- The wasm build's code is as it was, so its differences are the runs' spread: on mixed,
  K0 and HR in turn three times each under `perf stat` took 667–686 ms and 669–710, in
  12.57–12.93 G cycles and 12.63–13.17 G, at the same instructions. Natively on mixed,
  HR took 489–492 ms to K0's 504–513.
- `nci`'s −5% (natively and in wasm) is not the change: it runs the same instructions.
- Natively, ruzstd-decompress now takes 1.09 times libzstd's time on `x-ray`, from 1.21 in
  run 21, and 1.19 on mixed's tiles, from 1.22; 1.07 by geomean over the workloads.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on nightly with
  AddressSanitizer) found nothing.

## 2026-11-06: Silesia in wasm since alpha.7

The question: v0.9.2-alpha.8's release comparison had Silesia 2–6% slower in wasm than
v0.9.2-alpha.7, over spreads of 0.4–4.1% (8.7% for `-19`), with every other workload faster.
Which of #21–#28 is it?

Each merge built as a variant (Q21 to Q28, `main` after #21 to #28), with v0.9.2-alpha.7 (A7),
all with the harness of #21 on. Their times were too spread to tell (five rounds on lab2:
silesia `-3`'s spread 6–13% each, one pass of 965 ms). Their instructions are not (wasm
under Node, `perf stat` of the whole process over its passes, three runs each, a pass;
[`results/run23-counts.log`](results/run23-counts.log)):

| | silesia `-3` | `--fast=4` | `nci` | `x-ray` |
|---|---:|---:|---:|---:|
| A7 | 5,446 M | 3,315 M | 430.5 M | 195.2 M |
| Q21 (#21, the benchmarks) | 5,447 M | 3,315 M | 430.6 M | 195.0 M |
| Q22 (#22, the dictionary kept) | 5,446 M | 3,313 M | 430.4 M | 195.1 M |
| Q23 (#23, the sequences' run) | **5,569 M (+2.2%)** | **3,379 M (+1.9%)** | **441.9 M (+2.6%)** | 195.2 M |
| Q24 to Q28 | 5,549–5,569 M | 3,377–3,379 M | 440.2–441.6 M | 194.3–195.3 M |

A build's three counts differ by 0.7% at most, mostly by under 0.1%. So #23, which gave the native build its
run of sequences with the buffer's state in locals and kept wasm's executor as it was: its
wasm runs about 8 instructions more a sequence on silesia, `x-ray`, nearly all literals, as
many as before. The cycles moved by 1–6%, about as much as from one run to the next.

What it changed for wasm: the sequence's copy became `copy_sequence`, shared with the
native writer, which took the ring's `Vec` and made a slice of it at each chunk's copy. In
the module, both chunk loops now load the `Vec`'s pointer and length for each chunk (the
match's loop 45 wasm instructions to 35 before), as the chunks' stores might have changed
them. The variants, on `main` at `c59e9f0` (Q28):

| | | silesia `-3` | `--fast=4` | `nci` |
|---|---|---:|---:|---:|
| Q28 | `main` | 5,553 M | 3,378 M | 441.4 M |
| WS2 | the ring's buffer as a slice to the limit, once a sequence | 5,521 M | 3,357 M | 436.8 M |
| WS4 | WS2, its room checked against the slice's length, not the limit | 5,542 M | 3,382 M | 436.8 M |
| WS5 | WS2, `copy_sequence` taking a slice (the commit) | 5,521 M | 3,357 M | 436.5 M |

The chunk loops are as before #23 in WS2 and on. Moving the block's sum and its check back
after the copy, as alpha.7 had them, took 85 M instructions more than WS2 on silesia `-3`.
The rest, 75 M a pass, is V8's code for the loop as #23 wrote it, which is not more of the
stack's loads and stores: sampled over its instructions, they are 18% of the function in
WS2, 22% in Q22.

The full run of the wasm build, on lab2 after a restart, six rounds (run 23; Q28 and WS5
decoded every workload to the same output):

| workload | Q28 | WS5 | spread Q28 |
|---|---:|---:|---:|
| silesia `-3` | 372.15 | 365.53 (−1.8%) | 8.3% |
| silesia `-19` | 378.11 | 375.15 (−0.8%) | 4.4% |
| silesia `--fast=4` | 231.56 | 226.23 (−2.3%) | 6.4% |
| `nci` | 32.76 | 32.37 (−1.2%) | 8.2% |
| `x-ray` | 12.74 | 12.85 (+0.9%) | 10.3% |
| GitHub, with the dictionary | 8.41 | 8.32 (−1.1%) | 1.6% |
| GitHub, small frames | 25.51 | 25.32 (−0.7%) | 4.9% |
| mixed 2880x1800@2 | 668.48 | 675.03 (+1.0%) | 6.7% |
| terminal 2880x1800@2 | 1.30 | 1.29 (−0.8%) | 1.9% |
| video clip, raw blocks | 16.05 | 15.46 (−3.7%) | 10.2% |

ms a pass, the median of six rounds; geomean −1.1%, every difference within its spread.
Every workload: [`results/run23-wasm.md`](results/run23-wasm.md). The native build's code is
unchanged.

- The release comparison's 2–6% was partly the runs' spread: here Q28's times, paired
  round by round with A7's, were 2–3% slower on `nci` and `--fast=4`, and its cycles
  from 1.5% fewer to 2.7% more on Silesia.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on nightly with
  AddressSanitizer) found nothing; it runs the native build, whose code is unchanged.

## 2026-11-06: the tiles' decoders kept across the passes

The question: in wasm on terminal's tiles, 35% of the time is the harness's `replay`, and
16% libc's `memmove` and `memset`. What are they?

`replay`, in V8's code, is nearly all the sequences' fast loop, inlined into it (`perf`
over `node --perf-prof`, a build of `main` at `6647a72`). The libc time is large copies
(`memmove`), forward and backward, and `memset`. Natively (callgrind, the same harness),
copying and zeroing were 44% of terminal's instructions. 11.3% was the ring buffer's
`reserve_amortized` (17 a pass, each copying the ring's data to a buffer twice the size),
and 6.3% its `init_ahead` zeroing the room it grew into (77 a pass). Neither should happen
more than once for a decoder that is reused: a reset keeps the ring's capacity.

`replay` made its two `FrameDecoder`s and the bitmap's buffer anew each pass. So every
pass grew both rings from empty, the still stream's (with no content size, and a window of
up to 8 MB) a doubling at a time from the 128 KB a frame is given at its start. zDesk's
`WlshareTiles` keeps them for the session. Both harnesses (`decode_bench` and libzstd's)
now make them once, in a `Session`, for all the passes. On terminal's tiles, a pass went
from 21.97 M instructions to 17.71 M natively (callgrind, ten passes against six:
`reserve_amortized` now runs 17 times in all, in the first). What is left of libc natively
(13% of the samples) is copying: the still stream's bodies to the bitmap (6.8%), the long
matches (2.4%), and `read_to_end` zeroing its buffer (0.8%).

The full run, on lab2 after a restart (run 24): `main`'s code with the harness before (WS5)
and after (NH), and libzstd's with each (Z, ZS). Every variant decoded every workload to the
same output.

| workload | native WS5 | native NH | wasm WS5 | wasm NH | Z | ZS |
|---|---:|---:|---:|---:|---:|---:|
| terminal 2880x1800@2 | 1.02 | **0.93 (−8.3%)** | 1.28 | **1.19 (−6.8%)** | 1.65 | 1.65 |
| mixed 2880x1800@2 | 493.98 | 482.53 (−2.3%) | 674.34 | 668.78 (−0.8%) | 413.35 | 414.74 |

ms a pass, the median of three rounds. The other workloads, whose harness and decoder are
as they were, moved by up to 6.7% natively and 4.3% in wasm, mostly within their spreads;
the GitHub frames beyond them (natively +3.0% over 0.9%; in wasm +1.8% over 0.6%, and with
their dictionary +3.9% over 3.6%). Every workload:
[`results/run24-session.md`](results/run24-session.md).

- libzstd's times are as they were: its contexts do not grow a buffer a doubling at a time.
- Natively, ruzstd-decompress now takes 0.57 times libzstd's time on terminal's tiles,
  from 0.62, and 1.16 on mixed's, from 1.19; 1.06 by geomean over the workloads.
- Tile results from before run 24 count the growth in every pass. They are not comparable
  with those after it.
- The decoder is unchanged. A frame's ring is given its window at the start, up to its
  content size and a block, so that a header claiming a large frame costs nothing until
  its data does. The still stream, whose size is not known, still grows from there: once
  for a reused decoder, as in zDesk.

## 2026-11-06: the table of pairs' checks in wasm

The question: in wasm, what does a round of the four streams' loop from the table of pairs
spend besides its lookups? On `x-ray`, which it decodes nearly all of, the loop is 85% of
the instructions sampled (`perf` over `node --perf-prof`, `main` at `74df403`). Of a round
of 20 lookups, about 40 instructions are the four streams' output room checks, which LLVM
vectorizes and V8 takes back lane by lane, and about 24 the refills' bounds checks on each
source (`&source[index..][..8]`), with the length loaded from the stack.

The four output positions go through a vector (LLVM's SLP vectorizer) on the way to each
store. With that vectorizer off for the whole build (`-C llvm-args=-vectorize-slp=false`),
the wasm was slower, not faster (harness medians, three rounds): `x-ray` 5.8%, mixed's tiles
4.8%, Silesia `-3` and `nci` 2.8–2.9%. The vector keeps the positions out of the general
registers, of which V8 has too few for the loop.

The variants (lab2, wasm unless said, a pass's instructions and cycles over the whole
process, the median of the runs):

| | | `x-ray` instructions | `x-ray` cycles |
|---|---|---:|---:|
| M30 | `main` at `74df403` | 194.2 M | 47.8–51.0 M |
| P1 | each stream's last offset with a round's room, found once: one compare a round | −4.1% | within the spread |
| P2 | P1, the refills from the source to the container's end, checked against its length once a round | −10.0% | between P1's and M30's |
| P2c | P2 without the mask on the bytes taken | −10.7% | below M30's |
| P3 | P2c, the sequences' fast loop refilled the same way | −11.3% | |

- **P1**: the four room checks became one vector compare (`i32x4.le_u`, `all_true`).
- **P2** put two operations on each stream's chain from one lookup to the next, at each
  refill: the mask on the bytes taken (`& 63`, which proved the refill in bounds), and the
  `- 8` to the window's start, which wasm cannot fold into the load. Its loop runs fewer
  instructions than P1's in more cycles: it waits on those chains.
- **P2c** takes the bytes as at most 8, as `trailing_zeros` is at most 64 to the compiler,
  and checks `8 + 8 * refills` bytes a round: no mask, no bounds checks. Writing the offset
  as `(len - 8) - bytes`, to take the `- 8` off the chain, LLVM wrote back to
  `(len - bytes) - 8` with a bounds check.
- **P3** ran fewer instructions on Silesia (wasm 2.2%, natively 1.1%) in more cycles (wasm
  `-3` 1.7%, `nci` 2.1%): the tail's length is on the sequences' chain.

The wasm builds' times move with the process: each run lands in a faster or a slower mode,
`x-ray` 12.0 or 13 ms and more, for M30 as for P2c. In 16 rounds of each in turn (`perf
stat`, a pass), P2c against M30:

| | `x-ray` cycles | `x-ray` median | mixed cycles | mixed median |
|---|---:|---:|---:|---:|
| wasm | −7.3% | −7.4% (14 of 16 rounds faster) | −0.8% | −0.8% |
| native | +1.4% | +1.0% | −0.3% | −1.1% |

Mixed's tiles in wasm, in four such sets of runs, were −1.2%, +1.9%, −0.8% and +2.3%: no
clear change. Natively the loop runs 4 refills a round and its instructions fell 3.6% on
`x-ray`, in 1% more cycles there, in every run.

The commit (P2c): in the four streams' loop from the table of pairs, each stream's last
offset with a round's room is found once, and the refills are from the source up to the
container's end (`BitReaderReversed::tail`, `refill_tail`, `resume_tail`), checked once a
round. A round now stops with up to 40 bytes of a source left natively, where it stopped
at 37, the rest decoded a stream at a time as before; in wasm at 16, as before.

The full run, on lab2 after a restart (run 25). Every variant decoded every workload to the
same output.

| workload | native M30 | native P2c | wasm M30 | wasm P2c | libzstd |
|---|---:|---:|---:|---:|---:|
| `x-ray` | 9.63 | 9.73 (+1.0%) | 12.78 | 12.76 (−0.1%) | 8.87 |
| mixed 2880x1800@2 | 481.25 | 481.47 (+0.0%) | 661.62 | 676.93 (+2.3%) | 412.30 |
| silesia `-3` | 259.54 | 254.40 (−2.0%) | 376.73 | 367.40 (−2.5%) | 197.09 |
| terminal 2880x1800@2 | 0.93 | 0.96 (+3.2%) | 1.21 | 1.19 (−2.1%) | 1.70 |

ms a pass, the median of three rounds; geomean against M30 natively −1.2%, in wasm −0.5%.
Beyond M30's spread: natively Silesia `-19` −5.2% (over 2.3%), terminal's tiles +3.2%
(over 2.6%), the GitHub frames with their dictionary −1.5% (over 0.8%) and `x-ray`; in wasm
the GitHub frames with their dictionary +4.9% (over 1.8%) and Silesia `-19` +1.7% (over
1.1%). Its wasm rounds on `x-ray` were 12.25, 14.59 and
12.76 ms for P2c, 12.78, 12.80 and 12.54 for M30: three rounds do not tell the modes
apart, and the 16 above are the measure. Every workload:
[`results/run25-tail.md`](results/run25-tail.md).

- Natively, ruzstd-decompress takes 1.10 times libzstd's time on `x-ray`, and 1.17 on
  mixed's tiles.
- Fuzzing (`decode`, `decode_dict` and `interop`, three minutes each, on nightly with
  AddressSanitizer) found nothing.

## 2026-11-06: unsafe code in the native loops

The question: with the ban on unsafe code lifted (`b4c6dca`), how close do the sequences' and
the Huffman streams' loops come to libzstd natively, written as libzstd writes them? Before,
on silesia `-3`, ruzstd-decompress ran 3,600 M instructions a pass to libzstd's 2,408 M, the
sequences' loop nearly all of the difference, with 44 loads and stores of the stack a
sequence and a bounds check on each copy (see the [roadmap](../roadmap.md), item 1).

The steps (lab2, natively, a pass's instructions and cycles, `perf stat` of the harness, one
run each; `main` is U0):

| | | silesia `-3` instructions | cycles |
|---|---|---:|---:|
| U0 | `main` at `abd76b0` | 3,600 M | 969 M |
| U1 | the sequences' loop by raw pointers: `RawRun`, `RawWriter`, `refill_raw` | 3,067 M | 843 M |
| U2 | U1, a BMI2 build of the loop chosen by `cpuid`; the offset history picked by selects; matches across the ring's wrap in the loop | 2,802 M | 850 M |
| U3 | U2, the offset history in three fields, the room check against one address | 2,819 M | 833–859 M |
| U5 | U3, the sequences counted in chunks the source has room for; the shifts without masks | 2,733 M | 845 M |
| U6 | U5, the four Huffman streams' loops by raw pointers, in a BMI2 build | 2,645 M | 810–855 M |
| U10 | U6, the padded copy in the same call; each block's last sequence in the loop | 2,674 M | 818–858 M |
| libzstd | | 2,408 M | 718–736 M |

- **U1** keeps the reader's position as a pointer, refilled with no checks, and the
  literals' position, the offset history and the ring's tail in locals; a sequence is
  copied as libzstd's `ZSTD_execSequence` copies it, in a chunk of 16 and then chunks of 32
  past its end into free space (`WILD`, 32: the literals' padding grew to that), after one
  check for room. A match within 16 bytes is spread as `ZSTD_overlapCopy8` spreads it, out of
  line, and one into the dictionary in the loop.
- **U2**: the BMI2 build's shifts take their count from any register (`shlx`, `shrx`),
  9% fewer instructions, in as many cycles, as `x86-64-v3` did in run 21. Where the data
  wraps, the ring's older part, from the head to its end, is taken as the dictionary is: of
  silesia `-3`'s 14.46 M sequences a pass, 2.6% reach back across the wrap, which went to the
  slow path before; now 1,493 do, those for which the ring needs room.
- From U2 on, ruzstd-decompress runs as many instructions a cycle as libzstd (3.3 on silesia
  `-3`), and the instructions have stopped telling the cycles: U5 runs 3% fewer than U3 in as
  many. A pass's loads (742 M to libzstd's 706 M), L1 misses (27 M to 30 M), L2 misses
  (3.9 M to 3.8 M) and mispredictions (4.8 M to 5.4 M) are much the same as libzstd's; its
  stores are 43% more (423 M to 296 M), about 10 a sequence of the loop's spills: it holds
  11 values from one sequence to the next and 6 for the whole loop, more than x86-64's
  registers.
- **U6**: a Huffman lookup is a shift, a load, a store and a shift, as in libzstd's
  assembly loop, the steps counted from what every stream has left of its source and its
  output, with no check of its own: `x-ray` 133 M to 104 M instructions (libzstd 92 M) and
  36.8 M to 34.6 M cycles; mixed's tiles 11% fewer instructions, 1% fewer cycles.
- **U10**: a small block paid the loop's setup twice, for the source and for its padded copy,
  and its last sequence went through the slow tail and the safe writer's setup: the GitHub
  frames with their dictionary, about 23 sequences a block, 59.2 M to 57.1 M instructions and
  16.1 M to 15.0 M cycles. The padding at the stream's end is skipped in one step.

Tried and left out (one run each, U4 against U3, the others against U6):

| | | silesia `-3` instructions | cycles |
|---|---|---:|---:|
| U4 | U5 with the states stored on an error, so that LLVM computes them before the copies, not after | +4.1% | +3–6% |
| U7 | the copy of a match within 16 bytes inline | −0.7% | +1.5% |
| U8 | 8 sequences decoded ahead of their copy, each match's source prefetched, as libzstd's decoder for long windows | +24.6% | +10.1% |

U8's queue went through the stack, about 45 instructions a sequence; libzstd uses that decoder
only for windows over 16 MB, and Silesia's match sources miss L2 0.27 times a sequence.

The commits: `ac3616b` (the sequences, U10's), `e56ebae` (the Huffman streams) and
`dc41c17` (wasm's shifts, below). WebAssembly keeps its safe loops: none of this changes
what it runs, but for the padding skipped in one step and the literals' padding.
AddressSanitizer under the fuzzers (`decode`, `decode_dict`, `interop`, three minutes each)
and Miri, over a test of the raw writer against a byte at a time and the decode of the
corpus's small files (AGENTS.md), found nothing; the tests pass on arm64 (`remote-lxc`),
which runs the same loops without BMI2.

The full run, on lab2 after a restart (run 26): M26 is `main` at `abd76b0` (U0), R26 the
branch at U10. Every build decoded every workload to the same output.

| workload | M26 | R26 | libzstd | M26 ÷ libzstd | R26 ÷ libzstd |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 255.96 | 224.07 (−12.5%) | 199.74 | 1.28 | **1.12** |
| silesia `-19` | 266.48 | 228.26 (−14.3%) | 203.74 | 1.31 | **1.12** |
| silesia `--fast=4` | 160.56 | 140.50 (−12.5%) | 127.34 | 1.26 | **1.10** |
| `nci` | 23.81 | 21.25 (−10.8%) | 18.38 | 1.30 | **1.16** |
| `x-ray` | 9.82 | 9.62 (−2.1%) | 8.75 | 1.12 | **1.10** |
| the GitHub frames with their dictionary | 4.67 | 4.06 (−13.1%) | 3.63 | 1.29 | **1.12** |
| the GitHub frames | 18.67 | 17.49 (−6.3%) | 19.51 | 0.96 | **0.90** |
| mixed's tiles 2880x1800@2 | 492.56 | 471.39 (−4.3%) | 413.15 | 1.19 | **1.14** |
| the video clip | 15.00 | 15.38 (+2.6%) | 20.30 | 0.74 | **0.76** |
| terminal's tiles 2880x1800@2 | 0.93 | 0.79 (−14.9%) | 1.65 | 0.56 | **0.48** |

ms a pass natively, the median of three rounds; geomean against M26 −9.0%, and against
libzstd 1.07 for M26, **0.97** for R26. M26's spread was 0.5–7.7%. Every workload:
[`results/run26-raw.md`](results/run26-raw.md).

In wasm R26 ran 1.7% slower than M26 by geomean in the full run, silesia `-3` +3.9% (within
M26's 4.6% spread); beyond M26's spread, the video clip +8.5% (over 3.5%) and terminal's
tiles +3.2% (over 0.7%). Three more rounds of wasm alone
([`results/run26-wasm.md`](results/run26-wasm.md)) gave +0.6% by geomean; beyond M26's
spread there, silesia `-3` +2.4% (over 1.8%), the GitHub frames +2.1% (over 1.7%) and
silesia `-19` +1.2% (over 1.1%). The only change on wasm's path through the sequences was the
shifts without masks, which wasm now takes as before: with that (R27, three rounds of wasm
alone) the geomean is −0.1% against M26, every workload within M26's spread
([`results/run26-wasm-r27.md`](results/run26-wasm-r27.md)).

## 2026-11-06: the raw loops in wasm

The question: run 26's raw loops, the sequences' and the four Huffman streams', ran only
natively, wasm keeping its safe loops. How do they do in V8?

The variants, wasm, harness medians of three rounds against XM, `main` at `b16f2f8`. Each
is XM with the native code of some files compiled for wasm too:

| | | silesia `-3` | `x-ray` | the GitHub frames with their dictionary | the GitHub frames | mixed's tiles | terminal's tiles |
|---|---|---:|---:|---:|---:|---:|---:|
| XM | `main` (ms a pass) | 375.24 | 14.14 | 8.27 | 25.09 | 658.04 | 1.22 |
| XW1 | the raw loops, both | −16.6% | −21.2% | −15.3% | −1.8% | −7.9% | −12.6% |
| XS | the sequences' raw loop alone | −15.1% | −8.6% | −15.3% | −2.9% | −3.0% | −11.8% |
| XH | the Huffman streams' raw loops alone | −5.4% | −18.9% | +0.3% | +0.1% | −3.1% | −0.9% |
| XW1m | XW1, the table of pairs' shift by a multiplication, as wasm's safe loop had it | −16.9% | −16.3% | −11.6% | −1.0% | −8.9% | −14.2% |

XM's rounds on `x-ray` were 23.5% apart, the others' at most 12.4%, most under 6%. Each
half gains where its loop runs, and the two add up. In eight rounds more of XW1 and XW1m in
turn, XW1m was slower: `x-ray` 4.0%, mixed's tiles 1.6%, silesia `-3` 1.0%. The
multiplication had made the safe loop faster in V8 (the video 5–6%, on lab1), which puts a
variable shift's count in `cl`; in the raw loop the shift is faster.

The commit (`8c37fd6`): the raw loops are the only loops. Wasm's safe loops are gone, and
so are its own sequence run, its copy of a sequence and its multiplication. The ring
writer's check for room now adds in 64 bits, so that with 32-bit addresses (wasm32) it
cannot wrap for a ring ending near the top of memory. 676 lines less.

The full run, on lab2 after a restart (run 27): M27 is `main` at `b16f2f8` (XM in the
results), B27 the branch at `8c37fd6` (XB). Every build decoded every workload to the same
output.

| workload | native M27 | native B27 | wasm M27 | wasm B27 | libzstd |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 228.34 | 229.29 (+0.4%) | 379.15 | 313.47 (−17.3%) | 199.12 |
| silesia `-19` | 237.52 | 239.23 (+0.7%) | 386.99 | 329.45 (−14.9%) | 203.66 |
| silesia `--fast=4` | 143.15 | 143.73 (+0.4%) | 224.47 | 200.83 (−10.5%) | 127.08 |
| `nci` | 20.92 | 20.88 (−0.2%) | 32.42 | 28.05 (−13.5%) | 18.19 |
| `x-ray` | 9.56 | 9.57 (+0.1%) | 12.57 | 11.41 (−9.3%) | 8.85 |
| the GitHub frames with their dictionary | 4.18 | 4.12 (−1.6%) | 8.46 | 7.03 (−16.9%) | 3.62 |
| the GitHub frames | 17.85 | 17.82 (−0.2%) | 25.12 | 24.68 (−1.7%) | 19.40 |
| mixed's tiles 2880x1800@2 | 467.62 | 460.39 (−1.5%) | 665.89 | 585.95 (−12.0%) | 426.35 |
| the video clip | 15.14 | 15.09 (−0.3%) | 15.23 | 15.36 (+0.8%) | 20.45 |
| terminal's tiles 2880x1800@2 | 0.80 | 0.80 (−0.2%) | 1.22 | 1.08 (−11.4%) | 1.82 |

ms a pass, the median of three rounds; geomean against M27 natively −0.2%, in wasm
**−10.8%**. Beyond M27's spread in wasm: every workload but the video clip's raw blocks
(+0.8%, within 7.9%) and the GitHub frames (−1.7%, within 3.1%). Natively the code is the
same, and every workload is within M27's spread. In wasm B27 takes 1.33 times its native
time by geomean, where M27 took 1.50 in run 26. Every workload:
[`results/run27-wasm-raw.md`](results/run27-wasm-raw.md).

- The tests pass under Node (`wasm32-wasip1`) and on arm64 (`remote-lxc`). AddressSanitizer
  under the fuzzers (`decode`, `decode_dict`, `interop`, three minutes each) and Miri over
  AGENTS.md's tests found nothing. Both now check the loops wasm runs, on a 64-bit target.

## 2026-11-06: the sequence tables in one allocation

The question: the sequences' loop holds more values than x86-64 has registers (run 26), three
of them the tables' bases. With the three tables in one allocation, a state an index into it
and each entry's next state one too, the loop needs one base: does that take its spills away?

The variants, against XB (run 27's B27, the same code as `main` at `25377d5`), natively, a
pass's instructions and cycles (`perf stat` of the harness, `cyc.sh`, the median of three
rounds), on lab2 after a restart:

| | | silesia `-3` | silesia `--fast=4` | `nci` | `x-ray` | the GitHub frames | with their dictionary | mixed's tiles |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| XB | instructions (cycles), M a pass | 2,674 (827) | 1,666 (520) | 219 (76.3) | 104 (34.4) | 182 (66.9) | 57.1 (15.4) | 6,002 (2,592) |
| T1 | the tables in one allocation of 4 × 512 states: each kind's built table in a region of its own, the predefined and RLE tables of all three in the fourth | +1.7% (−1.5%) | +1.8% (+3.0%) | +1.6% (−0.9%) | +0.0% (+2.3%) | +1.0% (−1.5%) | +1.8% (+0.6%) | +0.8% (+1.4%) |
| T2 | T1, the next states held in an empty `asm!` before the copies, so that LLVM computes them there | +3.2% (−1.9%) | +3.6% (−1.3%) | +3.7% (+2.5%) | +0.1% (+2.6%) | +1.1% (−1.3%) | +2.1% (−1.9%) | +1.7% (+0.5%) |
| T3 | XB with T2's `asm!` alone | +6.2% (+1.4%) | +6.6% (−0.2%) | +6.0% (+0.5%) | +0.1% (+0.6%) | +2.1% (−1.0%) | +5.3% (−1.3%) | +3.8% (+0.8%) |

Each variant runs more instructions, and the cycles move by at most 3% either way, with no
direction from one workload to the next. In five rounds of harness medians, T1 against XB was
within the spread natively and in wasm. In T1's code (BMI2 build) the single base is itself
reloaded from the stack: the registers it freed went to other values. The spills are not
mostly the bases. LLVM moves the next states' computation below the sequence's copy, keeping
each table entry's next state and bit count, six values, live across it; T2's `asm!` moves
the computation back above the copy, and then the new states are spilled instead. The
patches: [`variants/T1-one-table.patch`](variants/T1-one-table.patch),
[`variants/T3-states-barrier.patch`](variants/T3-states-barrier.patch) (T2 is both).

T1 was then taken after all, for wasm (below).

## 2026-11-06: the sequences in V8, on the dictionary frames

The question: the GitHub frames with their dictionary took 1.71 times their native time in
wasm (run 27), the most of the workloads. What does V8's code spend?

A pass (XB, lab2, `perf stat`): natively 57.0 M instructions and 15.5 M cycles, in wasm
100.7 M and 26.3 M (Node's whole process over the passes run, so slightly more). Instructions
sampled (`v8prof.sh`): the sequences are 58% of the native pass, about 33 M, and 56% in wasm,
about 57 M; besides them, 6% of the wasm pass was in V8's `memory_copy_wrapper`, the call out
of the module V8 makes for a `memory.copy`, which is what a `memcpy` of unknown length
compiles to under bulk memory. A uprobe on the wrapper, with its size argument and its
caller (`perf record --call-graph dwarf` and `node --perf-basic-prof`), over the harness's
five passes of 9,114 frames (the uprobe slows it to three timed ones):

| caller | calls | sizes |
|---|---:|---|
| the sequences (`decode_sequences`) | 595 k | 415 k of 33–64 bytes, 91 k of 65–256, 46 k of 16 or less, 42 k of 17–32 |
| `read_frame_header` | 89 k | 16 or less (`read_exact` from a slice) |
| `StreamingDecoder::read` | 45 k | |
| `DecodeBuffer::read`, the output's copy | 46 k | 43 k of 257 B–4 KB |
| `decode_literals` | 31 k | 16 or less |

In the sequences the calls were `push_from_dict`'s copy of a match's bytes from the
dictionary, past the fast path's 32 or near the dictionary's end, and the padded copy of a
stream's last bytes, once a block (`BitReaderReversed::padded`).

The variants, against XB (`main` at `25377d5`'s code), wasm, a pass's instructions and
cycles (`perf stat` of Node, the median of three rounds; `cycw3.sh`, `cycw2.sh` with the
dictionary frames):

| | | the GitHub frames with their dictionary | silesia `-3` | `nci` | mixed's tiles |
|---|---|---:|---:|---:|---:|
| XB | instructions (cycles), M a pass | 100.7 (26.5) | 4,538 (1,178) | 364.8 (107.2) | 9,161 (2,951) |
| D1 | up to 256 bytes from the dictionary in chunks of 16, the last ending at the copy's end; the padded copy a fixed 22 bytes where the stream has them | −4.3% (−4.4%) | −0.6% (−0.3%) | −0.7% (−0.5%) | +0.0% (−0.6%) |
| D2 | D1, the sequence tables in one allocation (T1, above) | −5.2% (−5.3%) | −2.1% (−1.1%) | −1.8% (−1.7%) | −0.5% (−1.3%) |

`memory_copy_wrapper` went from 6.0% of the dictionary frames' pass to 1.9%. In V8's code
for the loop the three tables' bases were three values, two of them reloaded from the stack
each sequence; with one, `nci` and mixed's tiles took fewer cycles than any of XB's rounds.
Natively D2 is as in T1, more instructions in as many cycles (below). What else the profile
shows of V8's code for the loop, for later: it spills about ten values on the loop's back
edge and reloads them, the bit window among them, so that the refill's chain goes through the
stack; and a read of bits takes about ten instructions to the native build's four, V8 using
no BMI2 shifts, so every count goes through `cl`, with an `and` of 63 and moves around it.

The commits: `a9d5384` (D1, with the raw writer's test now making matches of up to 302 bytes,
1,537 of the dictionary's copies past 32 bytes) and `bd3b6d9` (T1).

The full run, on lab2 after a restart (run 28): M28 is `main` at `25377d5`, B28 the branch
at `bd3b6d9`. Every build decoded every workload to the same output.

| workload | native M28 | native B28 | wasm M28 | wasm B28 | libzstd |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 222.85 | 232.99 (+4.6%) | 317.45 | 319.64 (+0.7%) | 203.59 |
| silesia `-19` | 230.05 | 225.76 (−1.9%) | 329.92 | 341.31 (+3.5%) | 205.09 |
| silesia `--fast=4` | 140.16 | 141.54 (+1.0%) | 193.82 | 190.59 (−1.7%) | 124.61 |
| `nci` | 20.66 | 20.66 (−0.0%) | 28.24 | 28.15 (−0.3%) | 18.38 |
| `x-ray` | 9.40 | 9.45 (+0.5%) | 11.01 | 11.16 (+1.4%) | 8.68 |
| the GitHub frames with their dictionary | 4.25 | 4.03 (−5.0%) | 7.31 | 6.56 (−10.3%) | 3.69 |
| the GitHub frames | 17.68 | 18.43 (+4.2%) | 24.89 | 24.52 (−1.5%) | 19.45 |
| mixed's tiles 2880x1800@2 | 458.52 | 456.85 (−0.4%) | 593.36 | 582.75 (−1.8%) | 411.06 |
| the video clip | 15.49 | 15.04 (−2.9%) | 16.30 | 15.30 (−6.1%) | 19.97 |
| terminal's tiles 2880x1800@2 | 0.79 | 0.79 (+0.5%) | 1.05 | 1.05 (+0.0%) | 1.65 |

ms a pass, the median of three rounds; geomean against M28 natively 0.0%, in wasm −1.7%.
Beyond M28's spread in wasm: the dictionary frames (−10.3%, over 5.8%), the rest within it.
Natively the GitHub frames' +4.2% is just past M28's 4.0% spread; a pass's cycles, counted
after the run (`cyc.sh`, three rounds), are within 1.3% of M28's on every workload counted,
the GitHub frames' +0.0%, silesia `-3`'s −0.2%, the instructions 0.5–4.2% more. Every workload:
[`results/run28-dict-copy.md`](results/run28-dict-copy.md).

- The tests pass natively, under Node (`wasm32-wasip1`, with and without default features)
  and on arm64 (`remote-lxc`). AddressSanitizer under the fuzzers (`decode`, `decode_dict`,
  `interop`, three minutes each) and Miri over AGENTS.md's tests found nothing.

## 2026-11-07: a new baseline, on a rebuilt lab2 and reproducible corpora

No change to the decoder. Its two reasons:

- **lab2 was rebuilt** on 2026-11-07, on a new address and a new processor: an
  `AMD EPYC-Milan Processor` (AVX2 and BMI2, no AVX-512), 8 vCPUs (4 cores, 2 threads),
  15 GB, Debian 13, kernel 6.12.115, swap off; from an `AMD EPYC-Rome Processor`, 4 vCPUs and
  8 GB. rustc 1.99.1, binaryen 120 and Node 26.13.0 (V8 14.6) as before; each run still
  pinned to vCPU 2. Timings from before run 29 are not comparable with later ones.
- **The Silesia corpus was not reproducible.** Its tar was made by `tar` of `unzip`'s
  output, so its headers held the user's name and times in the time zone of the `unzip`,
  and that tar went with the old lab2: another machine could not make the same `.zst` files.
  [`make-corpus.sh`](harness/make-corpus.sh) now makes every general corpus on any machine,
  checked against [`corpus.sha256`](harness/corpus.sha256) ("Running it again", below).
  Its Silesia `.zst` files are 289–475 bytes from those of runs 16–28 (66,217,517 bytes at
  `-3`, from 66,217,806); every other file is the same as in those runs, and lab2 and this
  repository's working machine made the same bytes.

The run (run 29), on lab2 after a restart: M29 is `main` at `ff75fa7` (v0.9.2-alpha.12,
run 28's B28 merged), with libzstd. Every build decoded every workload to the same output,
and the tiles and frames to run 28's hashes.

| workload | native M29 | wasm M29 | libzstd | native ÷ libzstd | wasm ÷ native |
|---|---:|---:|---:|---:|---:|
| silesia `-3` | 203.09 | 276.64 | 176.50 | 1.15 | 1.36 |
| silesia `-19` | 208.97 | 278.37 | 187.56 | 1.11 | 1.33 |
| silesia `--fast=4` | 132.81 | 173.25 | 116.25 | 1.14 | 1.30 |
| `nci` | 19.27 | 25.30 | 17.09 | 1.13 | 1.31 |
| `x-ray` | 8.79 | 9.99 | 7.23 | 1.22 | 1.14 |
| the GitHub frames with their dictionary | 3.49 | 5.43 | 3.14 | 1.11 | 1.55 |
| the GitHub frames | 15.28 | 21.69 | 16.48 | 0.93 | 1.42 |
| mixed's tiles 2880x1800@2 | 426.10 | 524.85 | 345.54 | 1.23 | 1.23 |
| the video clip | 13.69 | 13.87 | 19.47 | 0.70 | 1.01 |
| terminal's tiles 2880x1800@2 | 0.66 | 0.88 | 1.36 | 0.49 | 1.33 |
| **geomean** | | | | **0.99** | **1.29** |

ms a pass, the median of three rounds. Every workload:
[`results/run29-lab2-rebuilt.md`](results/run29-lab2-rebuilt.md).

- The rounds were closer than on the old lab2: natively 0.3–3.1%, but the dictionary frames
  6.5%; in wasm 0.9–4.3%; libzstd 0.4–1.6%, but `x-ray` 19.9% (6.84, 7.23 and 8.28 ms), so
  that `x-ray` takes ruzstd-decompress 1.06–1.29 times libzstd's time, 1.22 by the median.
- Against run 28's B28, the same code: every figure is 2.5–18.4% lower (libzstd 2.5–17.9%,
  ruzstd-decompress 6.2–17.1% natively and 9.1–18.4% in wasm), from the processor, and
  the ratios moved. Natively the geomean went from 0.97 to 0.99 times libzstd's time:
  libzstd gained more on mixed's tiles (1.11 to 1.23) and, less surely, `x-ray` (1.09 to 1.22), and
  ruzstd-decompress more on the video clip (0.75 to 0.70). In wasm it went from 1.33 to 1.29
  times the native time.

## 2026-11-08: decoding on several threads

What is new: `FrameDecoder::decode_all_parallel`, behind the `rayon` feature, decodes a whole
input on the threads of the rayon pool it is called in
([`src/decoding/parallel.rs`](../../src/decoding/parallel.rs)). In WebAssembly a thread is a
worker on a shared memory: [`examples/wasm-threads`](../../examples/wasm-threads) is a module
and the JavaScript that runs it on a pool of workers, in a browser or in Node.

- **Frames are independent**, so an input of several is decoded a run of frames to each job.
- **A frame's blocks** depend on each other in two ways: a block may repeat the tables of the
  one before it, and its matches copy from the blocks before it. Neither stops a block's
  literals (its Huffman streams) and its sequences (its FSE streams) from being decoded apart,
  once its tables are known. So a large frame, when there are fewer frames than threads, is
  decoded in three steps:
  1. the tables, block after block;
  2. a batch of blocks' literals and sequences into buffers of their own, side by side;
  3. those sequences executed in order on one thread, beside the next batch's first two steps.

  A batch is about 128 KB of input to each thread, and an eighth of the frame at most. Its
  stored literals and sequences take at most about 2 MB to each thread, for a block of a few
  bytes can hold a block's worth of either. That cap came after run 32. On the WSL
  workstation, 6 threads, it moved `x-ray`, `nci`, silesia `-3` and `--fast=4` by no more
  than their noise. A frame smaller than two batches is decoded as on one thread.
- **For that split, the one-thread code was divided, not copied:**
  - The sequences' loop is generic over where each sequence goes: executed at once, as before,
    or stored (`SequenceSink`).
  - The literals' table step (the Huffman table and its table of pairs) is apart from the
    streams' decoding.
  - A compressed block's sections are parsed by one function for both paths.

### Variants and runs

All on lab2 after a restart. **A** is `main` at `ad2f5ab`. **B** is the branch's first version.
**C** is B with the table of pairs built in a BMI2 build again, as it was inside the streams'
loop, and the batches capped at an eighth of the frame.

- **Run 30**: A and B on one thread, all 10 workloads, three rounds
  ([`results/run30-one-thread.md`](results/run30-one-thread.md)).
- **Run 31**: A and B, five rounds, on the four smallest workloads
  ([`results/run31-check.md`](results/run31-check.md)). Then B on several threads
  ([`results/run31-threads.md`](results/run31-threads.md)).
- **Run 32**: the same for C ([`results/run32-check.md`](results/run32-check.md),
  [`results/run32-threads.md`](results/run32-threads.md)).

### On one thread

Every build decoded every workload to the same output.

| | native | wasm |
|---|---:|---:|
| B, run 30, geomean of the 10 workloads | +0.4% | +0.2% |
| B, run 31, `x-ray` | +3.0% | +0.7% |
| C, run 32, `x-ray` | +2.8% | +0.9% |
| C, run 32, the dictionary frames | +1.6% | +2.9% |

Every other workload was within ±1.9% of A, inside or near its spread.

`x-ray` is the one that moved beyond its spread natively. Its spread in run 32, 38.5% for A,
is one round, round 3 at 12.17 ms; A's other four took 8.78–8.84 ms and C's five 9.01–9.09
ms, so the +2.8%, a median of rounds, does not hang on it. `perf stat` over its passes (lab2,
vCPU 2) does not count more work:

| | instructions a pass | cycles a pass |
|---|---:|---:|
| A | 104.9 M | 33.3–33.7 M |
| C | 104.5 M | 34.1–34.3 M |

C runs 0.4% fewer instructions in 2–3% more cycles: the hot loops sit elsewhere in the
binary, not more work done. The table of pairs' build in a BMI2 build again (C) did not move
it.

### On several threads (run 32)

[`harness/threads.sh`](harness/threads.sh) times `decode_all_parallel` on 1 to 8 threads:

- **natively** by [`examples/decode_parallel.rs`](../../examples/decode_parallel.rs), built
  with build.sh's profile;
- **in wasm** by the example's `bench.mjs`, the module on a pool of Node 26's workers.

A figure is the median of decodes repeated for 2 s and at least 3 times, unpinned. In wasm it
is the decode alone, in the module, without the copies of the input in and the output out.

It adds a workload: **Silesia in frames of 4 MiB**, `silesia.tar` cut into 4 MiB pieces, each
`zstd -3`'d on its own and the frames put back to back (51 frames, 66.5 MB). That is what
`pzstd` or zstd's seekable format write, and decodes to the same tar.

ms a decode, then how many times faster than on one thread.

| workload | native, 1 | native, 4 | native, 8 | wasm, 1 | wasm, 4 | wasm, 8 |
|---|---:|---:|---:|---:|---:|---:|
| Silesia in frames of 4 MiB | 218.96 | 76.84 (2.85×) | 46.17 (4.74×) | 289.30 | 102.63 (2.82×) | 63.98 (4.52×) |
| the GitHub frames | 15.96 | 5.44 (2.93×) | 3.52 (4.53×) | 22.73 | 7.36 (3.09×) | 4.59 (4.95×) |
| the GitHub frames with their dictionary | 4.61 | 2.90 (1.59×) | 2.46 (1.87×) | 6.19 | 3.37 (1.84×) | 2.52 (2.46×) |
| `x-ray` | 10.03 | 5.83 (1.72×) | 4.03 (2.49×) | 11.50 | 5.99 (1.92×) | 4.92 (2.34×) |
| `nci` | 23.55 | 17.07 (1.38×) | 16.43 (1.43×) | 27.62 | 18.71 (1.48×) | 17.76 (1.56×) |
| silesia `-3` | 221.30 | 157.47 (1.41×) | 131.76 (1.68×) | 297.13 | 190.55 (1.56×) | 174.20 (1.71×) |
| silesia `-19` | 233.35 | 159.12 (1.47×) | 140.02 (1.67×) | 313.77 | 206.91 (1.52×) | 179.93 (1.74×) |
| silesia `--fast=4` | 145.20 | 127.58 (1.14×) | 94.88 (1.53×) | 186.84 | 152.72 (1.22×) | 117.88 (1.59×) |

2, 3 and 6 threads are in the results.

- **Independent frames scale**: 4.5–5.0 times as fast on 8 threads, on lab2's 4 cores of 2
  threads each, natively and in wasm alike. The dictionary frames gain less: they are the
  quickest per byte, so what stays on one thread weighs more. That is finding the frames, and
  putting the runs' outputs together.
- **One frame gets 1.4–2.5 times as fast.**
  - Its sequences' execution stays on one thread. On one thread it was about half of a decode
    (the WSL workstation, silesia `-3`: tables 25 ms, literals and sequences 148 ms, execution
    177 ms).
  - Execution includes the window's copy into the output and the checksum's hash, about a
    fifth of it (40 of 186 ms there).
  - Execution also slows as the other threads work beside it, from 177 ms alone to 190, 208
    and 230 ms on 2, 4 and 6 threads there.
  - `x-ray`, nearly all Huffman literals, gains most. The Silesia tars, most of whose output
    comes from matches, gain least, `--fast=4`'s raw literals least of all.
- **2 and 3 threads move from run to run on lab2.**
  - Run 31's and run 32's figures differ by up to a third. For one thread against two, on
    the GitHub frames and `--fast=4`, pinning to vCPUs 0 and 2 (separate cores, by `lscpu`)
    gave what pinning to 0 and 1 (one core) did. The VM's vCPUs may not be the host's.
  - Two threads were slower than one now and then: the dictionary frames 0.72× natively in
    run 32 (1.24× in wasm), `--fast=4` 0.97× natively on 3 threads.
  - From 4 threads up they differ less: up to a quarter on the dictionary frames, the
    quickest workload, and up to 14% on the rest. The split frames' batches changed between
    B and C, so not all of that is noise.
- **In wasm the threads gain about as much as natively**: within 0.25× of it on 8 threads on
  all but the GitHub sets, which gain more in wasm (4.95× to 4.53×, and 2.46× to 1.87× with
  the dictionary). The module's threads are Node's workers (`worker_threads`) on a shared
  `WebAssembly.Memory`. In headless
  Chrome 155 on the WSL workstation, silesia `-3` took 562 ms on one thread and 298 ms on six,
  the GitHub frames 43 ms and 13 ms.

## Running it again

### On any machine

Needs: Rust stable through rustup (a variant's `rust-toolchain.toml` is honoured) with the
`wasm32-wasip1` target, binaryen's `wasm-opt` (120 for figures like these), Node with
`node:wasi` (26.13.0 here), zstd 1.5.7, GNU tar (`TAR=gtar` on macOS), curl, unzip and
Python 3. From the repository's root, with any two directories for the corpora, builds and
results:

```sh
export SCRATCH=$HOME/ruzstd-bench TARGETS=$HOME/ruzstd-bench-target
docs/benchmarks/harness/make-corpus.sh                   # the corpora, checked by SHA-256
mkdir -p "$SCRATCH/variants/M" "$SCRATCH/results"
git archive HEAD | tar -x -C "$SCRATCH/variants/M"       # a variant: a commit's tree
docs/benchmarks/harness/build.sh M Z                     # it, native and wasm, and libzstd
docs/benchmarks/harness/run.sh "$SCRATCH/results/run.jsonl" M Z
python3 -I docs/benchmarks/harness/summary.py "$SCRATCH/results/run.jsonl" M
```

(`summary.py` needs only Python's standard library; the maintainer runs it as
`uv run --python 3.13 python -I summary.py ...`.)

- **The settings** are environment variables, read by every script from
  [`env.sh`](harness/env.sh): `SCRATCH`, `TARGETS`, `CARGO`, `WASM_OPT`, `NODE`, `PIN` and
  `ROUNDS`. Unset, they are the maintainer's layout (below), so the commands above without
  the `export` are how the recorded runs were made.
- **What runs without the maintainer's files**: 8 of the 10 workloads. The video clip
  cannot be downloaded again (`make-corpus.sh` reads it from `VIDEO`, and leaves it and
  its check out without it), and wlshare's tiles come from frame samples that are not public.
  `run.sh` runs what is in `$SCRATCH/corpus` and `$SCRATCH/tiles` and leaves the rest out.
- **Each process is pinned** to one CPU, `taskset -c 2` by default on Linux with 3 CPUs or
  more and nothing elsewhere (`PIN=` for none, `PIN="taskset -c 5"` for another). Time on a
  quiet machine: nothing else running, restarted first.
- **Comparing**: a figure compares with another of the same run, on the same machine. Across
  machines only ratios carry, and those loosely: ruzstd-decompress's time over libzstd's,
  natively, and the wasm build's over its own native time. Run 29's are in
  [`latest.md`](latest.md).
- **Checks**: `make-corpus.sh` stops on a file that is not the expected one, `run.sh` on a
  build that fails a workload, and `summary.py` prints `outputs differ:` with the workloads
  whose builds decoded to different output (`none` when all agree).

A run of one variant and libzstd, three rounds, took about six minutes on lab2 (run 29); `ROUNDS=1` is a quick check that everything builds and decodes.

**On several threads**: `docs/benchmarks/harness/threads.sh "$SCRATCH/results/threads.jsonl"`,
then `harness/threads-summary.py` on its output (run 32). It needs the nightly of
`examples/wasm-threads` (rustup installs it) and `wasm-bindgen-cli` 0.2.130, and makes
`corpus-threads/silesia.tar.l3.4M-frames.zst` from `corpus-src/silesia.tar`, outside
`corpus/`, which `run.sh` runs all of. It takes about four minutes.

### What the scripts do

1. **The general corpora**: [`make-corpus.sh`](harness/make-corpus.sh) `[SCRATCH]` writes
   `corpus/` from `corpus-src/`, downloading what can be downloaded, and checks every file
   against [`corpus.sha256`](harness/corpus.sha256), so any machine with zstd 1.5.7 and GNU
   tar makes the same bytes (lab2 and the maintainer's workstation did, 2026-11-07, the
   second in a path with a space). What it makes:
   - From `silesia.zip` (SHA-256 `0626e25f…63c77af`): the Silesia corpus as one tar (its 12
     files in the zip's order, owner 0, mode 0644, mtime 0, GNU format), `zstd -3`, `-19`
     and `--fast=4`; `silesia-x-ray.l1.zst` (`zstd -1` of `x-ray`) and `silesia-nci.l3.zst`
     (`zstd -3` of `nci`).
   - From zstd's
     [`github_users_sample_set.tar.zst`](https://github.com/facebook/zstd/releases/download/v1.1.3/github_users_sample_set.tar.zst)
     (SHA-256 `e35074ab…89fc0de1`): its files in name order (case-insensitive, lower case
     first), each `zstd -3`'s frame back to back, `github.frames`; a dictionary trained on
     them (`zstd --train`, 110 KB), `github-dict.dict`, given its files in the order of
     [`github-dict-order.txt`](harness/github-dict-order.txt), since the training depends
     on it; and `github-dict.frames` (`zstd -3 -D`). `run.sh` takes `NAME.frames` in the
     `frames` mode, with `NAME.dict` if there is one.
   - The first minute of `Multicolored_Explosions-4k.webm` (4K VP9 video, no audio), cut
     without re-encoding (`ffmpeg -t 60 -c copy -an`: 129,927,535 bytes, SHA-256
     `ba8a6a586890f80bd78a3a5180644d867882f7c5370c63573b5aec582ed66e54`), `zstd -T0 -3`.
     The video is already compressed, so zstd stores it as it is, in raw blocks: it
     measures that path. At `-19` it is the same, so it is left out.

   What would make the bytes differ, and what the script pins: a tar's headers hold the
   owner's name, the files' times (from a zip, in the time zone of the `unzip`) and their
   order (a locale's sort); `zstd --train -r` takes its files in the directory's order; and
   zstd writes other bytes for input from a pipe, of no known size. The number of threads
   does not change its output (`-T1` to `-T0` alike; `--single-thread` would). The Silesia
   tar of runs 16–28 was made on lab2 without these and went with that machine: its `.zst`
   files are 289–475 bytes from the script's, which run 29 measures from. Every other file
   is the same as in those runs. `enwik9` at both levels was a workload until 2026-11-04.
   It is text, which Silesia has, and it was the slowest workload.
2. **The tiles**, which need wlshare's frame samples (not public): an rsync of
   `../wlshare` at `aff1945` with
   [`wlshare-tile-dump.patch`](harness/wlshare-tile-dump.patch) applied, its
   motion-bench built (`MOTION_BENCH`), then [`dump-tiles.sh`](harness/dump-tiles.sh)
   `[OUT]` over the samples (`SAMPLES`), which writes `SCENARIO-SIZE-PICTURE.tiles`: a byte
   for the kind (0 planar, 1 still), `w` and `h` as u16, the length as u32, little endian,
   then the body. The bytes do not hang on the processor. Two are benchmarked, in
   `$SCRATCH/tiles/`: `terminal-2880x1800@2-lossless.tiles`, and
   `mixed-2880x1800@2-lossless-200M.tiles`, the first 200,000,000 bytes of the mixed
   session's (`head -c 200000000`; the harness drops the last, cut, body).
3. **The variants**: a tree each in `$SCRATCH/variants/NAME` (a `git archive` of the
   commit, with the patches above), then [`build.sh`](harness/build.sh) `NAME …` on the
   timing machine, with the harness of the variant's own tree where it has one, and the
   variant's `rust-toolchain.toml` if it has one, else the default toolchain; `build.sh Z`
   builds libzstd, [`libzstd/`](harness/libzstd) at its `Cargo.lock`'s zstd-sys (1.5.7),
   into `bin/Z-native`.
4. [`run.sh`](harness/run.sh) `OUT.jsonl VARIANT…` (wasm through
   [`run-wasi.mjs`](harness/run-wasi.mjs); `PLATFORMS=native` for native alone), then
   [`summary.py`](harness/summary.py) `OUT.jsonl [BASE]` for the tables against BASE, A
   by default.

### The maintainer's rig

The defaults follow `../devtools`' layout, everything under `~/codes/staging-area`:
`ruzstd-decompress-scratch/` (`SCRATCH`), `ruzstd-decompress-target/` (`TARGETS`) and
Node in `node/`. Timings come from lab2, a VM kept for them (its processor in each run's
section), restarted before a run, pinned to vCPU 2. The video clip, the tiles (all of them
as dumped, and the two cuts) and every corpus file are on the maintainer's backup drive,
`ruzstd-decompress-artifacts/`, with their `SHA256SUMS`; `make-corpus.sh`'s `VIDEO` and
`dump-tiles.sh`'s defaults point there.

### V8's tiers

In wasm, V8 runs a function in its unoptimized tier (Liftoff) until its optimized code
(TurboFan) is ready, and a call already running stays in it. The harness's checked
passes, before the timed ones, take that: on mixed's tiles the first, a call of `replay`
with the sequences' loop inlined, runs in Liftoff to its end, 2.1 s, about three times a
TurboFan pass, and every pass after it is TurboFan's (lab2, 2026-11-06, `perf` samples
by time of the harness with one checked pass). The timed passes are not affected: with
`--no-liftoff` the medians were slower, not faster (the middle of three runs: mixed's
tiles 1.3%, the GitHub frames 1.1%, terminal's tiles 3.3%). A count over the whole
process, such as `perf stat` of Node, does include that pass: on mixed's tiles, which
time three passes, it is no measure of one. The tiles and frames modes run two checked
passes since `ce55cf2`, the frame mode one.
