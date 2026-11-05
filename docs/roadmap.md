# Roadmap

What to make faster next, most promising first, as of 2026-11-05 (after the
sequences with the buffer's state in locals, run 18 of
[`benchmarks/README.md`](benchmarks/README.md)). Figures from run 18 and later are
from lab2, on the generic workloads (Silesia, the GitHub sample set, wlshare's tiles
as a check); the older ones, from `perf` profiles on lab1 on the tiles, say so.
zDesk decodes in WebAssembly. The crate stays free of unsafe code.

Natively, by geomean over the 10 workloads, ruzstd-decompress takes 1.20 times
libzstd's time: 1.33–1.40 times on Silesia but `x-ray` (1.19), 1.67 on the GitHub
frames with their dictionary, 1.24 without. It is faster on the raw blocks of the
video clip (0.86 times libzstd's time) and terminal's tiles (0.65). In wasm, the
workloads take 1.2–1.4 times their native time, the dictionary frames 1.6; the video
clip's raw blocks take 0.92 times.

## 1. The sequences, again

On silesia `-3` (lab2, before run 18), the sequences' decoding and execution were
3.1 G instructions a pass to libzstd's 1.7 G, at a higher rate a cycle: the gap is
the instructions. Natively, the fast loop now keeps the ring's and the executor's
state in locals (4–6% off Silesia); in WebAssembly the same was 6–12% slower in V8,
so that build executes them as before (run 18 lists what was tried). What is left:
- **WebAssembly**: V8's code for the loop spills either way. A form of the loop it
  takes better is not known; the forms tried are in run 18.
- **The chunk copies' bounds checks**: three a chunk of the literals and of the
  match, that the copy could check once for the sequence. Slicing each chunked copy
  once was tried on lab1 (no faster natively, mixed and silesia 4–5% slower in wasm),
  before the buffer's state was in locals.
- The 2–11% of sequences the fast copy does not take (lab1, tiles): matches from
  less than 16 bytes back (a repeated pattern was tried, and was no faster), and
  those near the end of the initialised buffer.
- libzstd's sequence loop is built for BMI2 when the processor has it (its
  `_bmi2` functions), whose shifts take no `cl` register: the take of each field of
  bits is 7 instructions here. The crate cannot pick a function at run time without
  unsafe code; a build for `x86-64-v3` would show what it is worth.

## 2. What is left of the table builds

On uncover (lab1), which builds 847 sequence tables and 88 Huffman tables a pass:
- The sequence tables' build, 13% natively and 12% in wasm. Natively the loop
  over the states is bound by its instructions, about 3.6 ns a state; the
  tries at its chain of counts through memory were slower. In wasm, V8's code
  for it costs the same whether the symbols are walked or not, so what it
  spends is V8's: worth reading its code for the loop's spills and checks.
  Each build also zeroes about 1.5 KB of arrays on the stack.
- Reading the tables' descriptions (`read_distribution`), 6%: still a chain
  from one value to the next, as in libzstd.
- The Huffman tables' build, about 4%, half of it sorting the symbols by
  length.
- Not keeping a table that is the same as the last: no FSE-compressed
  description in terminal's, uncover's or browser's tiles is the same as the
  one before it of its kind (counted 2026-11-05, with each table keeping the
  bytes it was built from); the repeats counted before were of the predefined
  tables, which are kept already.

## 3. The Huffman loops in wasm

The table of pairs' loop now moves its windows on by a multiplication in
wasm, and each stream finishes from the table of pairs: video 5–6% faster in
wasm (lab1). In V8's code for the loop, two of the four streams' windows are still
in memory, and the four output positions still go through a vector each round
of lookups (packing them into one `u32` avoided that, and was slower). Two
streams to a loop, which fits in the registers, was much slower (video 15% in
wasm). What is left: fewer lookups a refill, or the single symbols' table,
which the small tiles and silesia mostly use, with the multiplication too
(its entries would need 4 bytes, not 2).

## 4. Smaller

- The frame checksum (XXH64, the `twox-hash` crate) is made only for frames
  with a checksum now; of the workloads, only the general corpora's have one
  (the `zstd` command's default), none of the tiles'.
- The harness's own function (`decode_bench`'s `replay`) is mostly decoder
  code inlined into it: natively on uncover, 14.7% of its 15.3% is the
  sequences' decoding (`decode_sequences`), which counts with item 1. In V8
  it is about 10% of video and runs in the baseline tier (Liftoff)
  throughout, being called once a pass: what of that is the decoder's is not
  yet known.
- The ring buffer's lazy initialisation cost silesia 2–4% in wasm in the
  quick checks before (not in a full run), with nothing in V8's profile to
  show where.
