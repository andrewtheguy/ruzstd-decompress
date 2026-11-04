//! This module contains the decompress_literals function, used to take a
//! parsed literals header and a source and decompress it.

use super::super::blocks::literals_section::{LiteralsSection, LiteralsSectionType};
use super::scratch::HuffmanScratch;
use crate::bit_io::BitReaderReversed;
use crate::decoding::errors::DecompressLiteralsError;
use alloc::vec::Vec;

/// Decode and decompress the provided literals section into `target`, returning the number of bytes read.
pub fn decode_literals(
    section: &LiteralsSection,
    scratch: &mut HuffmanScratch,
    source: &[u8],
    target: &mut Vec<u8>,
) -> Result<u32, DecompressLiteralsError> {
    match section.ls_type {
        LiteralsSectionType::Raw => {
            target.extend(&source[0..section.regenerated_size as usize]);
            Ok(section.regenerated_size)
        }
        LiteralsSectionType::RLE => {
            target.resize(target.len() + section.regenerated_size as usize, source[0]);
            Ok(1)
        }
        LiteralsSectionType::Compressed | LiteralsSectionType::Treeless => {
            let bytes_read = decompress_literals(section, scratch, source, target)?;

            //return sum of used bytes
            Ok(bytes_read)
        }
    }
}

/// Decompress the provided literals section and source into the provided `target`.
/// This function is used when the literals section is `Compressed` or `Treeless`
///
/// Returns the number of bytes read.
fn decompress_literals(
    section: &LiteralsSection,
    scratch: &mut HuffmanScratch,
    source: &[u8],
    target: &mut Vec<u8>,
) -> Result<u32, DecompressLiteralsError> {
    use DecompressLiteralsError as err;

    let compressed_size = section.compressed_size.ok_or(err::MissingCompressedSize)? as usize;
    let num_streams = section.num_streams.ok_or(err::MissingNumStreams)?;

    target.reserve(section.regenerated_size as usize);
    let source = &source[0..compressed_size];
    let mut bytes_read = 0;

    match section.ls_type {
        LiteralsSectionType::Compressed => {
            //read Huffman tree description
            bytes_read += scratch.table.build_decoder(source)?;
            vprintln!("Built huffman table using {} bytes", bytes_read);
        }
        LiteralsSectionType::Treeless if scratch.table.max_num_bits == 0 => {
            return Err(err::UninitializedHuffmanTable);
        }

        _ => { /* nothing to do, huffman tree has been provided by previous block */ }
    }

    let source = &source[bytes_read as usize..];

    let max_num_bits = scratch.table.max_num_bits;
    if max_num_bits == 0 {
        return Err(err::UninitializedHuffmanTable);
    }
    let regenerated_size = section.regenerated_size as usize;
    let start = target.len();
    target.resize(start + regenerated_size, 0);
    let out = &mut target[start..];

    if num_streams == 4 {
        //build jumptable
        if source.len() < 6 {
            return Err(err::MissingBytesForJumpHeader { got: source.len() });
        }
        let jump1 = source[0] as usize + ((source[1] as usize) << 8);
        let jump2 = jump1 + source[2] as usize + ((source[3] as usize) << 8);
        let jump3 = jump2 + source[4] as usize + ((source[5] as usize) << 8);
        bytes_read += 6;
        let source = &source[6..];

        if source.len() < jump3 {
            return Err(err::MissingBytesForLiterals {
                got: source.len(),
                needed: jump3,
            });
        }

        // The first three streams each decode to (regenerated_size + 3) / 4 bytes,
        // the fourth one to the rest.
        // Like libzstd (MIN_LITERALS_FOR_4_STREAMS), reject sizes this split doesn't work for.
        if regenerated_size < 6 {
            return Err(err::DecodedLiteralCountMismatch {
                decoded: 0,
                expected: regenerated_size,
            });
        }
        let segment_size = regenerated_size.div_ceil(4);
        debug_assert!(3 * segment_size <= regenerated_size);
        let (out0, rest) = out.split_at_mut(segment_size);
        let (out1, rest) = rest.split_at_mut(segment_size);
        let (out2, out3) = rest.split_at_mut(segment_size);

        let mut brs = [
            init_stream(&source[..jump1])?,
            init_stream(&source[jump1..jump2])?,
            init_stream(&source[jump2..jump3])?,
            init_stream(&source[jump3..])?,
        ];
        scratch
            .table
            .decode_4_streams(&mut brs, [out0, out1, out2, out3]);
        for br in &brs {
            check_stream_end(br, max_num_bits)?;
        }

        bytes_read += source.len() as u32;
    } else {
        //just decode the one stream
        assert!(num_streams == 1);
        let mut br = init_stream(source)?;
        scratch.table.decode_stream(&mut br, out);
        check_stream_end(&br, max_num_bits)?;
        bytes_read += source.len() as u32;
    }

    if target.len() != section.regenerated_size as usize {
        return Err(DecompressLiteralsError::DecodedLiteralCountMismatch {
            decoded: target.len(),
            expected: section.regenerated_size as usize,
        });
    }

    Ok(bytes_read)
}

/// Create a reader for a Huffman stream, skipping the 0 padding at the end of the last byte
/// and the first 1 found.
fn init_stream(stream: &[u8]) -> Result<BitReaderReversed<'_>, DecompressLiteralsError> {
    let mut br = BitReaderReversed::new(stream);
    let mut skipped_bits = 0;
    loop {
        let val = br.get_bits(1);
        skipped_bits += 1;
        if val == 1 || skipped_bits > 8 {
            break;
        }
    }
    if skipped_bits > 8 {
        //if more than 7 bits are 0, this is not the correct end of the bitstream. Either a bug or corrupted data
        return Err(DecompressLiteralsError::ExtraPadding { skipped_bits });
    }
    Ok(br)
}

/// A stream must be used up exactly by the symbols it decodes to.
fn check_stream_end(
    br: &BitReaderReversed<'_>,
    max_num_bits: u8,
) -> Result<(), DecompressLiteralsError> {
    if br.bits_remaining() != 0 {
        // Reported the way the previous state based decoder counted, which had already
        // read max_num_bits bits ahead.
        return Err(DecompressLiteralsError::BitstreamReadMismatch {
            read_til: br.bits_remaining() - max_num_bits as isize,
            expected: -(max_num_bits as isize),
        });
    }
    Ok(())
}
