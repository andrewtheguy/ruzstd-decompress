# Decode benchmarks

What the decoder costs, and what each change to it bought. The crate is a
general zstd decoder, so the workloads are standard corpora first: Silesia,
whole and by file, at several levels, and zstd's own small-frame and dictionary
set. A workload of its own is added only for what none of them covers. wlshare's
lossless tiles, which zDesk's page decodes in WebAssembly
(`zdesk/frontend/wasm/egfx`), are kept as a check on a real user, two of them
since 2026-11-05 (see "the workloads, for a generic decoder" below); the runs
before that date measured 14.

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

## Running it again

The scripts assume `../devtools`' layout, everything under
`~/codes/staging-area`; `ruzstd-decompress-scratch/` holds the corpora, variants
and results.

1. **The tiles** (on any machine; the bytes do not hang on the processor): an
   rsync of `../wlshare` at `aff1945` with
   [`wlshare-tile-dump.patch`](harness/wlshare-tile-dump.patch) applied, its
   motion-bench built into `wlshare-tiledump-target/`, then
   [`dump-tiles.sh`](harness/dump-tiles.sh), which writes
   `SCENARIO-SIZE-PICTURE.tiles`: a byte for the kind (0 planar, 1 still), `w`
   and `h` as u16, the length as u32, little endian, then the body.
2. **The general corpora**: the Silesia corpus as one tar, `zstd -3` and
   `zstd -19`, and the first minute of `Multicolored_Explosions-4k.webm` (4K
   VP9 video, no audio), cut without re-encoding (`ffmpeg -t 60 -c copy -an`:
   129,927,535 bytes, SHA-256
   `ba8a6a586890f80bd78a3a5180644d867882f7c5370c63573b5aec582ed66e54`),
   `zstd -T0 -3`, into `corpus/`. The video is already compressed, so zstd
   stores it as it is, in raw blocks: it measures that path. At `-19` it is the
   same, so it is left out. `enwik9` at both levels was the third until
   2026-11-04. It is text, which Silesia has, and it was the slowest workload.
   Since 2026-11-05 (zstd 1.5.7): `silesia.tar.fast4.zst` (`zstd -T0
   --fast=4`), `silesia-x-ray.l1.zst` (`zstd -1` of `x-ray`) and
   `silesia-nci.l3.zst` (`zstd -3` of `nci`), from `silesia.zip` (SHA-256
   `0626e25f…63c77af`); and from zstd's
   [`github_users_sample_set.tar.zst`](https://github.com/facebook/zstd/releases/download/v1.1.3/github_users_sample_set.tar.zst)
   (SHA-256 `e35074ab…89fc0de1`), its files in name order, each `zstd -3`'s
   frame back to back: `github.frames`, and with `zstd --train -r` of them,
   `github-dict.dict`, `github-dict.frames` (`zstd -3 -D`). `run.sh` takes
   `NAME.frames` in the `frames` mode, with `NAME.dict` if there is one. Of the
   tiles, only `mixed-2880x1800@2-lossless-200M.tiles` and
   `terminal-2880x1800@2-lossless.tiles` stay in `tiles/`; the rest are in
   `tiles-retired/`.
3. **The variants**: a tree each in `variants/A`… (a `git archive` of the
   commit, with the patches above), then [`build.sh`](harness/build.sh) `A B …`
   on the timing machine, with the harness of the variant's own tree where it has
   one; [`libzstd/`](harness/libzstd) is built the same way
   into `bin/Z-native`.
4. [`run.sh`](harness/run.sh) `OUT.jsonl VARIANT…` (wasm through
   [`run-wasi.mjs`](harness/run-wasi.mjs); `PLATFORMS=native` for native alone),
   on a quiet machine, then [`summary.py`](harness/summary.py) `OUT.jsonl
   [BASE]` for the tables against BASE, A by default (`uv run --python 3.13
   python -I summary.py OUT.jsonl M`). `build.sh` uses a variant's
   `rust-toolchain.toml` if it has one, else the default toolchain.
