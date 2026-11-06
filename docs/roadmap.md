# Roadmap

What to make faster next, most promising first, as of 2026-11-06 (after the
unsafe native loops, run 26 of [`benchmarks/README.md`](benchmarks/README.md)). Figures from run 18 and later are
from lab2, on the generic workloads (Silesia, the GitHub sample set, wlshare's tiles
as a check); the older ones, from `perf` profiles on lab1 on the tiles, say so. The
decoder is a generic one, natively and in WebAssembly alike.

Unsafe code is allowed since run 26, kept to the hot loops: natively, the sequences' and the
four Huffman streams' loops run by raw pointers, as libzstd's, in a BMI2 build where the
processor has it. WebAssembly keeps the safe loops.

Natively, by geomean over the 10 workloads (run 26), ruzstd-decompress takes 0.97 times
libzstd's time: 1.10–1.16 times on Silesia, `x-ray` included, 1.12 on the GitHub frames
with their dictionary, 1.14 on mixed's tiles; 0.90 on the GitHub frames without it, and
less on the raw blocks of the video clip (0.76) and terminal's tiles (0.48). In wasm, which
libzstd is not built for here, the workloads take 1.26–1.65 times their native time (run
26), 1.50 by geomean, the dictionary frames 2.14, the video clip's raw blocks the same; 1.24–
1.48 before run 26, whose native loops wasm does not have.

## 1. The sequences, again

Natively (run 26), the loop is libzstd's: the reader's position a raw pointer, the
literals', the offset history and the ring's tail in locals, each sequence copied in wild
chunks past its end after one check for room, a match within 16 bytes and one back across
the ring's wrap in the loop, in a BMI2 build. On silesia `-3` it runs as many instructions a
cycle as libzstd (3.3), its loads, L1 and L2 misses and mispredictions much the same; it
trails by about 7 cycles a sequence (40 to 35), with about 10 stores a sequence more, the
loop's spills: 11 values carried from one sequence to the next and 6 for the whole loop,
more than x86-64's registers. What is left:
- **The registers**, natively: the three tables' bases could be one, were the tables in one
  allocation, the states then indices into it; that frees two. Tried and slower or no
  faster: the states stored on an error so that they are computed before the copies, the
  short matches' copy inline, and 8 sequences decoded ahead with their matches' sources
  prefetched (run 26).
- **The raw loop in WebAssembly**: not tried. Its loops are still the safe ones. The raw
  pointers would take Rust's bounds checks away there too; whether V8's code for the raw
  loop spills less than for the safe one (below) is to be measured, on the same workloads.
- **The safe loop in WebAssembly**: V8's code for the loop spills either way. A form of the loop it
  takes better is not known; the forms tried are in run 18. Since #23 it runs 1.4% more
  instructions on silesia `-3` than before (75 M a pass, about 5 a sequence), not more of
  the stack's loads and stores: V8's code for the loop as #23 wrote it (run 23). Each of a
  sequence's six takes is two variable shifts, and V8 moves each shift's count into `cl`
  and masks it, about 12 instructions a take. Taking each by one shift and a mask from a
  table of 32, the window shifted once after each group of takes (8 variable shifts a
  sequence, not 12), was slower (lab2, 2026-11-06, a pass's instructions and cycles):
  silesia `-3` 4.0% more instructions and 7.7% more cycles, nci 3.8% and 6.5%, the GitHub
  frames 1.5% more instructions, `x-ray` and terminal's tiles the same. V8 takes four
  instructions to load a mask (no scaled index for a wasm load) and spills more of the
  loop's values. Natively it ran 4% more cycles on silesia `-3` and nci.
- **The dictionary frames in wasm** take 1.8 times their native time: there the fast
  copy takes no match into the dictionary, which the slow path takes (run 20; the check
  for one in the loop made V8's code 2% slower on silesia).

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

Since run 25 the table of pairs' round checks each stream's output room with one compare,
and refills from the source up to the container's end with no bounds checks: in wasm 10.7%
fewer instructions on `x-ray` and 4% on mixed's tiles, `x-ray` 7% faster over 16 rounds,
mixed's tiles no clearer than ±2%. The loop waits on each stream's chain from one lookup
to the next (the window's shift, the index, the entry's load, its multiplier, the
multiplication): fewer instructions off that chain save little. The four output positions
in a vector (LLVM's SLP vectorizer) are faster in V8 than without (run 25). Refilling the
sequences' fast loop the same way put the tail's length on its chain, and was slower.
