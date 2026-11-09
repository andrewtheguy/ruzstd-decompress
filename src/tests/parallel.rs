//! [`FrameDecoder::decode_all_parallel`] and [`FrameDecoder::decode_each_parallel`] against the
//! decoder on one thread: the same bytes, or an error where it errs; and each held to its
//! output's room.
extern crate std;

use crate::decoding::errors::FrameDecoderError;
use crate::decoding::{Dictionary, FrameDecoder, decompressed_bound};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use std::fs;

/// Every `.zst` in `dir`, sorted, with its path.
fn zst_files(dir: &str) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "zst"))
        .map(|p| (String::from(p.to_str().unwrap()), fs::read(&p).unwrap()))
        .collect();
    files.sort();
    files
}

/// What `input` decodes to on one thread, by the streaming decoder's frames one after another.
fn decode_serial(decoder: &mut FrameDecoder, input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // Room enough that `decode_all_to_vec` is not short of it.
    let mut room = input.len() * 16 + (1 << 20);
    loop {
        out.clear();
        out.reserve(room);
        match decoder.decode_all_to_vec(input, &mut out) {
            Ok(()) => return Ok(out),
            Err(crate::decoding::errors::FrameDecoderError::TargetTooSmall) if room < 1 << 30 => {
                room *= 4;
            }
            Err(e) => return Err(alloc::format!("{e}")),
        }
    }
}

/// `f` in a pool of each of these many threads; in WebAssembly without threads, in rayon's
/// global pool, which is the calling thread.
fn in_pools(f: impl Fn(usize) + Sync) {
    if cfg!(target_family = "wasm") {
        return f(rayon::current_num_threads());
    }
    for threads in [1, 2, 3, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| f(threads));
    }
}

/// Room for what `input` decodes to: its bound, where it has one.
fn bound(input: &[u8]) -> usize {
    decompressed_bound(input).map_or(0, |bound| bound.min(1 << 30) as usize)
}

fn decode_parallel(decoder: &FrameDecoder, input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = b"before".to_vec();
    out.reserve_exact(bound(input));
    let capacity = out.capacity();
    let result = decoder.decode_all_parallel(input, &mut out);
    assert_eq!(out.capacity(), capacity, "the output was grown");
    match result {
        Ok(()) => {
            assert_eq!(&out[..6], b"before");
            Ok(out.split_off(6))
        }
        Err(e) => {
            assert_eq!(out, b"before", "the output is left as it was on an error");
            Err(alloc::format!("{e}"))
        }
    }
}

#[test]
fn corpus_files_alone_and_together() {
    let files = zst_files("./decodecorpus_files");
    assert!(files.len() > 50);
    let mut serial = FrameDecoder::new();
    let expected: Vec<Vec<u8>> = files
        .iter()
        .map(|(p, f)| decode_serial(&mut serial, f).unwrap_or_else(|e| panic!("{p}: {e}")))
        .collect();
    let mut all = Vec::new();
    let mut all_expected = Vec::new();
    for (i, ((_, f), e)) in files.iter().zip(&expected).enumerate() {
        all.extend_from_slice(f);
        all_expected.extend_from_slice(e);
        if i % 7 == 0 {
            // A skippable frame between some, of a magic from the range they have.
            all.extend_from_slice(&(0x184D2A50u32 + (i as u32 % 16)).to_le_bytes());
            all.extend_from_slice(&5u32.to_le_bytes());
            all.extend_from_slice(b"skip!");
        }
    }
    let decoder = FrameDecoder::new();
    in_pools(|threads| {
        for ((p, f), e) in files.iter().zip(&expected) {
            let got = decode_parallel(&decoder, f).unwrap_or_else(|err| panic!("{p}: {err}"));
            assert!(got == *e, "{p} on {threads} threads");
        }
        let got = decode_parallel(&decoder, &all).unwrap();
        assert!(
            got == all_expected,
            "all the files together, on {threads} threads"
        );
    });
}

#[test]
fn dictionary_frames() {
    let dict = fs::read("./dict_tests/dictionary").unwrap();
    let files = zst_files("./dict_tests/files");
    let mut serial = FrameDecoder::new();
    serial
        .add_dict(Dictionary::decode_dict(&dict).unwrap())
        .unwrap();
    let mut decoder = FrameDecoder::new();
    decoder
        .add_dict(Dictionary::decode_dict(&dict).unwrap())
        .unwrap();
    let mut all = Vec::new();
    let mut all_expected = Vec::new();
    for (p, f) in &files {
        all.extend_from_slice(f);
        all_expected.extend(decode_serial(&mut serial, f).unwrap_or_else(|e| panic!("{p}: {e}")));
    }
    in_pools(|threads| {
        let got = decode_parallel(&decoder, &all).unwrap();
        assert!(got == all_expected, "on {threads} threads");
    });
    // Without it, every frame that names it fails.
    let err = decode_parallel(&FrameDecoder::new(), &all).unwrap_err();
    assert!(err.contains("dictionary id"), "{err}");
}

/// Text of words in a random order, compressible as a book is, with stretches of random bytes
/// (raw blocks, raw literals) and of one byte (RLE blocks).
#[cfg(not(target_family = "wasm"))]
fn sample(len: usize, seed: u64) -> Vec<u8> {
    const WORDS: [&str; 24] = [
        "the",
        "decoder",
        "zstd",
        "frame",
        "block",
        "of",
        "sequences",
        "and",
        "literals",
        "huffman",
        "table",
        "window",
        "match",
        "offset",
        "repeat",
        "a",
        "to",
        "is",
        "in",
        "stream",
        "thread",
        "pool",
        "batch",
        "\n",
    ];
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        match next() % 64 {
            0 => out.extend((0..next() % 300_000).map(|_| next() as u8)),
            1 => out.extend(core::iter::repeat_n(next() as u8, next() % 300_000)),
            _ => {
                for _ in 0..next() % 2000 {
                    out.extend_from_slice(WORDS[next() % WORDS.len()].as_bytes());
                    out.push(b' ');
                }
            }
        }
    }
    out.truncate(len);
    out
}

/// Frames of many blocks, by libzstd at levels that choose differently: its tables repeated
/// from block to block, its literals raw or in one or four streams.
#[cfg(not(target_family = "wasm"))]
fn many_block_frames() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let mut cases = Vec::new();
    for (seed, len) in [(1, 3 << 20), (2, 700_000)] {
        let data = sample(len, seed);
        for level in [-5, 1, 3, 9, 19] {
            let compressed = zstd::bulk::compress(&data, level).unwrap();
            cases.push((
                alloc::format!("seed {seed}, level {level}"),
                data.clone(),
                compressed,
            ));
        }
        // No content size in the header, and a checksum.
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        std::io::Write::write_all(&mut encoder, &data).unwrap();
        cases.push((
            alloc::format!("seed {seed}, streamed"),
            data.clone(),
            encoder.finish().unwrap(),
        ));
    }
    cases
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn frames_of_many_blocks() {
    let decoder = FrameDecoder::new();
    let cases = many_block_frames();
    in_pools(|threads| {
        for (name, data, compressed) in &cases {
            let got =
                decode_parallel(&decoder, compressed).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(got == *data, "{name} on {threads} threads");
        }
        // Two of them, fewer frames than threads: each frame's blocks side by side.
        let mut two = cases[1].2.clone();
        two.extend_from_slice(&cases[7].2);
        let mut expected = cases[1].1.clone();
        expected.extend_from_slice(&cases[7].1);
        assert!(decode_parallel(&decoder, &two).unwrap() == expected);
    });
}

/// Cut short or with a byte changed, a frame decodes to an error, or, its checksum and its
/// blocks' checks passed, to what the decoder on one thread makes of it.
#[test]
#[cfg(not(target_family = "wasm"))]
fn damaged_frames() {
    let mut serial = FrameDecoder::new();
    let decoder = FrameDecoder::new();
    let cases = many_block_frames();
    let (_, _, compressed) = &cases[2];
    let mut state = 7u64;
    in_pools(|_| {
        for cut in [0, 1, 5, 100, compressed.len() / 2, compressed.len() - 1] {
            assert!(decode_parallel(&decoder, &compressed[..cut]).is_err() || cut == 0);
        }
    });
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    for (_, _, compressed) in cases.iter().step_by(3) {
        for _ in 0..40 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let mut damaged = compressed.clone();
            let at = (state >> 20) as usize % damaged.len();
            damaged[at] ^= 1 << ((state >> 8) % 8);
            let got = pool.install(|| decode_parallel(&decoder, &damaged));
            if let Ok(got) = got {
                assert!(
                    Ok(got) == decode_serial(&mut serial, &damaged),
                    "byte {at} changed"
                );
            }
        }
    }
}

#[test]
fn empty_and_skippable_only() {
    let decoder = FrameDecoder::new();
    assert_eq!(decode_parallel(&decoder, &[]).unwrap(), vec![]);
    let mut skip = 0x184D2A5Fu32.to_le_bytes().to_vec();
    skip.extend_from_slice(&3u32.to_le_bytes());
    skip.extend_from_slice(b"abc");
    assert_eq!(decode_parallel(&decoder, &skip).unwrap(), vec![]);
    skip.pop();
    assert!(decode_parallel(&decoder, &skip).is_err());
}

/// A frame decoded in place into the room its header says it needs fills that room exactly,
/// with nothing grown and copied: z000033's frame, of an even length, is decoded in place under
/// test (see `parallel::in_place`). A byte less room fails, nothing written past it.
#[test]
fn in_place_fills_its_room() {
    let input = fs::read("./decodecorpus_files/z000033.zst").unwrap();
    assert_eq!(decompressed_bound(&input).unwrap(), 1_022_035);
    let mut out = Vec::with_capacity(1_022_035);
    FrameDecoder::new()
        .decode_all_parallel(&input, &mut out)
        .unwrap();
    assert_eq!(out.len(), 1_022_035);
    assert_eq!(out.capacity(), out.len(), "the output was grown");

    let mut out = Vec::with_capacity(1_022_034);
    let result = FrameDecoder::new().decode_all_parallel(&input, &mut out);
    assert!(
        matches!(result, Err(FrameDecoderError::TargetTooSmall)),
        "{result:?}"
    );
    assert_eq!((out.len(), out.capacity()), (0, 1_022_034));
}

/// A frame of several compressed blocks, split across two threads (tests take small batches):
/// small enough for Miri, which runs it over the executor's raw loop on stored sequences.
#[test]
fn split_small_frame() {
    let mut serial = FrameDecoder::new();
    let decoder = FrameDecoder::new();
    // Miri takes minutes over one. Under test z000006's frame, of an even length, is decoded in
    // place, into the output, and z000025's into the decoder's ring (see `parallel::in_place`).
    let names: &[&str] = if cfg!(miri) {
        &["z000006.zst"]
    } else {
        &["z000006.zst", "z000025.zst"]
    };
    for name in names {
        let input = fs::read(alloc::format!("./decodecorpus_files/{name}")).unwrap();
        let expected = decode_serial(&mut serial, &input).unwrap();
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build();
        let got = match pool {
            Ok(pool) => pool.install(|| decode_parallel(&decoder, &input)),
            // WebAssembly without threads.
            Err(_) => decode_parallel(&decoder, &input),
        };
        assert!(got.unwrap() == expected, "{name}");
    }
}

/// A frame of a 1 KiB window: 8 raw blocks of 1 KiB, then `blocks` compressed blocks of 10 bytes,
/// each `num_sequences` sequences of no literals and a match of 3 by RLE tables, which read no
/// bits: a block's worth of sequences from a few bytes.
fn frame_of_bitless_sequences(blocks: usize, num_sequences: u16) -> Vec<u8> {
    // No content size, a window descriptor of 1 KiB.
    let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x00];
    let block_header = |len: u32, block_type: u32, last: bool| {
        let header = (len << 3) | (block_type << 1) | u32::from(last);
        [header as u8, (header >> 8) as u8, (header >> 16) as u8]
    };
    for i in 0..8u8 {
        frame.extend_from_slice(&block_header(1024, 0, false));
        frame.extend((0..1024u32).map(|j| (j as u8).wrapping_mul(i + 1)));
    }
    let n = num_sequences;
    let content = [
        0x00,                  // raw literals, none
        0x80 | (n >> 8) as u8, // the number of sequences, in two bytes
        n as u8,
        0x54, // literal lengths, offsets and match lengths: RLE
        0,    // literal length 0
        0,    // offset code 0: a repeated offset
        0,    // match length 3
        0x01, // the bitstream: its end mark alone
    ];
    for i in 0..blocks {
        frame.extend_from_slice(&block_header(content.len() as u32, 2, i + 1 == blocks));
        frame.extend_from_slice(&content);
    }
    frame
}

/// Blocks a few bytes long that each decode to a block's worth of sequences, and one with more
/// than its block can hold: the batches are cut by what they store, not by their input alone.
#[test]
fn blocks_of_many_bitless_sequences() {
    let mut serial = FrameDecoder::new();
    let decoder = FrameDecoder::new();
    // 341 matches of 3 make 1023 bytes, as many as a block of a 1 KiB window holds.
    let full = frame_of_bitless_sequences(2000, 341);
    let expected = decode_serial(&mut serial, &full).unwrap();
    assert_eq!(expected.len(), 8 * 1024 + 2000 * 1023);
    let over = frame_of_bitless_sequences(2000, 342);
    assert!(decode_serial(&mut serial, &over).is_err());
    in_pools(|threads| {
        let got = decode_parallel(&decoder, &full).unwrap();
        assert!(got == expected, "on {threads} threads");
        let err = decode_parallel(&decoder, &over).unwrap_err();
        // Split, the block is refused before its sequences are stored.
        let refused = if threads > 1 {
            "DecompressedSizeTooLarge { max: 1024, at_least: 1026 }"
        } else {
            "1026"
        };
        assert!(err.contains(refused), "on {threads} threads: {err}");
    });
}

/// A window of 4 GiB, allowed by the limit, is more than a 32-bit `usize` holds: refused, not
/// truncated to nothing, by both decoders.
#[test]
#[cfg(target_pointer_width = "32")]
fn window_past_usize() {
    // No content size; window log 32; one empty raw block.
    let frame = [0x28, 0xB5, 0x2F, 0xFD, 0x00, 22 << 3, 0x01, 0x00, 0x00];
    let mut decoder = FrameDecoder::new();
    decoder.set_max_window_size(1 << 40);
    let err = decode_parallel(&decoder, &frame).unwrap_err();
    assert!(err.contains("4294967296"), "{err}");
    let err = decode_serial(&mut decoder, &frame).unwrap_err();
    assert!(err.contains("4294967296"), "{err}");
}

/// What `input` decodes to on the pool it is called in, into `room` bytes of spare capacity: the
/// output, never grown, or the error, the output as it was.
fn decode_into(
    decoder: &FrameDecoder,
    input: &[u8],
    room: usize,
) -> Result<Vec<u8>, FrameDecoderError> {
    let mut out = b"before".to_vec();
    out.reserve_exact(room);
    let capacity = out.capacity();
    let result = decoder.decode_all_parallel(input, &mut out);
    assert_eq!(out.capacity(), capacity, "the output was grown");
    match result {
        Ok(()) => Ok(out.split_off(6)),
        Err(e) => {
            assert_eq!(out, b"before", "the output is left as it was on an error");
            Err(e)
        }
    }
}

/// Hand-made frames, decoding to more or less than their headers say, fail as on one thread.
#[test]
fn held_to_the_header() {
    use super::content_size::{Block, frame};
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let (first, second) = data.split_at(600);
    let blocks = [Block::Raw(first), Block::Raw(second)];
    let decoder = FrameDecoder::new();
    in_pools(|threads| {
        assert_eq!(
            decode_into(&decoder, &frame(&blocks, Some(1000)), 1000).unwrap(),
            data
        );
        let over = decode_into(&decoder, &frame(&blocks, Some(900)), 2000);
        assert!(
            matches!(
                over,
                Err(FrameDecoderError::ContentSizeExceeded { content_size: 900 })
            ),
            "on {threads} threads: {over:?}"
        );
        let short = decode_into(&decoder, &frame(&blocks, Some(1100)), 2000);
        assert!(
            matches!(
                short,
                Err(FrameDecoderError::ContentSizeShort {
                    content_size: 1100,
                    decoded: 1000
                })
            ),
            "on {threads} threads: {short:?}"
        );
        // The room is short of what the frame says, and of what it decodes to.
        let room = decode_into(&decoder, &frame(&blocks, Some(1000)), 999);
        assert!(
            matches!(room, Err(FrameDecoderError::TargetTooSmall)),
            "{room:?}"
        );
        // The frame is corrupt, past both: corrupt, not short of room.
        let both = decode_into(&decoder, &frame(&blocks, Some(900)), 800);
        assert!(
            matches!(both, Err(FrameDecoderError::TargetTooSmall)),
            "the room, 800, is reached first: {both:?}"
        );
        let both = decode_into(&decoder, &frame(&blocks, Some(700)), 800);
        assert!(
            matches!(
                both,
                Err(FrameDecoderError::ContentSizeExceeded { content_size: 700 })
            ),
            "{both:?}"
        );
    });
}

/// `compressed`, one frame whose header gives its content size in 4 bytes, saying `size`.
#[cfg(not(target_family = "wasm"))]
fn with_content_size(compressed: &[u8], size: u32) -> Vec<u8> {
    let descriptor = compressed[4];
    assert_eq!(
        descriptor & 0x23,
        0,
        "a window descriptor and no dictionary id"
    );
    assert_eq!(descriptor >> 6, 2, "a content size of 4 bytes");
    let mut out = compressed.to_vec();
    out[6..10].copy_from_slice(&size.to_le_bytes());
    out
}

/// libzstd's frames of many compressed blocks, their headers changed to say more or less than
/// they decode to, fail on one thread and on several, the blocks split or not.
#[test]
#[cfg(not(target_family = "wasm"))]
fn compressed_held_to_the_header() {
    let data = sample(1 << 20, 3);
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
    encoder.window_log(17).unwrap();
    encoder
        .set_pledged_src_size(Some(data.len() as u64))
        .unwrap();
    encoder.include_contentsize(true).unwrap();
    std::io::Write::write_all(&mut encoder, &data).unwrap();
    let compressed = encoder.finish().unwrap();
    let len = data.len() as u32;
    assert!(with_content_size(&compressed, len) == compressed);

    let mut serial = FrameDecoder::new();
    let decoder = FrameDecoder::new();
    for (size, exceeded) in [(len - 1, true), (len / 2 + 17, true), (len + 1, false)] {
        let changed = with_content_size(&compressed, size);
        let err = decode_serial(&mut serial, &changed).unwrap_err();
        assert!(err.contains("header gives"), "{size}: {err}");
        in_pools(|threads| {
            let got = decode_into(&decoder, &changed, 2 << 20);
            let right = match got {
                Err(FrameDecoderError::ContentSizeExceeded { content_size }) => {
                    exceeded && content_size == u64::from(size)
                }
                Err(FrameDecoderError::ContentSizeShort {
                    content_size,
                    decoded,
                }) => !exceeded && content_size == u64::from(size) && decoded == u64::from(len),
                _ => false,
            };
            assert!(right, "{size} on {threads} threads: {got:?}");
        });
    }
}

/// Outputs of less room than their input decodes to fail, nothing written past their capacity:
/// with frames side by side, a frame's blocks side by side, and frames that give no size.
#[test]
#[cfg(not(target_family = "wasm"))]
fn held_to_the_room() {
    let decoder = FrameDecoder::new();
    let files = zst_files("./decodecorpus_files");
    let mut all = Vec::new();
    for (_, f) in &files {
        all.extend_from_slice(f);
    }
    let cases = many_block_frames();
    let all_len = decode_into(&decoder, &all, bound(&all)).unwrap().len();
    in_pools(|threads| {
        assert_eq!(decode_into(&decoder, &all, all_len).unwrap().len(), all_len);
        for room in [0, all_len / 2, all_len - 1] {
            let got = decode_into(&decoder, &all, room);
            assert!(
                matches!(got, Err(FrameDecoderError::TargetTooSmall)),
                "{room} on {threads} threads: {got:?}"
            );
        }
        for (name, data, compressed) in &cases {
            let got = decode_into(&decoder, compressed, data.len() - 1);
            assert!(
                matches!(got, Err(FrameDecoderError::TargetTooSmall)),
                "{name} on {threads} threads: {got:?}"
            );
        }
    });
}

/// Inputs each decoded to their own output, side by side: each the same bytes as on one thread
/// or the same failure, with its own room; a failed one leaves its output as it was.
#[test]
fn each_input_to_its_own_output() {
    let files = zst_files("./decodecorpus_files");
    let mut serial = FrameDecoder::new();
    let expected: Vec<Vec<u8>> = files
        .iter()
        .map(|(p, f)| decode_serial(&mut serial, f).unwrap_or_else(|e| panic!("{p}: {e}")))
        .collect();
    let mut inputs: Vec<&[u8]> = files.iter().map(|(_, f)| &f[..]).collect();
    // A frame cut short, an empty input, and one that is short of room.
    let cut = &files[3].1[..files[3].1.len() / 2];
    inputs.insert(5, cut);
    inputs.insert(9, &[]);
    let short = 13;
    let decoder = FrameDecoder::new();
    in_pools(|threads| {
        let mut outputs: Vec<Vec<u8>> = inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let mut out = b"before".to_vec();
                let room = if i == short { 1 } else { bound(input) };
                out.reserve_exact(room);
                out
            })
            .collect();
        let capacities: Vec<usize> = outputs.iter().map(Vec::capacity).collect();
        let results = decoder.decode_each_parallel(&inputs, &mut outputs);
        assert_eq!(results.len(), inputs.len());
        let mut files_left = expected.iter();
        for (i, (result, out)) in results.iter().zip(&outputs).enumerate() {
            assert_eq!(out.capacity(), capacities[i], "output {i} was grown");
            if i == 5 {
                assert!(result.is_err() && out == b"before", "{i}: {result:?}");
                continue;
            }
            if i == 9 {
                assert!(result.is_ok() && out == b"before", "{i}: {result:?}");
                continue;
            }
            let expected = files_left.next().unwrap();
            if i == short && !expected.is_empty() {
                assert!(
                    matches!(result, Err(FrameDecoderError::TargetTooSmall)) && out == b"before",
                    "{i} on {threads} threads: {result:?}"
                );
                continue;
            }
            assert!(result.is_ok(), "{i} on {threads} threads: {result:?}");
            assert!(
                &out[..6] == b"before" && out[6..] == expected[..],
                "{i} on {threads} threads"
            );
        }
    });
}

/// Two inputs of large frames, fewer than the threads: each frame's blocks side by side.
#[test]
#[cfg(not(target_family = "wasm"))]
fn each_of_few_large_inputs() {
    let cases = many_block_frames();
    let decoder = FrameDecoder::new();
    let inputs = [&cases[1].2[..], &cases[11].2[..]];
    in_pools(|threads| {
        let mut outputs: Vec<Vec<u8>> = inputs
            .iter()
            .map(|input| Vec::with_capacity(bound(input)))
            .collect();
        let results = decoder.decode_each_parallel(&inputs, &mut outputs);
        assert!(
            results.iter().all(Result::is_ok),
            "on {threads} threads: {results:?}"
        );
        assert!(
            outputs[0] == cases[1].1 && outputs[1] == cases[11].1,
            "on {threads} threads"
        );
    });
}
