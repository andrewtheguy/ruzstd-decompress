# Roadmap

What to make faster next, most promising first, as of 2026-11-04 (after the
table builds, [`benchmarks/README.md`](benchmarks/README.md)). The figures
are from `perf` profiles on lab1, natively unless they say wasm; zDesk's
workload is the tiles, decoded in WebAssembly. The crate stays free of unsafe
code.

By geomean, ruzstd and libzstd are now even natively, with the harness
keeping its decoders across tiles as zDesk's page does. ruzstd is faster on
the still, drag, browser and terminal tiles; uncover is 1.1–1.3 times slower,
and video, mixed and silesia 1.4–1.5 times.

## 1. The Huffman loops in wasm

The table of pairs made video 8–11% faster natively but 6–7% in wasm, where
`decode_literals` is still 52% of video's time. In V8's code for the loop,
every variable shift moves its count into `cl` and the streams' state is
spilled; LLVM also vectorizes the four output positions' additions for
`simd128` (packing them into one `u32` avoids that, and was slower). Worth
trying: two streams to a loop, which needs fewer registers, or fewer lookups a
refill.

## 2. The sequences, again

The sequence loop and its copies are still about a third of uncover's time.
What is left of them:
- The 2–11% of sequences the fast copy does not take: matches from less than
  16 bytes back (a repeated pattern was tried, and was no faster), and those
  near the end of the initialised buffer or reaching into the dictionary.
- The checks the executor still makes for each sequence: the literals' bound
  and the block's decoded length, which could be checked once a block from
  the sums.

## 3. What is left of the table builds

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

## 4. Smaller

- The frame checksum (XXH64, the `twox-hash` crate) is 3–5% of video.
- The harness's own function (`decode_bench`'s `replay`) is 19% of uncover,
  natively and in wasm, and about 6% of video natively and 10% in wasm: most likely the copy
  of the output out of the decoder into the caller's `Vec`, as zDesk's page
  makes it too; not yet confirmed. In V8 it runs in the baseline tier
  (Liftoff) throughout, being called once a pass.
- The ring buffer's lazy initialisation cost silesia 2–4% in wasm in the
  quick checks before (not in a full run), with nothing in V8's profile to
  show where.
