# Roadmap

What to make faster next, most promising first, as of 2026-11-06 (after the
raw loops in wasm, run 27 of [`benchmarks/README.md`](benchmarks/README.md)). Figures from run 18 and later are
from lab2, on the generic workloads (Silesia, the GitHub sample set, wlshare's tiles
as a check); the older ones, from `perf` profiles on lab1 on the tiles, say so. The
decoder is a generic one, natively and in WebAssembly alike.

Unsafe code is allowed since run 26, kept to the hot loops: the sequences' and the four
Huffman streams' loops run by raw pointers, as libzstd's, natively in a BMI2 build where the
processor has it, and since run 27 in WebAssembly too, which had kept safe loops of its own.

Natively, by geomean over the 10 workloads (run 27), ruzstd-decompress takes 0.96 times
libzstd's time: 1.08–1.17 times on Silesia, `x-ray` included, 1.14 on the GitHub frames
with their dictionary, 1.08 on mixed's tiles; 0.92 on the GitHub frames without it, and
less on the raw blocks of the video clip (0.74) and terminal's tiles (0.44). In wasm, which
libzstd is not built for here, the workloads take 1.19–1.71 times their native time (run
27), 1.33 by geomean, the dictionary frames most, the video clip's raw blocks about the
same; 1.50 by geomean with the safe loops (run 26).

## 1. The sequences, again

Natively (run 26), the loop is libzstd's: the reader's position a raw pointer, the
literals', the offset history and the ring's tail in locals, each sequence copied in wild
chunks past its end after one check for room, a match within 16 bytes and one back across
the ring's wrap in the loop, in a BMI2 build. On silesia `-3` it runs as many instructions a
cycle as libzstd (3.3), its loads, L1 and L2 misses and mispredictions much the same; it
trails by about 7 cycles a sequence (40 to 35), with about 10 stores a sequence more, the
loop's spills: 11 values carried from one sequence to the next and 6 for the whole loop,
more than x86-64's registers. What is left:
- **The registers**, natively: what is spilled is not mostly the tables' bases. LLVM
  computes the next states after the sequence's copy, keeping six of the entries' fields
  live across it; forced before it, the states themselves are spilled. Tried and slower or no
  faster: the states stored on an error so that they are computed before the copies, the
  short matches' copy inline, and 8 sequences decoded ahead with their matches' sources
  prefetched (run 26); the three tables in one allocation, one base for all three, and the
  states computed before the copies by an empty `asm!` (1–6% more instructions, the cycles
  within 3% either way; [`benchmarks/README.md`](benchmarks/README.md), 2026-11-06).
- **WebAssembly** runs the same loop since run 27, with the Huffman streams' raw loops: both
  together 10.8% faster than wasm's safe loops by geomean. With the sequences' raw loop alone
  (three rounds), silesia `-3` was 15% faster, the GitHub frames with their dictionary 15%
  and terminal's tiles 12%, nearly all of what both gave there. What V8's code for it spends
  is not yet profiled; the dictionary frames take 1.71 times their native time there, the
  most of the workloads.

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
  libzstd. In wasm (lab2, 2026-11-06, V8's code, instructions sampled) it is 32% of
  those frames, 7.7% of terminal's tiles and 1.7% of mixed's, where the sequence tables'
  build is 2.0%, 4.4% and 1.7% (2.3% of silesia `-3`). Of the Huffman build there, about
  35% is the weights' FSE loop, waiting on each state's load; 16% the pass over all the
  weights that lists the symbols with a code and the highest weight; 10% placing the
  symbols by weight, a count through memory; 8% the table's fill and 5% the counts by
  weight. Listing the symbols in the weights' loop, which has room for it, was no clear
  gain (harness medians, five rounds): the GitHub frames 1.8% faster natively and 1.0% in
  wasm, terminal's tiles 1.1% slower both ways, all within the spread.

## 3. The Huffman loops

Natively (run 26), both of the four streams' loops run by raw pointers, as libzstd's
assembly loop: a lookup is a shift, a load, a store and a shift, the steps counted from what
every stream has left of its source and output, in a BMI2 build. On `x-ray` a pass runs
104 M instructions to libzstd's 92 M, in 1.10 times its time; mixed's tiles, mostly Huffman
literals, 1.14. The loop waits on each stream's chain from one lookup to the next, as
libzstd's does.

In wasm the same loops run since run 27, with the sequences' raw loop: both together made
`x-ray` 9% faster than wasm's safe loops, mixed's tiles 12%, which now take 1.19 and 1.27
times their native time. With the Huffman streams' raw loops alone (three rounds), `x-ray`
was 19% faster and mixed's tiles 3%, the sequences' alone 9% and 3%. The table of pairs'
shift by a multiplication, which made the safe loop faster in V8, made the raw one slower
(`x-ray` 4%).
