//! This module contains the decompress_literals function, used to take a
//! parsed literals header and a source and decompress it.

use super::super::blocks::literals_section::{LiteralsSection, LiteralsSectionType};
use super::scratch::HuffmanScratch;
use crate::bit_io::BitReaderReversed;
use crate::decoding::errors::DecompressLiteralsError;
use crate::huff0::HuffmanTable;
use alloc::vec::Vec;

/// Decode and decompress the provided literals section into `target`, returning the number of bytes read.
pub fn decode_literals(
    section: &LiteralsSection,
    scratch: &mut HuffmanScratch,
    source: &[u8],
    target: &mut Vec<u8>,
) -> Result<u32, DecompressLiteralsError> {
    let tree_bytes = prepare_literals(section, &mut scratch.table, source)?;
    decode_literals_with(section, &scratch.table, tree_bytes, source, target)
}

/// What a literals section needs of `table` before its literals decode from it: the table built
/// from its description, if it has one, and whatever else it decodes from (see
/// [`HuffmanTable::prepare_4_streams`]). Returns the description's bytes.
pub fn prepare_literals(
    section: &LiteralsSection,
    table: &mut HuffmanTable,
    source: &[u8],
) -> Result<u32, DecompressLiteralsError> {
    use DecompressLiteralsError as err;
    let tree_bytes = match section.ls_type {
        LiteralsSectionType::Raw | LiteralsSectionType::RLE => return Ok(0),
        LiteralsSectionType::Compressed => {
            let compressed_size =
                section.compressed_size.ok_or(err::MissingCompressedSize)? as usize;
            //read Huffman tree description
            let bytes = table.build_decoder(&source[..compressed_size])?;
            vprintln!("Built huffman table using {} bytes", bytes);
            bytes
        }
        LiteralsSectionType::Treeless => 0,
    };
    if table.max_num_bits == 0 {
        return Err(err::UninitializedHuffmanTable);
    }
    if let Some((regenerated, compressed)) = four_streams(section) {
        table.prepare_4_streams(regenerated, compressed);
    }
    Ok(tree_bytes)
}

/// Whether [`prepare_literals`] would change `table` for this section, which has no tree
/// description of its own.
#[cfg(feature = "rayon")]
pub fn literals_need_prepare(section: &LiteralsSection, table: &HuffmanTable) -> bool {
    table.max_num_bits != 0
        && four_streams(section).is_some_and(|(r, c)| table.needs_prepare_4_streams(r, c))
}

/// A Huffman section's regenerated and compressed sizes, if it has four streams.
fn four_streams(section: &LiteralsSection) -> Option<(usize, usize)> {
    match section.ls_type {
        LiteralsSectionType::Compressed | LiteralsSectionType::Treeless
            if section.num_streams == Some(4) =>
        {
            Some((
                section.regenerated_size as usize,
                section.compressed_size? as usize,
            ))
        }
        _ => None,
    }
}

/// Decode a literals section into `target` with `table`, which [`prepare_literals`] has made
/// ready for it, `tree_bytes` the description it read: returns the number of bytes read.
pub fn decode_literals_with(
    section: &LiteralsSection,
    table: &HuffmanTable,
    tree_bytes: u32,
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
            decompress_literals(section, table, tree_bytes, source, target)
        }
    }
}

/// Decompress the provided literals section and source into the provided `target`.
/// This function is used when the literals section is `Compressed` or `Treeless`
///
/// Returns the number of bytes read.
fn decompress_literals(
    section: &LiteralsSection,
    table: &HuffmanTable,
    tree_bytes: u32,
    source: &[u8],
    target: &mut Vec<u8>,
) -> Result<u32, DecompressLiteralsError> {
    use DecompressLiteralsError as err;

    let compressed_size = section.compressed_size.ok_or(err::MissingCompressedSize)? as usize;
    let num_streams = section.num_streams.ok_or(err::MissingNumStreams)?;

    target.reserve(section.regenerated_size as usize);
    let source = &source[0..compressed_size];
    let mut bytes_read = tree_bytes;
    let source = &source[bytes_read as usize..];

    let max_num_bits = table.max_num_bits;
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
        table.decode_4_streams_sized(&mut brs, [out0, out1, out2, out3], compressed_size);
        for br in &brs {
            check_stream_end(br, max_num_bits)?;
        }

        bytes_read += source.len() as u32;
    } else {
        //just decode the one stream
        assert!(num_streams == 1);
        let mut br = init_stream(source)?;
        table.decode_stream(&mut br, out);
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
