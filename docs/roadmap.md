# Roadmap

What to make faster next, most promising first, as of 2026-11-05 (after the
table of pairs' rounds, run 22 of
[`benchmarks/README.md`](benchmarks/README.md)). Figures from run 18 and later are
from lab2, on the generic workloads (Silesia, the GitHub sample set, wlshare's tiles
as a check); the older ones, from `perf` profiles on lab1 on the tiles, say so.
zDesk decodes in WebAssembly. The crate stays free of unsafe code.

Natively, by geomean over the 10 workloads, ruzstd-decompress takes 1.07 times
libzstd's time: 1.29–1.31 times on Silesia but `nci` (1.25) and `x-ray` (1.09), 1.25 on
the GitHub frames with their dictionary, 1.19 on mixed's tiles. It is faster on the GitHub
frames without it (0.96 times libzstd's time), the raw blocks of the video clip (0.78) and
terminal's tiles (0.62). In wasm, the workloads take 1.3–1.45 times their native time, the
dictionary frames 1.8; the video clip's raw blocks take the same.

## 1. The sequences, again

On silesia `-3` (lab2, before run 21), the sequences' decoding and execution were
3.1 G instructions a pass to libzstd's 1.7 G, about 215 a sequence, 44 of them loads and
stores of the stack: the loop's state does not fit the registers. Natively, the fast loop
keeps the ring's and the executor's state in locals (4–6% off Silesia), and copies a
short sequence in two copies of 16 bytes, without loops or most of their checks (run 21:
silesia `-3` 4–10% faster, its mispredictions near libzstd's); in WebAssembly the locals
were 6–12% slower in V8, so that build executes them as before (run 18 lists what was
tried). What is left:
- **WebAssembly**: V8's code for the loop spills either way. A form of the loop it
  takes better is not known; the forms tried are in run 18. Since #23 it runs 1.4% more
  instructions on silesia `-3` than before (75 M a pass, about 5 a sequence), not more of
  the stack's loads and stores: V8's code for the loop as #23 wrote it (run 23).
- **The registers**: the loop's 44 loads and stores of the stack a sequence. Decoding a
  batch into an array before executing it, by a function each, and a table's entry
  packed into a `u64`, were both slower (run 21): the decoding alone took 117–119
  instructions a sequence, the six fields' takes 48 of them.
- **The longer sequences' copies**, natively: those with over 16 literals or a match over
  16 bytes, or its source within 16 bytes of the tail, about a quarter on silesia, still
  go by chunks with three or four bounds checks each. Taking up to 32 of each the same
  way made the loop's spills worse (run 21's S3). In wasm, every sequence is copied that
  way. Natively they are not worth more (counted 2026-11-05 on lab2, after run 21): about
  10 of the 245 instructions a sequence (callgrind), 2–3% of the cycles sampled. Of the
  236 M cycles a pass ruzstd trails libzstd by on silesia `-3`, 225 M are the sequences',
  and it is the instruction count, 3,625 M to 2,408 M a pass (`perf stat`), at more
  instructions a cycle (3.9 to 3.2); the cache misses, the loads stalled on a store and the
  TLB misses are much the same as libzstd's. The hottest instruction in both is the match
  source's load (17% of the cycles, libzstd's 14%). The rest of the 80 instructions more a
  sequence are the six fields' takes, a third the cost in libzstd's BMI2 build of its loop
  (chosen at run time, which needs unsafe code), and the checks and the spills.
- The sequences the fast copy does not take, 4% on silesia (run 21): matches from less
  than 16 bytes back (a repeated pattern was tried on lab1, and was no faster), and
  matches back across the ring's wrap, at about 197 instructions each in the slow path.
  Taken by the fast loop's writer, or by one of the slow path's, they were no faster
  as the harness decodes; decoded by `read_to_end`, which leaves twice as many of them,
  the loop's writer ran 2% fewer instructions.
- **The dictionary frames in wasm** take 1.8 times their native time: there the fast
  copy takes no match into the dictionary, which the slow path takes (run 20; the check
  for one in the loop made V8's code 2% slower on silesia).
- Not BMI2: a build for `x86-64-v3`, whose shifts take any register, ran 7% fewer
  instructions on Silesia in 0.5–2% more cycles (run 21). What libzstd's lead is, in
  cycles, at much the same mispredictions, is not yet known.

## 2. What is left of the table builds

On the workloads benchmarked now, the table builds are a small part of what ruzstd
trails libzstd by (lab2, 2026-11-05, after run 21, cycles sampled a pass): on mixed's
tiles, of 2,408 M to libzstd's 2,037 M, the Huffman tables are 108 M to 64 M and the
sequence tables 68 M to 59 M, against the sequences' 767 M to 615 M and the Huffman
decoding's 754 M to 643 M (the table of pairs' loop about 650 M, libzstd's assembly loop
for its pairs about 540 M); on the GitHub frames, where ruzstd is ahead (70 M to 73 M),
the Huffman tables are 25 M to 31 M and the sequence tables 4 M to 5 M; with their
dictionary the tables are the dictionary's, and the gap (18 M to 14 M) is the sequences'.
What follows was measured on uncover's tiles, no longer benchmarked.

On uncover (lab1), which builds 847 sequence tables and 88 Huffman tables a pass:
- The sequence tables' build, 13% natively and 12% in wasm. Natively the loop
  over the states is bound by its instructions, about 3.6 ns a state; the
  tries at its chain of counts through memory were slower. In wasm, V8's code
  for it costs the same whether the symbols are walked or not, so what it
  spends is V8's: worth reading its code for the loop's spills and checks.
  Each build also zeroes about 1.5 KB of arrays on the stack.
- Reading the tables' descriptions (`read_distribution`), 6%: still a chain
  from one value to the next, as in libzstd.
- The Huffman tables' build was rewritten in run 19 (the GitHub frames 24%
  faster natively); on those frames it is now a third of the pass, over a third of that
  reading the FSE-coded weights, a chain from one state to the next as in
  libzstd.
- Not keeping a table that is the same as the last: no FSE-compressed
  description in terminal's, uncover's or browser's tiles is the same as the
  one before it of its kind (counted 2026-11-05, with each table keeping the
  bytes it was built from); the repeats counted before were of the predefined
  tables, which are kept already.

## 3. The Huffman loops

Natively, the four streams' loop from the table of pairs takes four refills a round
between its checks (run 22: `x-ray` 8% and mixed's tiles 5% fewer cycles). On `x-ray` it
runs about 15.8 instructions a lookup, libzstd's assembly loop about 9: the lookup's own
11 or so, a third of them the stack's, the four windows, output chunks, their offsets and
the table being more than the registers hold; and each refill's 16, with its bounds
checks. An entry of separate bytes, each loaded on its own, took 1 instruction a lookup
fewer and 14% more cycles on `x-ray`; the output staged on the stack and copied out each
round, 6% more (run 22 lists what was tried).

In wasm, the table of pairs' loop moves its windows on by a multiplication,
and each stream finishes from the table of pairs: video 5–6% faster in
wasm (lab1). Rounds of two refills or more were 10–17% slower in V8 (run 22). In V8's code for the loop, two of the four streams' windows are still
in memory, and the four output positions still go through a vector each round
of lookups (packing them into one `u32` avoided that, and was slower). Two
streams to a loop, which fits in the registers, was much slower (video 15% in
wasm). What is left: fewer lookups a refill. The single symbols' table, which
the small tiles and silesia mostly use, is no faster with the multiplication
(lab2, 2026-11-06, a pass's instructions and cycles in wasm). Its entries kept
at 2 bytes, it took the multiplier from a table of 16 powers of two. In both
loops, the GitHub frames took 18% more cycles: their one stream's loop waits
on each lookup, and a multiplication takes longer than the shift. In the four
streams' loop alone, silesia `-3` took 3.5% more and terminal's tiles 4.2%,
mixed's 1% fewer. V8 ran about as many instructions as with the shift. Entries
of 4 bytes, holding the multiplier, would save only that table's load.

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
