# Roadmap

What to make faster next, most promising first, as of 2026-11-04 (after the
sequence loop, [`benchmarks/README.md`](benchmarks/README.md)). The figures
are from `perf` profiles on lab1, natively unless they say wasm; zDesk's
workload is the tiles, decoded in WebAssembly. The crate stays free of unsafe
code.

By geomean, libzstd is 1.05 times faster natively. ruzstd is faster on the
still, drag and browser tiles; uncover is 1.2–1.4 times slower, and video,
mixed and silesia 1.5–1.6 times.

## 1. What is left of the table builds

On uncover, where ruzstd is furthest behind on the small tiles:
- The sequence tables' build, about 12% of the time, about 1.8 times
  libzstd's (`ZSTD_buildFSETable`). Next to try: libzstd's spread for tables
  with no "less than one" probabilities, which needs no check of the top of
  the table, unrolled by two.
- Reading the tables' descriptions (`read_distribution`), about 5%: a word at
  a time, with fewer `Result`s, as `FSE_readNCount` does.
- The Huffman header and table (in `decode_literals`): every literals block
  fills all 2048 entries. Narrower tables were tried and made video slower in
  wasm, so the fill itself has to cost less: wider stores, or keeping the
  table when a block's weights are the previous block's.

## 2. The Huffman loops in wasm

The table of pairs made video 8–11% faster natively but 6–7% in wasm, where
`decode_literals` is still 52% of video's time. In V8's code for the loop,
every variable shift moves its count into `cl` and the streams' state is
spilled; LLVM also vectorizes the four output positions' additions for
`simd128` (packing them into one `u32` avoids that, and was slower). Worth
trying: two streams to a loop, which needs fewer registers, or fewer lookups a
refill.

## 3. The sequences, again

The sequence loop and its copies are still about a third of uncover's time.
What is left of them:
- The 2–11% of sequences the fast copy does not take: matches from less than
  16 bytes back (a repeated pattern was tried, and was no faster), and those
  near the end of the initialised buffer or reaching into the dictionary.
- The checks the executor still makes for each sequence: the literals' bound
  and the block's decoded length, which could be checked once a block from
  the sums.

## 4. Smaller

- The frame checksum (XXH64, the `twox-hash` crate) is 3–5% of video.
- About 6% of video natively, and 10% in wasm, is in the harness's own
  function (`decode_bench`'s `replay`), most likely the copy of the output
  out of the decoder into the caller's `Vec`; not yet confirmed. In V8 it runs
  in the baseline tier (Liftoff) throughout, being called once a pass.
- The ring buffer's lazy initialisation cost silesia 2–4% in wasm in the
  quick checks before (not in a full run), with nothing in V8's profile to
  show where.
