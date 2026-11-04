# Roadmap

What to make faster next, most promising first, as of 2026-11-04 (after
hashing only frames with a checksum, [`benchmarks/README.md`](benchmarks/README.md)).
The figures are from `perf` profiles on lab1, natively unless they say wasm;
zDesk's workload is the tiles, decoded in WebAssembly. The crate stays free of
unsafe code.

By geomean natively, libzstd takes 1.07 times ruzstd's time, with the harness
keeping its decoders across tiles as zDesk's page does. ruzstd is faster on
the still, drag, browser and terminal tiles; uncover is 1.07–1.18 times
slower, video and mixed 1.3–1.4 times, and silesia 1.5–1.6 times. In wasm,
video takes 1.23–1.26 times its native time.

## 1. The sequences, again

The sequence loop and its copies are still about a third of uncover's time.
What is left of them:
- The 2–11% of sequences the fast copy does not take: matches from less than
  16 bytes back (a repeated pattern was tried, and was no faster), and those
  near the end of the initialised buffer or reaching into the dictionary.
- The checks the executor still makes for each sequence: the literals' bound
  and the block's decoded length, which could be checked once a block from
  the sums.

## 2. What is left of the table builds

On uncover, which builds 847 sequence tables and 88 Huffman tables a pass:
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
- Keeping a table that is the same as the last: 69% of terminal's sequence
  tables are, 1% of uncover's.

## 3. The Huffman loops in wasm

The table of pairs' loop now moves its windows on by a multiplication in
wasm, and each stream finishes from the table of pairs: video 5–6% faster in
wasm. In V8's code for the loop, two of the four streams' windows are still
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
