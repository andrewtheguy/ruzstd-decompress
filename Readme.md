# ruzstd-decompress (a pure Rust zstd decoder)

# What is this

A pure Rust implementation of the Zstandard compression format, as defined in [RFC8878](https://www.rfc-editor.org/rfc/rfc8878.pdf).

This crate contains a fully operational implementation of the decompression portion of the standard.

This is a fork of ruzstd (<https://github.com/KillingSpark/zstd-rs>), a decoder only: upstream's
compressor and dictionary builder have been removed, as nothing that uses it compresses with it.

This crate is currently actively maintained.

# Current Status

## Decompression
The `decoding` module provides a complete
implementation of a Zstandard decompressor.

In terms of speed, `ruzstd-decompress` is behind the original C implementation
which has a rust binding located [here](https://github.com/gyscos/zstd-rs).

Measuring with the 'time' utility the original zstd and my decoder both
decoding the same enwik9.zst file from a ramfs, my decoder is about 3.5
times slower. Enwik9 is highly compressible, for less compressible data
(like a ubuntu installation .iso) my decoder comes close to only being
1.4 times slower.

# How can you use it?

## Decompression

Additionally to the descriptions and the docs you can have a look at the zstd / zstd_streaming binaries. They showcase how this library can be used.

### Easy

The easiest is to wrap the io::Read into a StreamingDecoder which itself implements io::Read. It will decode blocks as necessary to fulfill the read requests

```rust, no_run
use ruzstd_decompress::decoding::StreamingDecoder;
use ruzstd_decompress::io::Read;

let mut source: &[u8] = todo!("Get a reader from a File or any other source");
let mut decoder = StreamingDecoder::new(&mut source).unwrap();

let mut result = Vec::new();
decoder.read_to_end(&mut result).unwrap();
```

This might be a problem if you are accepting user provided data. Frames can be REALLY big when decoded. If this is the case you should either check how big the frame
actually is or use the memory efficient approach described below.

### Memory efficient

If memory is a concern you can decode frames partially. There are two ways to do this:

#### Streaming decoder

Use the StreamingDecoder and use a while loop to fill your buffer (see src/bin/zstd_stream.rs for an example). This is the
recommended approach.

#### Use the lower level FrameDecoder

For an example see the src/bin/zstd.rs file. Basically you can decode the frame until either a
given block count has been decoded or the decodebuffer has reached a certain size. Then you can collect the bytes decoded so far and do something with them, and resume decoding the frame in a loop until the frame has been decoded completely. Collecting gives every byte as soon as its block is decoded; the decoder keeps the window that later blocks copy from by itself.

### On several threads

With the `rayon` feature, `FrameDecoder::decode_all_parallel` decodes a whole input on the threads
of the rayon pool it is called in. Independent frames decode side by side. Within a large frame,
each block's literals and sequences decode side by side, and are executed in order on one thread.
So an input of many frames scales with the threads (4.5–5 times as fast on 8, natively and in
WebAssembly), but less with a dictionary (1.9 times natively, 2.5 in WebAssembly), and a single
frame gets 1.4–2.5 times as fast (`docs/benchmarks/README.md`).

```rust, ignore
// It decodes to the output's spare capacity, never grown: room first, from the headers.
let bound = ruzstd_decompress::decoding::decompressed_bound(&compressed)?;
let mut output = Vec::with_capacity(bound.min(LIMIT) as usize);
pool.install(|| decoder.decode_all_parallel(&compressed, &mut output))?;
```

`FrameDecoder::decode_each_parallel` decodes many inputs side by side, each to its own output,
within that output's spare capacity, with a result for each: such as a frame for each tile of a
picture, each held to what its tile can be. A run of inputs goes to each of the pool's jobs, its
buffers and tables made once for the run.

```rust, ignore
let mut outputs: Vec<Vec<u8>> = tiles.iter().map(|t| Vec::with_capacity(t.most)).collect();
let results = pool.install(|| decoder.decode_each_parallel(&frames, &mut outputs));
```

Every decoder holds a frame to the content size its header gives, as libzstd does: a frame that
decodes to more fails at the block that would take it past, which writes nothing, and one that
decodes to less fails at its end.

In WebAssembly a thread is a worker on a shared memory: `examples/wasm-threads` is a module and
the JavaScript that runs it on a pool of workers, in a browser or in Node.

## Roadmap

1. More Performance optimizations
    1. sequence_decoding and reverse_bitreader::get_bits. Those account for about 50% of the whole time used in decoding

## Testing

Tests take two forms.

1. Tests using well-formed files that have to decode correctly and are checked against their originals
1. Tests using malformed input that have been generated by the fuzzer. These don't have to decode (they are garbage) but they must not make the decoder panic

## Fuzzing

Fuzzing has been done on

1. Random input with no initial corpus
2. The \*.zst in /fuzz_decodecorpus


### You want to help fuzz?

Use `cargo +nightly fuzz run decode` or some other fuzz target to run the fuzzer, with AddressSanitizer, cargo-fuzz's default, which needs nightly; on stable, `cargo fuzz run --sanitizer none decode`. It is seeded with files created with decodecorpus.

If the fuzzer finds a crash it will be saved to the artifacts dir by the fuzzer. Run `cargo test artifacts` to run the artifacts tests.
This will tell you where the decoder panics exactly. If you are able to fix the issue please feel free to do a pull request. If not please still submit the offending input and I will see how to fix it myself.

# Contributing

Contributions will be published under the same MIT license as this project.
