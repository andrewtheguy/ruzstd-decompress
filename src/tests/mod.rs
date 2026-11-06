#[cfg(test)]
use alloc::vec;

#[cfg(test)]
use alloc::vec::Vec;

#[cfg(test)]
extern crate std;

#[cfg(all(test, not(feature = "std")))]
impl crate::io_nostd::Read for std::fs::File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, crate::io_nostd::Error> {
        std::io::Read::read(self, buf).map_err(|e| {
            if e.get_ref().is_none() {
                crate::io_nostd::Error::from(crate::io_nostd::ErrorKind::Other)
            } else {
                crate::io_nostd::Error::new(
                    crate::io_nostd::ErrorKind::Other,
                    alloc::boxed::Box::new(e.into_inner().unwrap()),
                )
            }
        })
    }
}

#[cfg(all(test, feature = "std"))]
#[allow(dead_code)]
fn assure_error_impl() {
    // not a real test just there to throw an compiler error if Error is not derived correctly

    use crate::decoding::errors::FrameDecoderError;
    let _err: &dyn std::error::Error = &FrameDecoderError::NotYetInitialized;
}

#[cfg(all(test, feature = "std"))]
#[allow(dead_code)]
fn assure_decoder_send_sync() {
    // not a real test just there to throw an compiler error if FrameDecoder is Send + Sync

    use crate::decoding::FrameDecoder;
    let decoder = FrameDecoder::new();
    std::thread::spawn(move || {
        drop(decoder);
    });
}

#[test]
fn skippable_frame() {
    use crate::decoding::errors;
    use crate::decoding::frame;

    let mut content = vec![];
    content.extend_from_slice(&0x184D2A50u32.to_le_bytes());
    content.extend_from_slice(&300u32.to_le_bytes());
    assert_eq!(8, content.len());
    let err = frame::read_frame_header(content.as_slice());
    assert!(matches!(
        err,
        Err(errors::ReadFrameHeaderError::SkipFrame {
            magic_number: 0x184D2A50u32,
            length: 300
        })
    ));

    content.clear();
    content.extend_from_slice(&0x184D2A5Fu32.to_le_bytes());
    content.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    assert_eq!(8, content.len());
    let err = frame::read_frame_header(content.as_slice());
    assert!(matches!(
        err,
        Err(errors::ReadFrameHeaderError::SkipFrame {
            magic_number: 0x184D2A5Fu32,
            length: 0xFFFFFFFF
        })
    ));
}

#[cfg(test)]
#[test]
fn test_frame_header_reading() {
    use crate::decoding::frame;
    use std::fs;

    let mut content = fs::File::open("./decodecorpus_files/z000088.zst").unwrap();
    let (_frame, _) = frame::read_frame_header(&mut content).unwrap();
}

#[test]
fn test_block_header_reading() {
    use crate::decoding;
    use crate::decoding::frame;
    use std::fs;

    let mut content = fs::File::open("./decodecorpus_files/z000088.zst").unwrap();
    let (_frame, _) = frame::read_frame_header(&mut content).unwrap();

    let mut block_dec = decoding::block_decoder::new();
    let block_header = block_dec.read_block_header(&mut content).unwrap();
    let _ = block_header; //TODO validate blockheader in a smart way
}

#[test]
fn test_frame_decoder() {
    use crate::decoding::BlockDecodingStrategy;
    use crate::decoding::FrameDecoder;
    use std::fs;

    let mut content = fs::File::open("./decodecorpus_files/z000088.zst").unwrap();

    struct NullWriter(());
    impl std::io::Write for NullWriter {
        fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> Result<(), std::io::Error> {
            Ok(())
        }
    }
    let mut _null_target = NullWriter(());

    let mut frame_dec = FrameDecoder::new();
    frame_dec.reset(&mut content).unwrap();
    frame_dec
        .decode_blocks(&mut content, BlockDecodingStrategy::All)
        .unwrap();
}

#[test]
fn test_decode_from_to() {
    use crate::decoding::FrameDecoder;
    use std::fs::File;
    use std::io::BufReader;
    use std::io::Read;
    let f = BufReader::new(File::open("./decodecorpus_files/z000088.zst").unwrap());
    let mut frame_dec = FrameDecoder::new();

    let content: Vec<u8> = f.bytes().map(|x| x.unwrap()).collect();

    let mut target = vec![0u8; 1024 * 1024];

    // first part
    let source1 = &content[..50 * 1024];
    let (read1, written1) = frame_dec
        .decode_from_to(source1, target.as_mut_slice())
        .unwrap();

    //second part explicitely without checksum
    let source2 = &content[read1..content.len() - 4];
    let (read2, written2) = frame_dec
        .decode_from_to(source2, &mut target[written1..])
        .unwrap();

    //must have decoded until checksum
    assert!(read1 + read2 == content.len() - 4);

    //insert checksum separatly to test that this is handled correctly
    let chksum_source = &content[read1 + read2..];
    let (read3, written3) = frame_dec
        .decode_from_to(chksum_source, &mut target[written1 + written2..])
        .unwrap();

    //this must result in these values because just the checksum was processed
    assert!(read3 == 4);
    assert!(written3 == 0);

    let read = read1 + read2 + read3;
    let written = written1 + written2;

    let result = &target.as_slice()[..written];

    if read != content.len() {
        panic!(
            "Byte counter: {} was wrong. Should be: {}",
            read,
            content.len()
        );
    }

    match frame_dec.get_checksum_from_data() {
        Some(chksum) => {
            #[cfg(feature = "hash")]
            if frame_dec.get_calculated_checksum().unwrap() != chksum {
                std::println!(
                    "Checksum did not match! From data: {}, calculated while decoding: {}\n",
                    chksum,
                    frame_dec.get_calculated_checksum().unwrap()
                );
            } else {
                std::println!("Checksums are ok!\n");
            }
            #[cfg(not(feature = "hash"))]
            std::println!(
                "Checksum feature not enabled, skipping. From data: {}\n",
                chksum
            );
        }
        None => std::println!("No checksums to test\n"),
    }

    let original_f = BufReader::new(File::open("./decodecorpus_files/z000088").unwrap());
    let original: Vec<u8> = original_f.bytes().map(|x| x.unwrap()).collect();

    if original.len() != result.len() {
        panic!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        panic!("Result differs in at least {} bytes from original", counter);
    }
}

#[test]
fn test_specific_file() {
    use crate::decoding::BlockDecodingStrategy;
    use crate::decoding::FrameDecoder;
    use std::fs;
    use std::io::BufReader;
    use std::io::Read;

    let path = "./decodecorpus_files/z000068.zst";
    let mut content = fs::File::open(path).unwrap();

    struct NullWriter(());
    impl std::io::Write for NullWriter {
        fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> Result<(), std::io::Error> {
            Ok(())
        }
    }
    let mut _null_target = NullWriter(());

    let mut frame_dec = FrameDecoder::new();
    frame_dec.reset(&mut content).unwrap();
    frame_dec
        .decode_blocks(&mut content, BlockDecodingStrategy::All)
        .unwrap();
    let result = frame_dec.collect().unwrap();

    let original_f = BufReader::new(fs::File::open("./decodecorpus_files/z000088").unwrap());
    let original: Vec<u8> = original_f.bytes().map(|x| x.unwrap()).collect();

    std::println!("Results for file: {path}");

    if original.len() != result.len() {
        std::println!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        std::println!("Result differs in at least {counter} bytes from original");
    }
}

#[test]
#[cfg(feature = "std")]
fn test_streaming() {
    use std::fs;
    use std::io::BufReader;
    use std::io::Read;

    let mut content = fs::File::open("./decodecorpus_files/z000088.zst").unwrap();
    let mut stream = crate::decoding::StreamingDecoder::new(&mut content).unwrap();

    let mut result = Vec::new();
    Read::read_to_end(&mut stream, &mut result).unwrap();

    let original_f = BufReader::new(fs::File::open("./decodecorpus_files/z000088").unwrap());
    let original: Vec<u8> = original_f.bytes().map(|x| x.unwrap()).collect();

    if original.len() != result.len() {
        panic!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        panic!("Result differs in at least {} bytes from original", counter);
    }

    // Test resetting to a new file while keeping the old decoder

    let mut content = fs::File::open("./decodecorpus_files/z000068.zst").unwrap();
    let mut stream = crate::decoding::StreamingDecoder::new_with_decoder(
        &mut content,
        stream.into_frame_decoder(),
    )
    .unwrap();

    let mut result = Vec::new();
    Read::read_to_end(&mut stream, &mut result).unwrap();

    let original_f = BufReader::new(fs::File::open("./decodecorpus_files/z000068").unwrap());
    let original: Vec<u8> = original_f.bytes().map(|x| x.unwrap()).collect();

    std::println!("Results for file:");

    if original.len() != result.len() {
        panic!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        panic!("Result differs in at least {} bytes from original", counter);
    }
}

#[test]
fn test_incremental_read() {
    use crate::decoding::FrameDecoder;

    let mut unread_compressed_content =
        include_bytes!("../../decodecorpus_files/abc.txt.zst").as_slice();

    let mut frame_dec = FrameDecoder::new();
    frame_dec.reset(&mut unread_compressed_content).unwrap();

    let mut output = [0u8; 3];
    let (_, written) = frame_dec
        .decode_from_to(unread_compressed_content, &mut output)
        .unwrap();

    assert_eq!(written, 3);
    assert_eq!(output.map(char::from), ['a', 'b', 'c']);

    assert!(frame_dec.is_finished());
    let written = frame_dec.collect_to_writer(&mut &mut output[..]).unwrap();
    assert_eq!(written, 3);
    assert_eq!(output.map(char::from), ['d', 'e', 'f']);
}

#[test]
#[cfg(not(feature = "std"))]
fn test_streaming_no_std() {
    use crate::io::Read;

    let content = include_bytes!("../../decodecorpus_files/z000088.zst");
    let mut content = content.as_slice();
    let mut stream = crate::decoding::StreamingDecoder::new(&mut content).unwrap();

    let original = include_bytes!("../../decodecorpus_files/z000088");
    let mut result = vec![0; original.len()];
    Read::read_exact(&mut stream, &mut result).unwrap();

    if original.len() != result.len() {
        panic!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        panic!("Result differs in at least {} bytes from original", counter);
    }

    // Test resetting to a new file while keeping the old decoder

    let content = include_bytes!("../../decodecorpus_files/z000068.zst");
    let mut content = content.as_slice();
    let mut stream = crate::decoding::StreamingDecoder::new_with_decoder(
        &mut content,
        stream.into_frame_decoder(),
    )
    .unwrap();

    let original = include_bytes!("../../decodecorpus_files/z000068");
    let mut result = vec![0; original.len()];
    Read::read_exact(&mut stream, &mut result).unwrap();

    std::println!("Results for file:");

    if original.len() != result.len() {
        panic!(
            "Result has wrong length: {}, should be: {}",
            result.len(),
            original.len()
        );
    }

    let mut counter = 0;
    let min = if original.len() < result.len() {
        original.len()
    } else {
        result.len()
    };
    for idx in 0..min {
        if original[idx] != result[idx] {
            counter += 1;
            //std::println!(
            //    "Original {:3} not equal to result {:3} at byte: {}",
            //    original[idx], result[idx], idx,
            //);
        }
    }
    if counter > 0 {
        panic!("Result differs in at least {} bytes from original", counter);
    }
}

#[test]
fn test_decode_all() {
    use crate::decoding::FrameDecoder;
    use crate::decoding::errors::FrameDecoderError;

    let skip_frame = |input: &mut Vec<u8>, length: usize| {
        input.extend_from_slice(&0x184D2A50u32.to_le_bytes());
        input.extend_from_slice(&(length as u32).to_le_bytes());
        input.resize(input.len() + length, 0);
    };

    let mut original = Vec::new();
    let mut input = Vec::new();

    skip_frame(&mut input, 300);
    input.extend_from_slice(include_bytes!("../../decodecorpus_files/z000089.zst"));
    original.extend_from_slice(include_bytes!("../../decodecorpus_files/z000089"));
    skip_frame(&mut input, 400);
    input.extend_from_slice(include_bytes!("../../decodecorpus_files/z000090.zst"));
    original.extend_from_slice(include_bytes!("../../decodecorpus_files/z000090"));
    skip_frame(&mut input, 500);

    let mut decoder = FrameDecoder::new();

    // decode_all with correct buffers.
    let mut output = vec![0; original.len()];
    let result = decoder.decode_all(&input, &mut output).unwrap();
    assert_eq!(result, original.len());
    assert_eq!(output, original);

    // decode_all with smaller output length.
    let mut output = vec![0; original.len() - 1];
    let result = decoder.decode_all(&input, &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::TargetTooSmall)),
        "{:?}",
        result
    );

    // decode_all with larger output length.
    let mut output = vec![0; original.len() + 1];
    let result = decoder.decode_all(&input, &mut output).unwrap();
    assert_eq!(result, original.len());
    assert_eq!(&output[..result], original);

    // decode_all with truncated regular frame.
    let mut output = vec![0; original.len()];
    let result = decoder.decode_all(&input[..input.len() - 600], &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::FailedToReadBlockBody(_))),
        "{:?}",
        result
    );

    // decode_all with truncated skip frame.
    let mut output = vec![0; original.len()];
    let result = decoder.decode_all(&input[..input.len() - 1], &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::FailedToSkipFrame)),
        "{:?}",
        result
    );

    // decode_all_to_vec with correct output capacity.
    let mut output = Vec::new();
    output.reserve_exact(original.len());
    decoder.decode_all_to_vec(&input, &mut output).unwrap();
    assert_eq!(output, original);

    // decode_all_to_vec with smaller output capacity.
    let mut output = Vec::new();
    output.reserve_exact(original.len() - 1);
    let result = decoder.decode_all_to_vec(&input, &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::TargetTooSmall)),
        "{:?}",
        result
    );

    // decode_all_to_vec with larger output capacity.
    let mut output = Vec::new();
    output.reserve_exact(original.len() + 1);
    decoder.decode_all_to_vec(&input, &mut output).unwrap();
    assert_eq!(output, original);
}

/// Plaintext that `test_fixtures/window_128mib.zst` and
/// `test_fixtures/window_256mib.zst` decode to. The 128mib window fixture was
/// produced with `zstd --long=27` (a 128 MiB window descriptor) from this exact
/// content, so the bytes must match what is generated here. The 256mib window
/// fixture was created by editing the zstd header to claim the larger window
#[cfg(test)]
fn window_128_and_256mib_plaintext() -> Vec<u8> {
    "The quick brown fox jumps over the lazy dog.\n"
        .repeat(4096)
        .into_bytes()
}

/// Plaintext that `test_fixtures/window_8mib.zst` decodes to. Produced with
/// `zstd --long=23` (an 8 MiB window descriptor), below the default limit.
#[cfg(test)]
fn window_8mib_plaintext() -> Vec<u8> {
    "Sphinx of black quartz, judge my vow.\n"
        .repeat(4096)
        .into_bytes()
}

#[test]
fn test_large_window_decodes_when_limit_raised() {
    use crate::decoding::FrameDecoder;

    // 256 MiB window, above the default 128 MiB limit.
    let compressed = include_bytes!("../../test_fixtures/window_256mib.zst");
    let expected = window_128_and_256mib_plaintext();

    let mut decoder = FrameDecoder::new();
    decoder.set_max_window_size(300 * 1024 * 1024);
    assert_eq!(decoder.max_window_size(), 300 * 1024 * 1024);

    let mut output = vec![0u8; expected.len()];
    let written = decoder.decode_all(compressed, &mut output).unwrap();
    assert_eq!(written, expected.len());
    assert_eq!(output, expected);
}

#[test]
fn test_large_window_rejected_at_default_limit() {
    use crate::decoding::errors::FrameDecoderError;
    use crate::decoding::{DEFAULT_MAX_WINDOW_SIZE, FrameDecoder};

    let compressed = include_bytes!("../../test_fixtures/window_256mib.zst");

    let mut decoder = FrameDecoder::new();
    assert_eq!(decoder.max_window_size(), DEFAULT_MAX_WINDOW_SIZE);

    let mut output = vec![0u8; window_128_and_256mib_plaintext().len()];
    let result = decoder.decode_all(compressed, &mut output);
    // The reported max must be the effective configured limit, not the spec maximum.
    assert!(
        matches!(
            result,
            Err(FrameDecoderError::WindowSizeTooBig { requested, max })
                if requested == 256 * 1024 * 1024 && max == DEFAULT_MAX_WINDOW_SIZE
        ),
        "{:?}",
        result
    );
}

#[test]
fn test_multi_frame_large_window_decodes_when_raised() {
    use crate::decoding::FrameDecoder;

    // Two large-window frames back to back exercise both the first-frame
    // (FrameDecoderState::new) and later-frame (FrameDecoderState::reset) paths,
    // which historically applied the window check inconsistently.
    let frame = include_bytes!("../../test_fixtures/window_256mib.zst");
    let single = window_128_and_256mib_plaintext();

    let mut input = Vec::new();
    input.extend_from_slice(frame);
    input.extend_from_slice(frame);

    let mut expected = Vec::new();
    expected.extend_from_slice(&single);
    expected.extend_from_slice(&single);

    let mut decoder = FrameDecoder::new();
    decoder.set_max_window_size(300 * 1024 * 1024);

    let mut output = vec![0u8; expected.len()];
    let written = decoder.decode_all(&input, &mut output).unwrap();
    assert_eq!(written, expected.len());
    assert_eq!(output, expected);
}

#[test]
fn test_large_window_rejected_on_first_and_later_frames() {
    use crate::decoding::FrameDecoder;
    use crate::decoding::errors::FrameDecoderError;

    let big = include_bytes!("../../test_fixtures/window_256mib.zst"); // 256 MiB window
    let small = include_bytes!("../../test_fixtures/window_8mib.zst"); // 8 MiB window

    // First frame: a single 256 MiB-window frame is rejected under the default limit.
    let mut decoder = FrameDecoder::new();
    let mut output = vec![0u8; window_128_and_256mib_plaintext().len()];
    let result = decoder.decode_all(big, &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::WindowSizeTooBig { .. })),
        "first frame should be rejected: {:?}",
        result
    );

    // Later frame: a small-window frame decodes, then a 256 MiB-window frame is
    // rejected under the default limit, covering the FrameDecoderState::reset path.
    let mut input = Vec::new();
    input.extend_from_slice(small);
    input.extend_from_slice(big);

    let mut decoder = FrameDecoder::new();
    let mut output =
        vec![0u8; window_8mib_plaintext().len() + window_128_and_256mib_plaintext().len()];
    let result = decoder.decode_all(&input, &mut output);
    assert!(
        matches!(result, Err(FrameDecoderError::WindowSizeTooBig { .. })),
        "later frame should be rejected: {:?}",
        result
    );
}

#[test]
#[cfg(feature = "std")]
fn test_streaming_decoder_max_window_size() {
    use crate::decoding::StreamingDecoder;
    use crate::decoding::errors::FrameDecoderError;
    use std::io::Read;

    let compressed = include_bytes!("../../test_fixtures/window_256mib.zst");
    let expected = window_128_and_256mib_plaintext();

    // The default StreamingDecoder rejects the 256 MiB window on init.
    match StreamingDecoder::new(compressed.as_slice()) {
        Err(FrameDecoderError::WindowSizeTooBig { .. }) => {}
        Err(e) => panic!("expected WindowSizeTooBig, got {:?}", e),
        Ok(_) => panic!("expected WindowSizeTooBig, got a decoder"),
    }

    // Raising the limit lets the same frame decode through the streaming wrapper.
    let mut stream =
        StreamingDecoder::new_with_max_window_size(compressed.as_slice(), 300 * 1024 * 1024)
            .unwrap();
    let mut result = Vec::new();
    Read::read_to_end(&mut stream, &mut result).unwrap();
    assert_eq!(result, expected);
}

#[test]
fn test_max_window_size_clamped_to_format_maximum() {
    use crate::common::MAX_WINDOW_SIZE;
    use crate::decoding::FrameDecoder;

    let mut decoder = FrameDecoder::new();

    // A value below the format maximum is kept verbatim.
    decoder.set_max_window_size(2 * 1024 * 1024 * 1024);
    assert_eq!(decoder.max_window_size(), 2 * 1024 * 1024 * 1024);

    // A value above the format maximum is clamped down to it.
    decoder.set_max_window_size(u64::MAX);
    assert_eq!(decoder.max_window_size(), MAX_WINDOW_SIZE);
}

/// Frame header with a 1 MiB window, which makes Block_Maximum_Size 128 KiB.
#[cfg(test)]
const FRAME_HEADER_1MIB_WINDOW: [u8; 6] = [0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x50];

/// A compressed block whose literals section is `n` RLE copies of `b'x'`.
/// `sequences` is appended as the sequences section.
#[cfg(test)]
fn rle_literals_block(n: u32, sequences: &[u8]) -> Vec<u8> {
    let mut content = vec![
        0x0d | ((n & 0xf) << 4) as u8,
        (n >> 4) as u8,
        (n >> 12) as u8,
        b'x',
    ];
    content.extend_from_slice(sequences);
    let header = (content.len() as u32) << 3 | 2 << 1;
    let mut block = header.to_le_bytes()[..3].to_vec();
    block.extend_from_slice(&content);
    block
}

#[test]
fn test_literals_only_block_bounded_by_block_maximum_size() {
    use crate::common::MAX_BLOCK_SIZE;
    use crate::decoding::FrameDecoder;
    use crate::decoding::errors::{
        DecodeBlockContentError, DecompressBlockError, FrameDecoderError,
    };

    let decode = |literals: u32| {
        let mut input = FRAME_HEADER_1MIB_WINDOW.to_vec();
        input.extend(rle_literals_block(literals, &[0x00])); // no sequences
        input.extend_from_slice(&[0x01, 0x00, 0x00]); // empty last raw block
        FrameDecoder::new().decode_all(&input, &mut vec![0; 4 << 20])
    };

    assert!(decode(MAX_BLOCK_SIZE).is_ok());
    for literals in [MAX_BLOCK_SIZE + 1, (1 << 20) - 1] {
        let result = decode(literals);
        assert!(
            matches!(
                result,
                Err(FrameDecoderError::FailedToReadBlockBody(
                    DecodeBlockContentError::DecompressBlockError(
                        DecompressBlockError::DecompressedSizeTooLarge { .. }
                    )
                ))
            ),
            "{:?}",
            result
        );
    }
}

#[test]
fn test_literals_left_after_sequences_bounded_by_block_maximum_size() {
    use crate::common::MAX_BLOCK_SIZE;
    use crate::decoding::FrameDecoder;
    use crate::decoding::errors::{
        DecodeBlockContentError, DecompressBlockError, ExecuteSequencesError, FrameDecoderError,
    };

    let decode = |literals: u32| {
        let mut input = FRAME_HEADER_1MIB_WINDOW.to_vec();
        // a raw block of 4 bytes, for the sequence's match to copy from
        input.extend_from_slice(&[0x20, 0x00, 0x00, 0, 0, 0, 0]);
        // one sequence from RLE tables (LL 0, OF code 0, ML 3): copies 3 bytes and
        // leaves all of the literals to be appended after it
        input.extend(rle_literals_block(
            literals,
            &[0x01, 0x54, 0x00, 0x00, 0x00, 0x01],
        ));
        input.extend_from_slice(&[0x01, 0x00, 0x00]); // empty last raw block
        FrameDecoder::new().decode_all(&input, &mut vec![0; 4 << 20])
    };

    assert!(decode(MAX_BLOCK_SIZE - 3).is_ok());
    let result = decode(MAX_BLOCK_SIZE - 2);
    assert!(
        matches!(
            result,
            Err(FrameDecoderError::FailedToReadBlockBody(
                DecodeBlockContentError::DecompressBlockError(
                    DecompressBlockError::ExecuteSequencesError(
                        ExecuteSequencesError::TooManyBytesGenerated { .. }
                    )
                )
            ))
        ),
        "{:?}",
        result
    );
    assert!(decode((1 << 20) - 1).is_err());
}

pub mod bit_reader;
pub mod decode_corpus;
pub mod dict_test;
pub mod fuzz_regressions;

#[cfg(feature = "std")]
#[test]
fn verbose_disabled() {
    use crate::VERBOSE;
    const { assert!(!VERBOSE) };
}

/// Collecting gives every byte as soon as its block is decoded, long before the frame
/// ends, and the blocks after still copy from the window: a frame read a block at a time,
/// collected or read after each, comes out whole.
#[cfg(not(target_family = "wasm"))]
#[test]
fn collect_gives_each_block_as_decoded() {
    use crate::decoding::{BlockDecodingStrategy, FrameDecoder};
    use crate::io::Read;

    // Several blocks' worth, which does not all repeat.
    let data: Vec<u8> = (0..400_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8 ^ (i / 1000) as u8)
        .collect();
    // Compressed by libzstd at level 1, which the zstd crate binds.
    let compressed = zstd::bulk::compress(&data, 1).unwrap();
    let mut decoder = FrameDecoder::new();
    let mut source = &compressed[..];
    decoder.init(&mut source).unwrap();
    assert_eq!(decoder.can_collect(), 0);
    let mut out = Vec::new();
    let mut blocks = 0;
    while !decoder.is_finished() {
        decoder
            .decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1))
            .unwrap();
        blocks += 1;
        let new = decoder.can_collect();
        assert!(new > 0);
        // Every other block read in pieces, the rest collected at once.
        if blocks % 2 == 0 {
            let mut piece = [0; 1000];
            while decoder.can_collect() > 0 {
                let n = decoder.read(&mut piece).unwrap();
                out.extend_from_slice(&piece[..n]);
            }
        } else {
            out.extend_from_slice(&decoder.collect().unwrap());
        }
        assert_eq!(decoder.can_collect(), 0);
        assert_eq!(out, data[..out.len()]);
    }
    assert!(blocks > 2, "{blocks}");
    assert_eq!(decoder.collect().unwrap(), []);
    assert_eq!(out, data);
}

/// The checksum covers every byte decoded, however much of it was collected and when:
/// collected a block at a time, or not at all until the frame is done. A frame without a
/// checksum has none calculated, and decodes the same; one decoder takes the frames in turn,
/// with and without, so that a reset starts the hash anew or leaves it off.
#[cfg(feature = "hash")]
#[cfg(not(target_family = "wasm"))]
#[test]
fn checksum_whatever_was_collected() {
    use crate::decoding::{BlockDecodingStrategy, FrameDecoder};
    use std::io::Write;

    let data: Vec<u8> = (0..300_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8 ^ (i / 700) as u8)
        .collect();
    let compress = |checksum| {
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
        encoder.include_checksum(checksum).unwrap();
        // A window smaller than the data, so that collected bytes are dropped on the way.
        encoder.window_log(17).unwrap();
        encoder.write_all(&data).unwrap();
        encoder.finish().unwrap()
    };
    let (with, without) = (compress(true), compress(false));
    let mut decoder = FrameDecoder::new();
    for (checksum, collect_each_block) in
        [(true, true), (false, true), (true, false), (false, false)]
    {
        let mut source = if checksum { &with[..] } else { &without[..] };
        decoder.init(&mut source).unwrap();
        let mut out = Vec::new();
        while !decoder.is_finished() {
            decoder
                .decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1))
                .unwrap();
            if collect_each_block {
                out.extend(decoder.collect().unwrap());
            }
        }
        let expected = decoder.get_checksum_from_data();
        assert_eq!(expected.is_some(), checksum);
        assert_eq!(decoder.get_calculated_checksum(), expected);
        out.extend(decoder.collect().unwrap());
        assert_eq!(decoder.get_calculated_checksum(), expected);
        assert!(out == data, "{checksum} {collect_each_block}");
    }
}

/// A block that fails to decode adds nothing to the output: after the error there is
/// nothing to collect, everything before it having been collected. Sequences are executed
/// as they are decoded, so a block whose bitstream turns out bad at its end has to be undone.
#[cfg(not(target_family = "wasm"))]
#[test]
fn failed_block_adds_no_output() {
    use crate::decoding::{BlockDecodingStrategy, FrameDecoder};

    let data: Vec<u8> = (0..160_000u32)
        .map(|i| ((i.wrapping_mul(2_654_435_761) >> 24) as u8 % 7) ^ (i / 1000) as u8)
        .collect();
    let compressed = zstd::bulk::compress(&data, 3).unwrap();
    let mut failed_with_output_before = 0;
    for at in (0..compressed.len()).step_by(31) {
        for bit in [0x01u8, 0x80] {
            let mut corrupt = compressed.clone();
            corrupt[at] ^= bit;
            let mut decoder = FrameDecoder::new();
            let mut source = &corrupt[..];
            if decoder.init(&mut source).is_err() {
                continue;
            }
            let mut out = Vec::new();
            while !decoder.is_finished() {
                out.extend_from_slice(&decoder.collect().unwrap());
                match decoder.decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1)) {
                    Ok(_) => {}
                    Err(_) => {
                        assert_eq!(decoder.can_collect(), 0, "byte {at} bit {bit:#x}");
                        failed_with_output_before += usize::from(!out.is_empty());
                        break;
                    }
                }
            }
        }
    }
    // Errors past the first block were reached, not only ones in the frame header.
    assert!(failed_with_output_before > 0);
}

/// The small files of the corpus, and those compressed with the small dictionary, decoded and
/// checked against their originals: every path of the crate's unsafe loops, the sequences' and
/// the Huffman streams', on files small enough for Miri, which checks their pointers (see
/// AGENTS.md; it reads the files with isolation off).
#[test]
#[cfg(feature = "std")]
fn small_files_decode() {
    use crate::decoding::{Dictionary, FrameDecoder, StreamingDecoder};
    use std::io::Read;

    let mut files: Vec<_> = std::fs::read_dir("./decodecorpus_files")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "zst")
                && std::fs::metadata(path).unwrap().len() <= 5_000
        })
        .collect();
    files.sort();
    assert!(files.len() > 50, "{} files", files.len());
    for path in &files {
        let compressed = std::fs::read(path).unwrap();
        let original = std::fs::read(path.with_extension("")).unwrap();
        let mut decoder = StreamingDecoder::new(&compressed[..]).unwrap();
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        assert!(out == original, "{}", path.display());
    }

    let dict = std::fs::read("./dict_tests/dictionary-small").unwrap();
    let mut frame_decoder = FrameDecoder::new();
    frame_decoder
        .add_dict(Dictionary::decode_dict(&dict).unwrap())
        .unwrap();
    let mut small: Vec<_> = std::fs::read_dir("./dict_tests/files-small")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    small.sort();
    assert!(!small.is_empty());
    for path in &small {
        let compressed = std::fs::read(path).unwrap();
        let name = path.file_stem().unwrap();
        let original =
            std::fs::read(std::path::Path::new("./dict_tests/files").join(name)).unwrap();
        let mut decoder =
            StreamingDecoder::new_with_decoder(&compressed[..], &mut frame_decoder).unwrap();
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        assert!(out == original, "{}", path.display());
    }
}
