# Decoder performance work: handover

Branch `perf/compare-zstd-and-inline` on the fork
<https://github.com/PeterKnego/zstd-rs>, 8 commits on top of upstream `master` at
`fe37617`.

The code was written by an AI coding assistant. The fork owner directed the work but did not
review it line by line. This document helps upstream maintainers review the commits and pick
any of them. Every commit stands alone with its own explanation, measurement and tests, so
you can take any subset (see [Dependencies](#dependencies-between-commits)).

## Result

All changes are on the decoder side. The encoder is unchanged.

CPU cycles per `FrameDecoder::decode_all` call (libzstd level-3 frame, `perf stat -e cycles:u`,
pinned to one core, median of 5 runs, spread under 1%):

| input                           | master  | branch | speedup |
|---------------------------------|---------|--------|---------|
| rust-source (441 KB, text)      | 3.848M  | 2.319M | 1.66x   |
| decodecorpus z000033 (1.0 MB)   | 17.65M  | 8.21M  | 2.15x   |

Wall clock against libzstd (the `zstd` crate) on the branch, `cargo bench --bench compare_zstd`
on a quiet machine:

| input       | ruzstd   | libzstd  | ratio |
|-------------|----------|----------|-------|
| rust-source | 474.3 us | 147.9 us | 3.2x  |
| z000033     | 1.627 ms | 642.9 us | 2.5x  |

On `master` the same ratios are about 5.3x and 5.4x.

These totals include commit 3 (`cc92967`), which `peek-perf` later removes to avoid adding
new `unsafe` code (see [commit 3](#3-cc92967-copy-literals-with-overshooting-chunked-copies-instead-of-memcpy)).
That commit measured about 4% of `decode_all` time on both inputs.

Measured on an AMD Ryzen AI MAX+ 395 (Zen 5, x86_64), rustc 1.96.0. Nothing was measured on
other CPUs or on aarch64.

## How each commit was checked

- Unit tests and `cargo build --no-default-features` (no_std) pass after every commit.
- On the branch head, `cargo clippy --all-targets` reports one warning, in
  `src/tests/mod.rs:754`. This branch doesn't touch that file.
- The `decode` and `interop` fuzz targets ran for about 4 minutes each with 16 workers
  (AddressSanitizer, debug assertions) after `cc92967`, `1f34bdb`, `d53ced6`, `66a1c4a` and
  `0645c41`. None found a crash.
- Every new test was also run against a deliberately broken version of the code, to confirm
  it fails.
- Performance was measured with `perf stat` cycle counts against the previous commit, because
  wall-clock times on the test machine were noisy. Each commit's numbers are relative to its
  own parent, so they don't add up to the totals above.

## Commits, in order

### 1. `6b5b271` Add benchmark comparing ruzstd against the zstd crate

**What:** Adds a new criterion bench, `ruzstd/benches/compare_zstd.rs`, and its
`[[bench]]` entry in `Cargo.toml`. Benches:
- decompression: both decoders decode the same libzstd level-3 frame;
- compression: ruzstd `Fastest` against libzstd level 1, printing both compressed sizes.

The inputs are the crate's own `src/**/*.rs` files (concatenated in sorted order) and
`decodecorpus_files/z000033`. It uses the existing `zstd` dev-dependency.

**Risk:** none. The commit doesn't change library code.

### 2. `7030d54` Inline hot helpers in sequence decoding and execution

**What:** Adds `#[inline(always)]` to:
- `FSEDecoder::update_state`
- `DecodeBuffer::repeat`
- `RingBuffer::extend_from_within_unchecked`

perf showed all three being called as real functions from the hot loops.

**Measured:** decode_all 14.6% faster on text, 6.1% faster on z000033 (criterion).

**Review focus:** none needed. Only attributes change. `refill` was left `#[cold]` here because
inlining it in this commit made z000033 3.6% slower. Commit 8 solves that differently.

### 3. `cc92967` Copy literals with overshooting chunked copies instead of memcpy

> **Removed on `peek-perf`:** the commit "Remove the overshooting literal copy" undoes this
> change so that the branch adds no new `unsafe` blocks. Literal runs are copied with an
> exact `push` again, and `extend_from_prefix` and `push_prefix` are gone. The commits after
> this one still apply; commit 5 calls `push` instead of `push_prefix`.

**What:** Literal runs are now copied with the existing `copy_bytes_overshooting` (16-byte
chunks) instead of an exact `extend_from_slice`, which compiled to a glibc `memcpy` call per
sequence. The new `RingBuffer::extend_from_prefix(data, len)` takes all remaining literals
plus the run length. Reads may run past the run into later literals, and writes may run past
it into free ring-buffer space. A new `DecodeBuffer::push_prefix` wraps it.

**Measured:** decode_all about 4% faster on both inputs. memmove self time went from 4.4% to
1.5%.

**Review focus:** this is the only commit that adds an `unsafe` block. Check the bounds
argument in `extend_from_prefix`:
- `len <= data.len()` is asserted;
- the overshooting read stays inside `data`;
- the overshooting write stays inside the free regions of the ring buffer, which end at
  `head`.

New test: `extend_from_prefix_matches_model`, checked against a `VecDeque` model.

### 4. `f05f3f4` Fix encoding half of the interop fuzz target

**What:** This is a bug fix to the fuzz harness. The ruzstd encode helpers in
`fuzz/fuzz_targets/interop.rs` read their input reader to the end and then compressed the
already-used-up reader. So they always produced an empty frame, and the target failed on any
non-empty input. Also, the "compressed" variant used `CompressionLevel::Uncompressed`. The fix
passes the input slice directly and uses `Fastest`.

**Risk:** none. Only the fuzz harness changes. Take this one even if you take nothing else:
without it the interop target can't be used.

### 5. `1f34bdb` Execute sequences as they are decoded instead of collecting them first

**What:** `decode_sequences` now takes a closure that runs on each decoded sequence.
`execute_sequences` becomes the `SequenceExecutor` struct (`new` / `execute` / `finish`).
Before, sequences were collected into a `Vec` and executed in a second pass; that `Vec` and
`DecoderScratch::sequences` are removed.

**Measured:** cycles per decode 6.8% lower on text, 1.5% lower on z000033.

**Behavior change on corrupt input only:**
- an execution error in an early sequence is now reported before a decode error in a later
  one;
- a failing block may leave partial output in the decode buffer.

Valid input decodes exactly as before.

**Depends on:** commit 3, because it uses `push_prefix`.

### 6. `d53ced6` Decode Huffman literals with one lookup per symbol and interleaved streams

**What:**
- **One lookup per symbol.** The next `max_num_bits` bits of the stream are exactly the state
  that `HuffmanDecoder` kept, so each symbol is now one table lookup on peeked bits plus a
  consume. See `HuffmanTable::decode_stream`.
- **Interleaved streams.** The 4 streams decode in one loop into their spec-defined segments of
  a pre-sized output, with one refill check per `56 / max_num_bits` symbols. See
  `HuffmanTable::decode_4_streams`.
- **New helper.** `BitReaderReversed::ensure_bits` is added.
- **Old decoder kept as reference.** The old `HuffmanDecoder` stays, behind
  `cfg(any(test, feature = "fuzz_exports"))`. The extended huff0 round-trip test checks the new
  decoders against it.

**Measured:** cycles per decode 28.5% lower on z000033, 7.6% lower on text.

**Behavior change on malformed input only:** it is now stricter, like libzstd:
- each stream must decode to exactly its segment size;
- 4-stream sections need at least 6 literals (libzstd's `MIN_LITERALS_FOR_4_STREAMS`);
- decoding stops after the expected number of symbols.

The ruzstd encoder already uses 4 streams only for 6 or more literals.

### 7. `66a1c4a` Copy overlapping matches in doubling chunks

**What:** `repeat_in_chunks` used to copy `offset` bytes per iteration, so an offset-1 match
made one ring-buffer copy per output byte. Each copy now covers everything from the pattern
start to the current end, so the copies double in size (offset, 2·offset, 4·offset, …). That
takes log2(match_length / offset) copies.

**Measured:** cycles per decode 18.0% lower on z000033, 2.6% lower on text.

**Review focus:** the copy calls the existing `unsafe extend_from_within_unchecked`, now with
larger chunks. Its safety comment was rewritten to explain why each chunk's source lies inside
already-written data.

New test: `repeat_matches_bytewise_copy`, checked against a bytewise model. It covers offsets
1–24, match lengths 0–200, and several drain states.

### 8. `0645c41` Inline the common case of the reverse bit reader refill

**What:** `get_bits_triple` calls `refill` once per sequence, and `refill` was `#[cold]`, so
every sequence paid for a function call. The common case, where enough source is left, is now
inlined. The end-of-source branches moved unchanged into a `#[cold] #[inline(never)]`
`refill_slow`.

**Measured:** cycles per decode 12.5% lower on text, 9.1% lower on z000033.

New test: `matches_bitwise_model`, which checks `get_bits` and `get_bits_triple` against a
bitwise model for source lengths 0–39.

## Dependencies between commits

I cherry-picked each commit alone onto `master` (`fe37617`) and ran `cargo test --lib`:

| commit | alone on master |
|---|---|
| `6b5b271` bench | applies, tests pass |
| `7030d54` inline helpers | applies, tests pass |
| `cc92967` literal copies | **conflicts** in `decode_buffer.rs` (an adjacent `#[inline(always)]` from `7030d54`); applies cleanly on top of `7030d54` |
| `f05f3f4` fuzz harness fix | applies, tests pass |
| `1f34bdb` fused sequence loop | **conflicts** alone; `7030d54` + `cc92967` + `1f34bdb` apply and pass |
| `d53ced6` Huffman | applies, tests pass |
| `66a1c4a` match copy doubling | applies, tests pass |
| `0645c41` refill inlining | applies, tests pass |

So the only real dependency is `1f34bdb` → `cc92967`. The `cc92967` → `7030d54` conflict is
only textual.

## Easiest commits to review first

Smallest diff for the speedup, roughly in order of how much there is to check:
1. `f05f3f4`: fuzz harness bug fix.
2. `7030d54`: attributes only.
3. `0645c41`: moves existing code into a cold function, plus a model test.
4. `66a1c4a`: about 20 lines of logic, plus a model test.
5. `d53ced6`: largest gain on non-text data. Malformed-input handling changes.
6. `cc92967` and then `1f34bdb`: the new `unsafe` code, and the behavior change on corrupt
   input.

## Not done

- The encoder hasn't been touched. Against libzstd level 1, `Fastest` is 4.5–6.4x slower and
  produces 10–40% larger output.
- Ideas not attempted for the decoder:
  - libzstd-style sequence decoding tables, with base value and extra bits stored in the FSE
    entries;
  - cheaper Huffman and FSE table rebuilds, which take about 8–9% each on z000033;
  - decoding directly into the caller's buffer.
