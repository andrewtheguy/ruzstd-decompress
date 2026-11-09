//! A frame is held to the content size its header gives: one decoding to more fails at the
//! block that would take it past, writing nothing past it, and one decoding to less fails at its
//! end. And [`decompressed_bound`], what the headers say the frames decode to, and
//! [`frame_compressed_size`], where they say the first frame ends.
use crate::decoding::errors::FrameDecoderError;
use crate::decoding::{
    BlockDecodingStrategy, FrameDecoder, decompressed_bound, frame_compressed_size,
};
use alloc::vec::Vec;

/// A block of a hand-made frame.
pub enum Block<'a> {
    Raw(&'a [u8]),
    /// A byte, repeated.
    Rle(u8, usize),
}

/// A frame of `blocks`, of a 64 KiB window, whose header gives `content_size` (from 256 to
/// 65791 bytes) if it is `Some`.
pub fn frame(blocks: &[Block<'_>], content_size: Option<usize>) -> Vec<u8> {
    let mut out = alloc::vec![0x28, 0xB5, 0x2F, 0xFD];
    // Frame_Content_Size of 2 bytes, or none; not a single segment, so a window descriptor.
    out.push(if content_size.is_some() { 0x40 } else { 0x00 });
    out.push(6 << 3);
    if let Some(size) = content_size {
        out.extend_from_slice(&u16::try_from(size - 256).unwrap().to_le_bytes());
    }
    for (i, block) in blocks.iter().enumerate() {
        let last = u32::from(i + 1 == blocks.len());
        let (kind, size, content): (u32, usize, &[u8]) = match block {
            Block::Raw(data) => (0, data.len(), data),
            Block::Rle(byte, len) => (1, *len, core::slice::from_ref(byte)),
        };
        let header = last | (kind << 1) | ((size as u32) << 3);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(content);
    }
    out
}

/// What `frame` decodes to a block at a time, collected after each, as far as it does; and the
/// error that stopped it, after which there is nothing left to collect.
fn decode_by_blocks(frame: &[u8]) -> (Vec<u8>, Option<FrameDecoderError>) {
    let mut source = frame;
    let mut decoder = FrameDecoder::new();
    decoder.init(&mut source).unwrap();
    let mut out = Vec::new();
    while !decoder.is_finished() {
        if let Err(err) = decoder.decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1)) {
            assert_eq!(decoder.can_collect(), 0, "a failed block added output");
            return (out, Some(err));
        }
        out.extend(decoder.collect().unwrap());
    }
    (out, None)
}

#[test]
fn held_to_the_header() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let (first, second) = data.split_at(600);
    let blocks = [Block::Raw(first), Block::Raw(second)];

    let (out, err) = decode_by_blocks(&frame(&blocks, Some(1000)));
    assert!(err.is_none() && out == data);
    let (out, err) = decode_by_blocks(&frame(&blocks, None));
    assert!(err.is_none() && out == data);

    // The second block would take it past 900: it fails, having written nothing.
    let (out, err) = decode_by_blocks(&frame(&blocks, Some(900)));
    assert!(
        matches!(
            err,
            Some(FrameDecoderError::ContentSizeExceeded { content_size: 900 })
        ),
        "{err:?}"
    );
    assert_eq!(out, first);

    // It ends short of 1100: its last block fails, taken back.
    let (out, err) = decode_by_blocks(&frame(&blocks, Some(1100)));
    assert!(
        matches!(
            err,
            Some(FrameDecoderError::ContentSizeShort {
                content_size: 1100,
                decoded: 1000
            })
        ),
        "{err:?}"
    );
    assert_eq!(out, first);

    // An RLE block, the first, past 500.
    let (out, err) = decode_by_blocks(&frame(&[Block::Rle(7, 600)], Some(500)));
    assert!(
        matches!(
            err,
            Some(FrameDecoderError::ContentSizeExceeded { content_size: 500 })
        ),
        "{err:?}"
    );
    assert!(out.is_empty());

    // The decoder on a slice fails the same way.
    let mut decoder = FrameDecoder::new();
    let mut target = alloc::vec![0; 2000];
    let result = decoder.decode_all(&frame(&blocks, Some(900)), &mut target);
    assert!(
        matches!(
            result,
            Err(FrameDecoderError::ContentSizeExceeded { content_size: 900 })
        ),
        "{result:?}"
    );
}

#[test]
fn bound_from_the_headers() {
    let data = [3u8; 700];
    let sized = frame(&[Block::Raw(&data), Block::Rle(1, 300)], Some(1000));
    assert_eq!(decompressed_bound(&sized).unwrap(), 1000);
    // No content size: two blocks of at most the 64 KiB window each.
    let unsized_ = frame(&[Block::Raw(&data), Block::Rle(1, 300)], None);
    assert_eq!(decompressed_bound(&unsized_).unwrap(), 2 << 16);

    let mut both = sized.clone();
    both.extend_from_slice(&0x184D2A53u32.to_le_bytes());
    both.extend_from_slice(&2u32.to_le_bytes());
    both.extend_from_slice(b"ok");
    both.extend_from_slice(&unsized_);
    assert_eq!(decompressed_bound(&both).unwrap(), 1000 + (2 << 16));
    assert_eq!(decompressed_bound(&[]).unwrap(), 0);

    // Cut short anywhere, it is no bound.
    for cut in 1..sized.len() {
        assert!(decompressed_bound(&sized[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn first_frames_length_from_the_headers() {
    let data = [3u8; 700];
    let sized = frame(&[Block::Raw(&data), Block::Rle(1, 300)], Some(1000));
    assert_eq!(frame_compressed_size(&sized).unwrap(), sized.len());

    // What follows a frame is not of it, a skippable frame or another frame.
    let skippable = [&0x184D2A53u32.to_le_bytes()[..], &2u32.to_le_bytes(), b"ok"].concat();
    let followed = [&sized[..], &skippable, &sized].concat();
    assert_eq!(frame_compressed_size(&followed).unwrap(), sized.len());
    // A skippable frame is one.
    assert_eq!(
        frame_compressed_size(&[&skippable[..], &sized].concat()).unwrap(),
        skippable.len()
    );

    // Cut short anywhere, or empty, it begins with no whole frame.
    for cut in 0..sized.len() {
        assert!(
            frame_compressed_size(&sized[..cut]).is_err(),
            "cut at {cut}"
        );
    }

    // A frame's checksum, after its last block, is of it.
    let mut checksummed = sized.clone();
    checksummed[4] |= 0x04;
    checksummed.extend_from_slice(&[1, 2, 3, 4]);
    let followed = [&checksummed[..], &sized].concat();
    assert_eq!(frame_compressed_size(&followed).unwrap(), checksummed.len());
    for cut in sized.len()..checksummed.len() {
        assert!(
            frame_compressed_size(&checksummed[..cut]).is_err(),
            "cut at {cut}"
        );
    }
}
