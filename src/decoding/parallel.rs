//! Decoding on several threads, those of the rayon pool it is called in: what
//! [`FrameDecoder::decode_all_parallel`] and [`FrameDecoder::decode_each_parallel`] do.
//!
//! The frames of the input are independent, and are decoded side by side, a run of them to each
//! job; so are separate inputs, each to its own output. Within a frame, a block's matches copy
//! from the blocks before it, so its sequences must be executed in order; but what makes them,
//! the literals' Huffman streams and the sequences' FSE streams, needs only the block's tables.
//! So a frame of many blocks, when there are fewer frames than threads, is decoded in three
//! steps:
//!
//! 1. the tables, block after block, on one thread, as a block may repeat the one before's;
//! 2. each block's literals and sequences, decoded into buffers of its own, a batch of blocks side
//!    by side;
//! 3. the sequences executed into the frame's window, block after block, on one thread.
//!
//! A batch's execution runs beside the next batch's first two steps.
//!
//! Every output is decoded to its spare capacity and no further: each block is held to what is
//! left of it, and of its frame's content size where the header gives one, before it writes.
//!
//! [`FrameDecoder::decode_all_parallel`]: super::FrameDecoder::decode_all_parallel
//! [`FrameDecoder::decode_each_parallel`]: super::FrameDecoder::decode_each_parallel

use super::block_decoder::{
    self, BlockDecoder, BlockSections, check_literals, check_no_sequences, max_block_len,
    split_sections,
};
use super::decode_buffer::LITERALS_PADDING;
use super::dictionary::Dictionary;
use super::errors::{
    DecodeBlockContentError, DecompressBlockError, FrameDecoderError, ReadFrameHeaderError,
};
use super::frame::{self, FrameHeader};
use super::frame_decoder::{block_failed, check_content_size, check_window_size};
use super::literals_section_decoder::{
    decode_literals_with, literals_need_prepare, prepare_literals,
};
use super::scratch::{DecoderScratch, FSEScratch};
use super::sequence_execution::{SequenceExecutor, SequenceStore, execute_sequences};
use super::sequence_section_decoder::{decode_sequences_with, update_fse_tables};
use super::sequence_table::SeqTables;
use crate::blocks::block::BlockType;
use crate::blocks::literals_section::LiteralsSectionType;
use crate::blocks::sequence_section::{ModeType, Sequence};
use crate::huff0::HuffmanTable;
use crate::io::{Error, ErrorKind, Read};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use rayon::prelude::*;

/// The buffers of blocks executed, for the next blocks' literals and sequences.
type Spare = Vec<(Vec<u8>, Vec<Sequence>)>;

/// A batch of a frame's blocks is about this many bytes of them to each of the pool's threads:
/// enough that a batch's handing from thread to thread, which in WebAssembly is a wait on the
/// shared memory and a wake, is small beside its decoding.
///
/// Tests and the fuzzer take small batches, for their small frames to be split as well.
const BATCH_BYTES_PER_THREAD: usize = if cfg!(any(test, fuzzing)) {
    1 << 10
} else {
    128 << 10
};

/// A batch's literals and sequences, decoded and held until they are executed, take at most
/// about this many bytes to each of the pool's threads, however few bytes of input make them: a
/// block of a few bytes can hold a block's worth of either.
const BATCH_STORE_PER_THREAD: usize = if cfg!(any(test, fuzzing)) {
    16 << 10
} else {
    2 << 20
};

/// Natively, a frame is decoded in place, into the output, where it is at least this many bytes
/// of input, or is split (see [`in_place`]). A smaller one is decoded into the decoder's ring,
/// which earlier frames have made and initialised, and copied out: a new window in the output is
/// initialised as it is written, which for a small frame costs more than the copy.
const IN_PLACE_LEAST: usize = 32 << 10;

/// Whether `frame` is decoded in place, into the output, rather than into the decoder's ring and
/// copied out; `split` if its blocks are decoded side by side, `alone` if one job decodes the
/// whole input.
///
/// Only where one job does: jobs side by side, each into its own output, were slower in place
/// on 8 threads than each from its ring, warm from frame to frame. And never in WebAssembly,
/// where it was slower on several threads, not for the initialisation. Tests and the fuzzer
/// take both ways, by the frame's length.
fn in_place(frame: &Frame<'_>, split: bool, alone: bool) -> bool {
    if cfg!(any(test, fuzzing)) {
        frame.body.len().is_multiple_of(2)
    } else {
        !cfg!(target_family = "wasm") && alone && (split || frame.body.len() >= IN_PLACE_LEAST)
    }
}

/// A run of frames, or of inputs, decoded by one job, is at least this many bytes of them.
const RUN_BYTES_LEAST: usize = 32 << 10;

/// The most room reserved for output before it is decoded, whatever the frames' headers say:
/// past it, the output grows as it is decoded.
const ROOM_MOST: usize = 256 << 20;

/// Room for what `frames` decode to, up to [`ROOM_MOST`].
fn run_room(frames: &[Frame<'_>]) -> usize {
    frames
        .iter()
        .map(Frame::room)
        .fold(0, usize::saturating_add)
        .min(ROOM_MOST)
}

/// The room of an output: `limit` bytes, of which `used` are taken by the jobs that decode to it
/// side by side. A frame whose header gives its content size decodes to no more, so a job takes
/// its run's content sizes at once, before it decodes them ([`Self::claim`]); a frame that gives
/// none takes each block's bytes as it is decoded, from what was left when the block began.
/// Taken once for a run, not for each block: the jobs' blocks of small frames would each write
/// it, and it would go from core to core (the GitHub frames up to 1.7 times as slow on 6
/// threads).
struct Room<'r> {
    limit: usize,
    used: &'r AtomicUsize,
}

impl Room<'_> {
    /// How many bytes are left, as far as the jobs have taken them.
    fn left(&self) -> usize {
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }

    /// Take `n` bytes: fails, taking none, if fewer are left. Never more than the limit.
    fn take(&self, n: usize) -> Result<(), FrameDecoderError> {
        let limit = self.limit;
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(n).filter(|&used| used <= limit)
            })
            .map(drop)
            .map_err(|_| FrameDecoderError::TargetTooSmall)
    }

    /// Take the content sizes of those of `frames` whose headers give them, at once.
    fn claim(&self, frames: &[Frame<'_>]) -> Result<(), FrameDecoderError> {
        let sizes = frames
            .iter()
            .filter_map(|frame| frame.header.frame_content_size())
            .fold(0u64, u64::saturating_add);
        if sizes == 0 {
            return Ok(());
        }
        self.take(usize::try_from(sizes).unwrap_or(usize::MAX))
    }
}

/// Every frame of `input` decoded, appended to `output`, within its spare capacity; see
/// [`FrameDecoder::decode_all_parallel`](super::FrameDecoder::decode_all_parallel).
pub fn decode_all(
    input: &[u8],
    dicts: &BTreeMap<u32, Dictionary>,
    max_window_size: u64,
    output: &mut Vec<u8>,
) -> Result<(), FrameDecoderError> {
    let (frames, scan_error) = scan(input, max_window_size);
    let threads = rayon::current_num_threads();
    // A frame's blocks are decoded side by side only where its frames leave threads idle.
    let split_blocks = frames.len() < threads;
    let used = AtomicUsize::new(0);
    let room = Room {
        limit: output.capacity() - output.len(),
        used: &used,
    };
    let runs = split_runs(&frames, threads, |frame| frame.body.len());
    if runs.len() <= 1 {
        let mut scratch = None;
        decode_frames(
            &frames,
            dicts,
            &mut scratch,
            output,
            &room,
            split_blocks,
            true,
        )?;
    } else {
        let decoded: Vec<Result<Vec<u8>, FrameDecoderError>> = runs
            .par_iter()
            .map(|run| {
                // Room for what the frames say they decode to, so that it is not grown and copied
                // as they decode.
                let mut out = Vec::with_capacity(run_room(run).min(room.left()));
                let mut scratch = None;
                decode_frames(
                    run,
                    dicts,
                    &mut scratch,
                    &mut out,
                    &room,
                    split_blocks,
                    false,
                )?;
                Ok(out)
            })
            .collect();
        // Within `output`'s capacity, which the runs took their bytes from.
        for run in decoded {
            output.extend_from_slice(&run?);
        }
    }
    match scan_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Each of `inputs`, every frame of it, decoded to the spare capacity of the output of the same
/// index, side by side; see
/// [`FrameDecoder::decode_each_parallel`](super::FrameDecoder::decode_each_parallel).
pub fn decode_each<I: AsRef<[u8]> + Sync>(
    inputs: &[I],
    dicts: &BTreeMap<u32, Dictionary>,
    max_window_size: u64,
    outputs: &mut [Vec<u8>],
) -> Vec<Result<(), FrameDecoderError>> {
    assert_eq!(
        inputs.len(),
        outputs.len(),
        "as many outputs as there are inputs"
    );
    let threads = rayon::current_num_threads();
    // A frame's blocks are decoded side by side only where the inputs leave threads idle.
    let split_blocks = inputs.len() < threads;
    let mut jobs: Vec<(&[u8], &mut Vec<u8>)> = inputs
        .iter()
        .map(AsRef::as_ref)
        .zip(outputs.iter_mut())
        .collect();
    let mut runs = Vec::new();
    let mut rest = &mut jobs[..];
    for len in run_lens(rest.iter().map(|(input, _)| input.len()), threads) {
        let (run, after) = rest.split_at_mut(len);
        runs.push(run);
        rest = after;
    }
    let alone = runs.len() <= 1;
    let decode_run = |run: &mut [(&[u8], &mut Vec<u8>)]| {
        // One scratch for the run's inputs, its buffers and tables made once.
        let mut scratch = None;
        run.iter_mut()
            .map(|(input, output)| {
                let len = output.len();
                let used = AtomicUsize::new(0);
                let room = Room {
                    limit: output.capacity() - len,
                    used: &used,
                };
                let (frames, scan_error) = scan(input, max_window_size);
                let result = decode_frames(
                    &frames,
                    dicts,
                    &mut scratch,
                    output,
                    &room,
                    split_blocks,
                    alone,
                )
                .and(scan_error.map_or(Ok(()), Err));
                if result.is_err() {
                    output.truncate(len);
                }
                result
            })
            .collect::<Vec<_>>()
    };
    if alone {
        return runs.into_iter().flat_map(decode_run).collect();
    }
    let results: Vec<Vec<_>> = runs.into_par_iter().map(decode_run).collect();
    results.into_iter().flatten().collect()
}

/// `frames` decoded one after another, appended to `output` within `room`, which their content
/// sizes are taken from first, with `scratch` (see [`decode_frame`]).
fn decode_frames(
    frames: &[Frame<'_>],
    dicts: &BTreeMap<u32, Dictionary>,
    scratch: &mut Option<DecoderScratch>,
    output: &mut Vec<u8>,
    room: &Room<'_>,
    split_blocks: bool,
    alone: bool,
) -> Result<(), FrameDecoderError> {
    room.claim(frames)?;
    for frame in frames {
        decode_frame(frame, dicts, scratch, output, room, split_blocks, alone)?;
    }
    Ok(())
}

/// A frame found in the input: its header and its blocks, not yet decoded.
struct Frame<'a> {
    header: FrameHeader,
    window_size: usize,
    /// The frame after its header: its blocks, then its checksum if it has one.
    body: &'a [u8],
    /// The checksum the frame ends with, if it has one: checked with the `hash` feature.
    #[cfg_attr(not(feature = "hash"), allow(dead_code))]
    checksum: Option<u32>,
}

impl Frame<'_> {
    /// Room for what the frame decodes to: what its header says, as far as its blocks can make
    /// it and up to [`ROOM_MOST`], a header's word not trusted further before it is decoded.
    fn room(&self) -> usize {
        let max_block_len = max_block_len(self.window_size);
        // A block takes 3 bytes at least.
        let most = (self.body.len() / 3)
            .saturating_mul(max_block_len)
            .min(ROOM_MOST);
        self.header
            .frame_content_size()
            .unwrap_or(0)
            .min(most as u64) as usize
    }
}

/// A block found in a frame.
struct Block<'a> {
    block_type: BlockType,
    /// What follows its header: the block's bytes, or its one byte if it is RLE.
    content: &'a [u8],
    /// How many bytes a raw or an RLE block decodes to.
    decompressed_size: usize,
}

/// The frames of `input`, skippable frames left out, as far as they can be found: followed by the
/// error that stopped the search, if one did. A frame's end is found by its blocks' headers, which
/// say how long each is; nothing is decoded.
fn scan(mut input: &[u8], max_window_size: u64) -> (Vec<Frame<'_>>, Option<FrameDecoderError>) {
    let mut frames = Vec::new();
    while !input.is_empty() {
        match scan_frame(&mut input, max_window_size) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => {}
            Err(err) => return (frames, Some(err)),
        }
    }
    (frames, None)
}

/// The frame at the start of `input`, which is moved past it; `None` for a skippable frame.
fn scan_frame<'a>(
    input: &mut &'a [u8],
    max_window_size: u64,
) -> Result<Option<Frame<'a>>, FrameDecoderError> {
    use FrameDecoderError as err;
    let mut src = *input;
    let header = match frame::read_frame_header(&mut src) {
        Ok((header, _)) => header,
        Err(ReadFrameHeaderError::SkipFrame { length, .. }) => {
            *input = src.get(length as usize..).ok_or(err::FailedToSkipFrame)?;
            return Ok(None);
        }
        Err(e) => return Err(err::ReadFrameHeaderError(e)),
    };
    let window_size = check_window_size(header.window_size()?, max_window_size)?;
    let body = src;
    walk_blocks(&mut src, |_| {})?;
    let checksum = if header.descriptor.content_checksum_flag() {
        let mut checksum = [0; 4];
        src.read_exact(&mut checksum)
            .map_err(err::FailedToReadChecksum)?;
        Some(u32::from_le_bytes(checksum))
    } else {
        None
    };
    let body = &body[..body.len() - src.len()];
    *input = src;
    Ok(Some(Frame {
        header,
        window_size,
        body,
        checksum,
    }))
}

/// The blocks at the start of `src`, to its frame's last, each handed to `found`; `src` is moved
/// past them.
fn walk_blocks<'a>(
    src: &mut &'a [u8],
    mut found: impl FnMut(Block<'a>),
) -> Result<(), FrameDecoderError> {
    use FrameDecoderError as err;
    let mut block_dec = block_decoder::new();
    loop {
        let (block_header, _) = block_dec
            .read_block_header(&mut *src)
            .map_err(err::FailedToReadBlockHeader)?;
        let len = block_header.content_size as usize;
        if src.len() < len {
            let eof = Error::from(ErrorKind::UnexpectedEof);
            return Err(err::FailedToReadBlockBody(match block_header.block_type {
                BlockType::Compressed => DecodeBlockContentError::DecompressBlockError(
                    DecompressBlockError::BlockContentReadError(eof),
                ),
                step => DecodeBlockContentError::ReadError { step, source: eof },
            }));
        }
        let (content, rest) = src.split_at(len);
        found(Block {
            block_type: block_header.block_type,
            content,
            decompressed_size: block_header.decompressed_size as usize,
        });
        *src = rest;
        if block_header.last_block {
            return Ok(());
        }
    }
}

/// `items` split into runs of consecutive ones, each decoded by one job (see [`run_lens`]).
fn split_runs<T>(items: &[T], threads: usize, size: impl Fn(&T) -> usize + Clone) -> Vec<&[T]> {
    let mut runs = Vec::new();
    let mut rest = items;
    for len in run_lens(items.iter().map(size), threads) {
        let (run, after) = rest.split_at(len);
        runs.push(run);
        rest = after;
    }
    runs
}

/// The lengths of the runs of consecutive items of these `sizes`, in the input, each decoded by
/// one job: about four to a thread, by size, so that the threads finish about together. All in
/// one run on one thread.
fn run_lens(sizes: impl ExactSizeIterator<Item = usize> + Clone, threads: usize) -> Vec<usize> {
    let count = sizes.len();
    if count <= 1 || threads <= 1 {
        return alloc::vec![count];
    }
    // Saturating: the inputs of `decode_each` may be any slices, the same one many times.
    let total = sizes.clone().fold(0usize, usize::saturating_add);
    let each = (total / (4 * threads)).max(RUN_BYTES_LEAST);
    let mut lens = Vec::new();
    let (mut len, mut size) = (0, 0usize);
    for item in sizes {
        len += 1;
        size = size.saturating_add(item);
        if size >= each {
            lens.push(len);
            (len, size) = (0, 0);
        }
    }
    if len > 0 {
        lens.push(len);
    }
    lens
}

/// Decode `frame`, appending it to `output` within `room`, with `scratch`, made if it is `None`
/// and kept for the next frame; its blocks side by side if `split_blocks` and it has enough of
/// them. `alone` if one job decodes the whole input.
fn decode_frame(
    frame: &Frame<'_>,
    dicts: &BTreeMap<u32, Dictionary>,
    scratch: &mut Option<DecoderScratch>,
    output: &mut Vec<u8>,
    room: &Room<'_>,
    split_blocks: bool,
    alone: bool,
) -> Result<(), FrameDecoderError> {
    let checksum_flag = frame.header.descriptor.content_checksum_flag();
    let scratch = match scratch {
        Some(scratch) => {
            scratch.reset(frame.window_size, checksum_flag);
            scratch
        }
        None => scratch.insert(DecoderScratch::new(frame.window_size, checksum_flag)),
    };
    let dict = match frame.header.dictionary_id() {
        Some(dict_id) => Some(
            dicts
                .get(&dict_id)
                .ok_or(FrameDecoderError::DictNotProvided { dict_id })?,
        ),
        None => None,
    };
    if let Some(dict) = dict {
        scratch.init_from_dict(dict);
    }
    // At most its content size, which is taken from the output's room already: no more than its
    // capacity has, a run's own output grown past it only as it decodes.
    output.reserve(frame.room());
    // Split only where there are two batches at least, for one to execute beside the other's
    // decoding.
    let threads = rayon::current_num_threads();
    let split =
        split_blocks && threads > 1 && frame.body.len() >= 2 * threads * BATCH_BYTES_PER_THREAD;
    if in_place(frame, split, alone) {
        // Executed into `output` itself, which is the window.
        scratch.buffer.start_in_place(output);
        let result = decode_frame_blocks(frame, dict, scratch, room, split, None);
        scratch.buffer.end_in_place(output);
        result
    } else {
        decode_frame_blocks(frame, dict, scratch, room, split, Some(output))
    }
}

/// Collect what `scratch` has decoded: to `output`, or in place where it is `None`.
fn collect(
    scratch: &mut DecoderScratch,
    output: Option<&mut Vec<u8>>,
) -> Result<(), FrameDecoderError> {
    match output {
        Some(output) => {
            scratch
                .buffer
                .collect_to_writer(output)
                .map_err(FrameDecoderError::FailedToDrainDecodebuffer)?;
        }
        None => scratch.buffer.collect_in_place(),
    }
    Ok(())
}

/// [`decode_frame`]'s blocks, into `scratch`'s buffer, then collected to `output`, or decoded in
/// place where `output` is `None`; and its checksum checked.
fn decode_frame_blocks(
    frame: &Frame<'_>,
    dict: Option<&Dictionary>,
    scratch: &mut DecoderScratch,
    room: &Room<'_>,
    split: bool,
    mut output: Option<&mut Vec<u8>>,
) -> Result<(), FrameDecoderError> {
    let max_block_len = max_block_len(frame.window_size);
    // At most its content size, taken from the output's room already: in place, never past the
    // output's capacity.
    let content_size = frame.header.frame_content_size().unwrap_or(0);
    scratch
        .buffer
        .reserve(content_size.min(max_block_len as u64) as usize);

    if split {
        decode_blocks_split(frame, dict, scratch, room, output)?;
    } else {
        let mut src = frame.body;
        let mut block_dec = block_decoder::new();
        loop {
            let (header, _) = block_dec
                .read_block_header(&mut src)
                .map_err(FrameDecoderError::FailedToReadBlockHeader)?;
            let held = Held::new(frame, scratch, room, max_block_len);
            block_dec
                .decode_block_content(&header, scratch, &mut src)
                .map_err(|e| held.failed(frame, e))?;
            held.done(frame, scratch, room)?;
            collect(scratch, output.as_deref_mut())?;
            if header.last_block {
                break;
            }
        }
    }
    check_content_size(&frame.header, scratch.buffer.decoded())?;

    #[cfg(feature = "hash")]
    if let (Some(expected), Some(calculated)) = (frame.checksum, scratch.buffer.checksum()) {
        // The frame's checksum is the hash's lower 32 bits.
        let calculated = calculated as u32;
        if calculated != expected {
            return Err(FrameDecoderError::ChecksumMismatch {
                expected,
                calculated,
            });
        }
    }
    Ok(())
}

/// A block of a frame about to be decoded, the frame's buffer held to what is left of its
/// content size, where its header gives one, which its output's room was taken for; else to
/// what is left of the room.
struct Held {
    /// What the frame had decoded to before the block.
    decoded: u64,
    /// Whether that holds the block to less than its Block_Maximum_Size.
    held: bool,
    /// Whether the room is less than the content size left: the output too small, not the frame
    /// corrupt, where the block decodes to more.
    by_room: bool,
}

impl Held {
    fn new(
        frame: &Frame<'_>,
        scratch: &mut DecoderScratch,
        room: &Room<'_>,
        max_block_len: usize,
    ) -> Held {
        let decoded = scratch.buffer.decoded();
        let (content_left, room_left) = match frame.header.frame_content_size() {
            Some(size) => (size.saturating_sub(decoded), u64::MAX),
            None => (u64::MAX, room.left() as u64),
        };
        let left = content_left.min(room_left);
        scratch.buffer.set_most(decoded.saturating_add(left));
        Held {
            decoded,
            held: left < max_block_len as u64,
            by_room: room_left < content_left,
        }
    }

    /// The frame's error for the block's `err`.
    fn failed(&self, frame: &Frame<'_>, err: DecodeBlockContentError) -> FrameDecoderError {
        block_failed(err, self.held, || {
            if self.by_room {
                FrameDecoderError::TargetTooSmall
            } else {
                FrameDecoderError::ContentSizeExceeded {
                    content_size: frame.header.frame_content_size().unwrap_or(u64::MAX),
                }
            }
        })
    }

    /// The block decoded: its bytes taken from the room, unless its frame's content size was.
    fn done(
        &self,
        frame: &Frame<'_>,
        scratch: &DecoderScratch,
        room: &Room<'_>,
    ) -> Result<(), FrameDecoderError> {
        if frame.header.frame_content_size().is_some() {
            return Ok(());
        }
        room.take((scratch.buffer.decoded() - self.decoded) as usize)
    }
}

/// A block on its way through the three steps.
enum Work<'a> {
    Raw(&'a [u8]),
    Rle(u8, usize),
    Compressed(Compressed<'a>),
    /// The block whose tables could not be read, which ends the frame.
    Failed(DecompressBlockError),
}

/// A compressed block: what its tables make of it, then what its literals and sequences decode
/// to.
struct Compressed<'a> {
    sections: BlockSections<'a>,
    /// How many of the literals section's bytes are its Huffman table's description.
    tree_bytes: u32,
    huf: Arc<HuffmanTable>,
    seq: Arc<SeqTables>,
    /// The sequences' bitstream, after their tables' descriptions.
    bit_stream: &'a [u8],
    /// The literals, then `LITERALS_PADDING` bytes, once decoded.
    literals: Vec<u8>,
    sequences: Vec<Sequence>,
    decoded: Result<(), DecompressBlockError>,
}

/// The tables as the blocks so far have left them, each shared with the blocks that decode from
/// it: changed in place where no block holds it any more, else copied first.
struct Tables {
    huf: Arc<HuffmanTable>,
    fse: FSEScratch,
    seq: Arc<SeqTables>,
}

impl Tables {
    fn new(dict: Option<&Dictionary>) -> Tables {
        let mut huf = HuffmanTable::new();
        let mut fse = FSEScratch::new();
        if let Some(dict) = dict {
            huf.reinit_from_dict(&dict.huf.table, dict.id);
            fse.reinit_from_dict(&dict.fse, dict.id);
        }
        Tables {
            huf: Arc::new(huf),
            seq: Arc::new(fse.seq.clone()),
            fse,
        }
    }

    /// The first step for a block: its tables. `spare` gives buffers for its literals and
    /// sequences.
    fn prepare<'a>(
        &mut self,
        block: &Block<'a>,
        max_block_len: usize,
        spare: &mut Spare,
    ) -> Work<'a> {
        match block.block_type {
            BlockType::Raw => Work::Raw(block.content),
            BlockType::RLE => Work::Rle(block.content[0], block.decompressed_size),
            BlockType::Compressed | BlockType::Reserved => {
                match self.prepare_compressed(block.content, max_block_len, spare) {
                    Ok(work) => Work::Compressed(work),
                    Err(err) => Work::Failed(err),
                }
            }
        }
    }

    fn prepare_compressed<'a>(
        &mut self,
        content: &'a [u8],
        max_block_len: usize,
        spare: &mut Spare,
    ) -> Result<Compressed<'a>, DecompressBlockError> {
        let sections = split_sections(content, max_block_len)?;
        let lits = &sections.literals;
        let tree_bytes = if matches!(lits.ls_type, LiteralsSectionType::Compressed)
            || literals_need_prepare(lits, &self.huf)
        {
            prepare_literals(lits, Arc::make_mut(&mut self.huf), sections.literals_source)?
        } else {
            0
        };
        let seqs = &sections.sequences;
        // Each sequence's match is 3 bytes at least: more of them than that makes no block, and
        // would be stored before their execution found it.
        if seqs.num_sequences as usize > max_block_len / 3 {
            return Err(DecompressBlockError::DecompressedSizeTooLarge {
                max: max_block_len,
                at_least: 3 * seqs.num_sequences as usize,
            });
        }
        let bit_stream = if seqs.num_sequences == 0 {
            check_no_sequences(sections.sequences_source)?;
            sections.sequences_source
        } else {
            let source = sections.sequences_source;
            let bytes = update_fse_tables(seqs, source, &mut self.fse)?;
            let modes = seqs.modes.expect("checked by the tables' update");
            if [modes.ll_mode(), modes.of_mode(), modes.ml_mode()]
                .iter()
                .any(|mode| !matches!(mode, ModeType::Repeat))
            {
                self.seq = Arc::new(self.fse.seq.clone());
            }
            &source[bytes..]
        };
        let (literals, sequences) = spare.pop().unwrap_or_default();
        Ok(Compressed {
            tree_bytes,
            huf: self.huf.clone(),
            seq: self.seq.clone(),
            bit_stream,
            literals,
            sequences,
            sections,
            decoded: Ok(()),
        })
    }
}

impl Compressed<'_> {
    /// How many bytes its literals and sequences take, once decoded.
    fn store_len(&self) -> usize {
        self.sections.literals.regenerated_size as usize
            + self.sections.sequences.num_sequences as usize * size_of::<Sequence>()
    }

    /// The second step: the block's literals and sequences, decoded into its buffers.
    fn decode(&mut self) {
        self.decoded = self.try_decode();
    }

    fn try_decode(&mut self) -> Result<(), DecompressBlockError> {
        let BlockSections {
            literals: section,
            literals_source,
            sequences: seqs,
            ..
        } = &self.sections;
        self.literals.clear();
        let used = decode_literals_with(
            section,
            &self.huf,
            self.tree_bytes,
            literals_source,
            &mut self.literals,
        )?;
        check_literals(section, &self.literals, literals_source, used);
        let len = self.literals.len();
        self.literals.resize(len + LITERALS_PADDING, 0);
        if seqs.num_sequences != 0 {
            let mut store = SequenceStore {
                sequences: core::mem::take(&mut self.sequences),
            };
            store.sequences.clear();
            store.sequences.reserve(seqs.num_sequences as usize);
            let result = decode_sequences_with::<DecompressBlockError, _>(
                seqs,
                self.bit_stream,
                &self.seq,
                &mut store,
            );
            self.sequences = store.sequences;
            result?;
        }
        Ok(())
    }

    /// The third step: the block's sequences, executed into `scratch`'s buffer, of at most
    /// `max_block_len` bytes.
    fn execute(
        &mut self,
        scratch: &mut DecoderScratch,
        max_block_len: usize,
    ) -> Result<(), DecompressBlockError> {
        core::mem::replace(&mut self.decoded, Ok(()))?;
        let literals_len = self.literals.len() - LITERALS_PADDING;
        if self.sections.sequences.num_sequences == 0 {
            // Its tables checked them against Block_Maximum_Size; `max_block_len` may be less.
            BlockDecoder::check_decompressed_size(literals_len, max_block_len)?;
            scratch.buffer.push(&self.literals[..literals_len]);
            return Ok(());
        }
        let mut executor = SequenceExecutor::new(
            &mut scratch.buffer,
            &self.literals,
            &mut scratch.offset_hist,
            max_block_len,
        );
        execute_sequences(&mut executor, &self.sequences)?;
        executor.finish()?;
        Ok(())
    }
}

/// A frame's blocks in three steps (see the module's documentation), into `scratch`, which is set
/// up for the frame, and collected to `output` (see [`collect`]).
fn decode_blocks_split(
    frame: &Frame<'_>,
    dict: Option<&Dictionary>,
    scratch: &mut DecoderScratch,
    room: &Room<'_>,
    mut output: Option<&mut Vec<u8>>,
) -> Result<(), FrameDecoderError> {
    let max_block_len = max_block_len(frame.window_size);
    let threads = rayon::current_num_threads();
    let batch = Batch {
        // Eight batches to the frame at least, for the pipeline to be full most of the frame.
        bytes: (threads * BATCH_BYTES_PER_THREAD).min(frame.body.len() / 8),
        store: threads * BATCH_STORE_PER_THREAD,
        max_block_len,
    };
    let mut tables = Tables::new(dict);
    let mut spare = Vec::new();

    let mut all_blocks = Vec::new();
    walk_blocks(&mut { frame.body }, |block| all_blocks.push(block))?;
    let mut blocks = &all_blocks[..];
    let mut works = first_two(&mut tables, &mut spare, &mut blocks, &batch);
    loop {
        let failed = matches!(works.last(), Some(Work::Failed(_)));
        let more = !failed && !blocks.is_empty();
        let (executed, next_works) = rayon::join(
            || execute_works(works, frame, scratch, room, output.as_deref_mut()),
            || more.then(|| first_two(&mut tables, &mut spare, &mut blocks, &batch)),
        );
        spare.extend(executed?);
        match next_works {
            Some(next) => works = next,
            None => return Ok(()),
        }
    }
}

/// How large a batch of blocks is.
struct Batch {
    /// Bytes of the blocks in the input.
    bytes: usize,
    /// Bytes of their literals and sequences, decoded.
    store: usize,
    max_block_len: usize,
}

/// The first two steps for a batch of the first `blocks`, which are moved past it: their tables,
/// in order, as far as they can be read, then their literals and sequences, side by side.
fn first_two<'a>(
    tables: &mut Tables,
    spare: &mut Spare,
    blocks: &mut &[Block<'a>],
    batch: &Batch,
) -> Vec<Work<'a>> {
    let mut works = Vec::new();
    let (mut bytes, mut store) = (0, 0);
    while let Some((block, rest)) = blocks.split_first() {
        *blocks = rest;
        let work = tables.prepare(block, batch.max_block_len, spare);
        bytes += block.content.len();
        let done = match &work {
            Work::Failed(_) => true,
            Work::Compressed(c) => {
                store += c.store_len();
                store >= batch.store || bytes >= batch.bytes
            }
            Work::Raw(_) | Work::Rle(..) => bytes >= batch.bytes,
        };
        works.push(work);
        if done {
            break;
        }
    }
    // In about two jobs to a thread, for one slower than the others not to hold the batch up.
    let each = works.len().div_ceil(2 * rayon::current_num_threads());
    works.par_iter_mut().with_min_len(each).for_each(|work| {
        if let Work::Compressed(c) = work {
            c.decode();
        }
    });
    works
}

/// The third step for a batch of `frame`'s blocks, in order, within `room`; returns the buffers
/// they are done with.
fn execute_works(
    works: Vec<Work<'_>>,
    frame: &Frame<'_>,
    scratch: &mut DecoderScratch,
    room: &Room<'_>,
    mut output: Option<&mut Vec<u8>>,
) -> Result<Spare, FrameDecoderError> {
    let max_block_len = max_block_len(frame.window_size);
    let mut done = Vec::with_capacity(works.len());
    for work in works {
        let held = Held::new(frame, scratch, room, max_block_len);
        let most = scratch.buffer.block_most(max_block_len);
        let block_err = |e: DecompressBlockError| {
            held.failed(frame, DecodeBlockContentError::DecompressBlockError(e))
        };
        match work {
            Work::Raw(data) => {
                BlockDecoder::check_decompressed_size(data.len(), most).map_err(block_err)?;
                scratch.buffer.push(data);
            }
            Work::Rle(byte, len) => {
                BlockDecoder::check_decompressed_size(len, most).map_err(block_err)?;
                scratch.buffer.extend_and_fill(byte, len);
            }
            Work::Compressed(mut c) => {
                c.execute(scratch, most).map_err(block_err)?;
                done.push((c.literals, c.sequences));
            }
            Work::Failed(e) => return Err(block_err(e)),
        }
        held.done(frame, scratch, room)?;
        collect(scratch, output.as_deref_mut())?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::run_lens;

    /// Sizes adding up past `usize`, as many inputs of the same large slice do on wasm32.
    #[test]
    fn run_lens_past_usize() {
        let sizes = [usize::MAX / 2, usize::MAX / 2, usize::MAX / 2, 1];
        let lens = run_lens(sizes.iter().copied(), 4);
        assert_eq!(lens.iter().sum::<usize>(), sizes.len());
    }
}
