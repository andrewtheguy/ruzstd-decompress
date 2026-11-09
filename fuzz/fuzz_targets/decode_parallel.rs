#![no_main]
//! `FrameDecoder::decode_all_parallel` on four threads against the decoder on one: the same
//! bytes, or an error where it errs; or a checksum's mismatch, which only the parallel decoder
//! checks; or, where the input decodes to more than the room its headers' bound leaves (up to
//! 16 MiB), the output's being too small, the output never grown. And
//! `FrameDecoder::decode_each_parallel` of the input twice, each the same as alone. Under
//! `--cfg fuzzing` it splits even small frames' blocks across the threads.
#[macro_use]
extern crate libfuzzer_sys;
extern crate ruzstd_decompress;
use ruzstd_decompress::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd_decompress::decoding::{BlockDecodingStrategy, FrameDecoder, decompressed_bound};
use std::sync::OnceLock;

/// Every frame of `input`, one after another, on this thread.
fn decode_serial(mut input: &[u8]) -> Result<Vec<u8>, FrameDecoderError> {
    let mut decoder = FrameDecoder::new();
    let mut output = Vec::new();
    while !input.is_empty() {
        match decoder.reset(&mut input) {
            Ok(()) => {}
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                input = input
                    .get(length as usize..)
                    .ok_or(FrameDecoderError::FailedToSkipFrame)?;
                continue;
            }
            Err(e) => return Err(e),
        }
        while !decoder.is_finished() {
            decoder.decode_blocks(&mut input, BlockDecodingStrategy::UptoBytes(1 << 20))?;
            decoder
                .collect_to_writer(&mut output)
                .map_err(FrameDecoderError::FailedToDrainDecodebuffer)?;
        }
        decoder
            .collect_to_writer(&mut output)
            .map_err(FrameDecoderError::FailedToDrainDecodebuffer)?;
    }
    Ok(output)
}

fuzz_target!(|data: &[u8]| {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
    });
    let room = decompressed_bound(data).map_or(1 << 20, |bound| bound.min(16 << 20) as usize);
    let mut parallel = Vec::with_capacity(room);
    let capacity = parallel.capacity();
    let decoder = FrameDecoder::new();
    let result = pool.install(|| decoder.decode_all_parallel(data, &mut parallel));
    assert_eq!(parallel.capacity(), capacity, "the output was grown");
    let serial = decode_serial(data);
    match (&result, &serial) {
        (Ok(()), Ok(serial)) => assert!(parallel == *serial, "decoded to other bytes"),
        (Err(_), Err(_)) => assert!(parallel.is_empty(), "output left after an error"),
        (Err(FrameDecoderError::ChecksumMismatch { .. }), Ok(_)) => {}
        (Err(FrameDecoderError::TargetTooSmall), Ok(serial)) if serial.len() > capacity => {}
        (Ok(()), Err(e)) => panic!("only the serial decoder failed: {e}"),
        (Err(e), Ok(_)) => panic!("only the parallel decoder failed: {e}"),
    }

    let mut outputs = vec![Vec::with_capacity(room), Vec::with_capacity(room)];
    let results = pool.install(|| decoder.decode_each_parallel(&[data, data], &mut outputs));
    for (each, out) in results.iter().zip(&outputs) {
        assert_eq!(out.capacity(), capacity, "an output was grown");
        match (each, &result) {
            (Ok(()), Ok(())) => assert!(*out == parallel, "decoded to other bytes alone"),
            (Err(_), Err(_)) => assert!(out.is_empty(), "output left after an error"),
            _ => panic!("decoded each {each:?}, alone {result:?}"),
        }
    }
});
