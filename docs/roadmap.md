# Roadmap

What to make faster next, most promising first, as of 2026-11-04 (after the
table builds and the ring buffer, [`benchmarks/README.md`](benchmarks/README.md)).
The figures are from `perf` profiles on lab1, natively; zDesk's workload is
the tiles, decoded in WebAssembly. The crate stays free of unsafe code.

## 1. Huffman literals, two symbols to a lookup

On video and mixed tiles, the four-stream Huffman loop is 44–59% of the time,
and libzstd is still 1.7–1.9 times faster there. Its profile on video shows
its double-symbol decoder (`HUF_readDTableX2_wksp`,
`HUF_decompress4X2_usingDTable_internal_fast`), which takes two symbols from
one table entry when their codes are short enough. libzstd picks it or the
single-symbol decoder for each block, from the literals' sizes
(`HUF_selectDecoder`).

What it takes: the X2 table build, a second fast loop (in each output width,
as the X1 loop is), and the choice between them. The wasm build would grow
by an estimated 10–20 KB.

## 2. The sequence loop

On uncover tiles, decoding and executing the sequences is about 40% of the
time (25% on terminal), and libzstd's loop (`ZSTD_decompressSequences`) takes
about half as long.
- Copy literals in 16-byte chunks, as matches already are, when the literals
  buffer has the bytes past them: no `memcpy` call for each sequence.
- Fewer checks for each sequence: `bits_remaining`, the block's decoded
  length and the literals' bound could be checked less often, or once a block.
- Update the offset history without branches.

## 3. What is left of the table builds

On uncover these are still about 30% of the time:
- The sequence tables' build, about 11%, about 1.8 times libzstd's
  (`ZSTD_buildFSETable`). Next to try: libzstd's spread for tables with no
  "less than one" probabilities, which needs no check of the top of the
  table, unrolled by two.
- Reading the tables' descriptions (`read_distribution`), about 6%: a word at
  a time, with fewer `Result`s, as `FSE_readNCount` does.
- The Huffman header and table (in `decode_literals`, 13–15% with the
  single-stream decode): every literals block fills all 2048 entries.
  Narrower tables were tried and made video slower in wasm, so the fill
  itself has to cost less: wider stores, or keeping the table when a block's
  weights are the previous block's.

## 4. The ring buffer in wasm

Its lazy initialisation cost silesia 2–4% in wasm in the quick checks (not in
the full run: −0.4% and −2.2%), with nothing in V8's profile to show where.
Worth a look again if the general corpora matter in wasm.

## 5. Smaller

- The frame checksum (XXH64, the `twox-hash` crate) is 3–5% of video.
- About 6% of video is in the harness's own function (`decode_bench`), most
  likely the copy of the output out of the decoder into the caller's `Vec`;
  not yet confirmed.
