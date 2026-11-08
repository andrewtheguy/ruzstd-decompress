# ruzstd-mt: the decoder on several threads in WebAssembly

A WebAssembly module of ruzstd-decompress whose decodes run on a pool of threads, and the
JavaScript that starts the pool, the same in a browser and in Node:

```js
import { createDecoder } from "./js/ruzstd-mt.js";

const decoder = await createDecoder({ threads: 4 });    // default: navigator.hardwareConcurrency
const bytes = await decoder.decode(compressed);          // Uint8Array: every frame, decoded
decoder.close();
```

It is a sample to build on, not a package: the crate's own parallel decoder is
[`FrameDecoder::decode_all_parallel`](../../src/decoding/frame_decoder.rs) (the `rayon`
feature), and this is one way to give it threads in WebAssembly.

## What runs where

A page has no threads to spawn. A thread in WebAssembly is a worker that JavaScript starts.
That worker runs its own instance of the module on a memory shared by all of them. So:

- **The decoder worker** (`js/decoder-worker.js`) makes the module's first instance, and
  with it the shared memory. Every decode is called there. A decode waits for the pool's
  threads (`Atomics.wait`), which a browser's main thread is not allowed to do, so the decoder
  needs a worker of its own.
- **The pool workers** (`js/pool-worker.js`), `threads - 1` of them, each make an instance on
  that memory and take a seat in the decoder's rayon pool (`runPoolThread` in
  [`src/lib.rs`](src/lib.rs)). The decoder worker's own thread is the pool's first
  (`use_current_thread`), so `threads` threads decode in all.
- **`js/ruzstd-mt.js`**, on the page or in Node, starts them and hands decodes to the decoder
  worker. Each decode is a message there and back. The input is copied into the module's
  memory and the output out of it.
- **`js/env.js`** papers over the difference between a browser's workers and Node's
  `worker_threads`.

What the threads do (see [`src/decoding/parallel.rs`](../../src/decoding/parallel.rs)):

- **Frames are independent.** An input of many frames is decoded a run of frames to each
  job, side by side. This scales with the threads.
- **Within one frame** a block's matches copy from the blocks before it, so the sequences
  must be executed in order, on one thread. But a block's literals (its Huffman streams) and
  its sequences (its FSE streams) need only the block's tables. So a large frame is decoded in
  three steps:
  1. its tables, block after block;
  2. a batch of blocks' literals and sequences, side by side;
  3. their execution, in order, on one thread, beside the next batch's decoding.

  The execution is about half of a decode on most data, and it slows as the other threads
  work beside it. So one frame gets 1.4–2.5 times as fast on 8 threads, most where literals
  are most of it. Frames under about 256 KB a thread are decoded as on one thread.

So an input made of independent frames is what gains most. `zstd -T0` writes one frame,
whatever its threads. For many frames, compress pieces separately, as `pzstd` or zstd's
seekable format do: `split -b 4M data part. && for f in part.*; do zstd -q -c "$f"; done >
data.zst` decodes to `data` as well.

## Build

Needs: rustup, which installs the nightly of [`rust-toolchain.toml`](rust-toolchain.toml) by
itself; `wasm-bindgen-cli` at this crate's wasm-bindgen
(`cargo install wasm-bindgen-cli --version 0.2.130 --locked`); and binaryen's `wasm-opt`.

```sh
./build.sh          # pkg/: ruzstd_mt_bg.wasm and its glue, ruzstd_mt.js
```

**The nightly is needed only here, for the threads.** The standard library as shipped for
`wasm32-unknown-unknown` is built without atomics: its locks and its allocator are one
thread's. So it is built again with `atomics` (Cargo's `build-std`, in
[`.cargo/config.toml`](.cargo/config.toml)), which only a nightly Cargo does. The crate
itself builds on stable, and on one thread its wasm build needs neither.

The memory is imported and shared, with a maximum of 4 GiB, all that a 32-bit memory has.
A shared memory must declare a maximum, and a frame decodes into it whole. The flags are
`.cargo/config.toml`'s.

## Run

```sh
node test.mjs                              # decodecorpus_files on 1, 2 and 4 threads, against their originals
node bench.mjs FILE --threads 1,2,4,8      # ms a decode on each number of threads
node serve.mjs 8000                        # the demo, at http://127.0.0.1:8000/demo/
node browser-test.mjs DIR FILE...          # the module in headless Chrome, on files of DIR
```

- **The demo** ([`demo/index.html`](demo/index.html)) decodes a file you choose, with a
  dictionary if you give one. It times it on each number of threads and lets you save the
  output.
- **A shared memory needs a page that is cross-origin isolated**:
  `Cross-Origin-Opener-Policy: same-origin` and
  `Cross-Origin-Embedder-Policy: require-corp`. [`serve.mjs`](serve.mjs) sends both, and
  `crossOriginIsolated` is then true on the page. Without them `createDecoder` refuses.
- **`bench.mjs`** prints a JSON line per number of threads:
  - `decode_ms` is the decode alone, in the module.
  - `call_ms` adds the copies into the module's memory and out of it.
- **`browser-test.mjs`** runs `google-chrome` headless, which needs no GPU. It opens the demo
  over the DevTools protocol and decodes each file there on 1, 2, 4 and all threads, checking
  that every decode gives the same bytes. `--chrome PATH` picks another Chrome.

On lab2 (8 vCPUs, 4 cores, run 32 of the benchmarks), 8 threads against 1, in Node 26:

| input | 1 thread | 8 threads |
|---|---:|---:|
| Silesia in 51 frames of 4 MiB, 212 MB out | 289 ms | 64 ms (4.5×) |
| zstd's GitHub sample set, 9,114 small frames | 22.7 ms | 4.6 ms (4.9×) |
| Silesia's `x-ray`, one frame | 11.5 ms | 4.9 ms (2.3×) |
| Silesia at `-3`, one frame, 212 MB out | 297 ms | 174 ms (1.7×) |

[`docs/benchmarks/harness/threads.sh`](../../docs/benchmarks/harness/threads.sh) runs
`bench.mjs` and the native [`examples/decode_parallel.rs`](../decode_parallel.rs) side by side
on the benchmarks' corpora. Its figures are in
[`docs/benchmarks/README.md`](../../docs/benchmarks/README.md).

## Limits

- **Copies.** The input is copied in and the output out. For Silesia's 212 MB that adds about
  a quarter to the decode's time. A page that keeps its data in the module's memory can avoid
  both copies.
- **Memory.** A decode needs room in the module's memory for its input and its whole output,
  less than 4 GiB together. The memory grows to what the largest decode needed, and is not
  given back while the decoder lives.
- **Where it was tested.** Node 24 and 26, and headless Chrome 155 on Linux. A browser needs
  module workers, workers started from a worker, and cross-origin isolation. Other browsers
  that have those should run it, but have not been tried.
- **Closing.** `close()` frees the decoder first, and its pool's threads leave their seats.
  Only then are the workers ended, not while a thread waits in the module.
