#![no_main]
//! `FrameDecoder::decode_all_parallel` on four threads against the decoder on one: the same
//! bytes, or an error where it errs; or a checksum's mismatch, which only the parallel decoder
//! checks. Under `--cfg fuzzing` it splits even small frames' blocks across the threads.
#[macro_use]
extern crate libfuzzer_sys;
extern crate ruzstd_decompress;
use ruzstd_decompress::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd_decompress::decoding::{BlockDecodingStrategy, FrameDecoder};
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
    let mut parallel = Vec::new();
    let result = pool.install(|| FrameDecoder::new().decode_all_parallel(data, &mut parallel));
    match (result, decode_serial(data)) {
        (Ok(()), Ok(serial)) => assert!(parallel == serial, "decoded to other bytes"),
        (Err(_), Err(_)) => assert!(parallel.is_empty(), "output left after an error"),
        (Err(FrameDecoderError::ChecksumMismatch { .. }), Ok(_)) => {}
        (Ok(()), Err(e)) => panic!("only the serial decoder failed: {e}"),
        (Err(e), Ok(_)) => panic!("only the parallel decoder failed: {e}"),
    }
});
