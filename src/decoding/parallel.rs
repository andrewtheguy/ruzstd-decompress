//! Decoding on several threads, those of the rayon pool it is called in: what
//! [`FrameDecoder::decode_all_parallel`] does.
//!
//! The frames of the input are independent, and are decoded side by side, a run of them to each
//! job. Within a frame, a block's matches copy from the blocks before it, so its sequences must be
//! executed in order; but what makes them, the literals' Huffman streams and the sequences' FSE
//! streams, needs only the block's tables. So a frame of many blocks, when there are fewer frames
//! than threads, is decoded in three steps:
//!
//! 1. the tables, block after block, on one thread, as a block may repeat the one before's;
//! 2. each block's literals and sequences, decoded into buffers of its own, a batch of blocks side
//!    by side;
//! 3. the sequences executed into the frame's window, block after block, on one thread.
//!
//! A batch's execution runs beside the next batch's first two steps.
//!
//! [`FrameDecoder::decode_all_parallel`]: super::FrameDecoder::decode_all_parallel

use super::block_decoder::{
    self, BlockDecoder, BlockSections, check_literals, check_no_sequences, split_sections,
};
use super::decode_buffer::LITERALS_PADDING;
use super::dictionary::Dictionary;
use super::errors::{
    DecodeBlockContentError, DecompressBlockError, FrameDecoderError, ReadFrameHeaderError,
};
use super::frame::{self, FrameHeader};
use super::frame_decoder::check_window_size;
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
use crate::common::MAX_BLOCK_SIZE;
use crate::huff0::HuffmanTable;
use crate::io::{Error, ErrorKind, Read};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
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

/// A run of frames, decoded by one job, is at least this many bytes of them.
const RUN_BYTES_LEAST: usize = 32 << 10;

/// The most room reserved for output before it is decoded, whatever the frames' headers say:
/// past it, the output grows as it is decoded.
const ROOM_MOST: usize = 256 << 20;

/// Room for what `frames` decode to, up to [`ROOM_MOST`].
fn room(frames: &[Frame<'_>]) -> usize {
    frames
        .iter()
        .map(Frame::room)
        .fold(0, usize::saturating_add)
        .min(ROOM_MOST)
}

/// Every frame of `input` decoded, appended to `output`; see
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
    let runs = runs(&frames, threads);
    if runs.len() <= 1 {
        output.reserve(room(&frames));
        let mut scratch = None;
        for frame in &frames {
            decode_frame(frame, dicts, &mut scratch, output, split_blocks)?;
        }
    } else {
        let decoded: Vec<Result<Vec<u8>, FrameDecoderError>> = runs
            .par_iter()
            .map(|run| {
                // Room for what the frames say they decode to, so that it is not grown and copied
                // as they decode.
                let mut out = Vec::with_capacity(room(run));
                let mut scratch = None;
                for frame in *run {
                    decode_frame(frame, dicts, &mut scratch, &mut out, split_blocks)?;
                }
                Ok(out)
            })
            .collect();
        for run in decoded {
            output.extend_from_slice(&run?);
        }
    }
    match scan_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
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
    /// it and up to [`ROOM_MOST`], a header's word not trusted further.
    fn room(&self) -> usize {
        let max_block_len = self.window_size.min(MAX_BLOCK_SIZE as usize);
        // A block takes 3 bytes at least.
        let most = (self.body.len() / 3)
            .saturating_mul(max_block_len)
            .min(ROOM_MOST);
        self.header.frame_content_size().min(most as u64) as usize
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

/// The frames split into runs of consecutive ones, each decoded by one job: about four to a
/// thread, by their size in the input, so that the threads finish about together.
fn runs<'f, 'a>(frames: &'f [Frame<'a>], threads: usize) -> Vec<&'f [Frame<'a>]> {
    if frames.len() <= 1 || threads <= 1 {
        return alloc::vec![frames];
    }
    let total: usize = frames.iter().map(|f| f.body.len()).sum();
    let each = (total / (4 * threads)).max(RUN_BYTES_LEAST);
    let mut runs = Vec::new();
    let mut start = 0;
    let mut size = 0;
    for (i, frame) in frames.iter().enumerate() {
        size += frame.body.len();
        if size >= each {
            runs.push(&frames[start..=i]);
            start = i + 1;
            size = 0;
        }
    }
    if start < frames.len() {
        runs.push(&frames[start..]);
    }
    runs
}

/// Decode `frame`, appending it to `output`, with `scratch`, made if it is `None` and kept for
/// the next frame; its blocks side by side if `split_blocks` and it has enough of them.
fn decode_frame(
    frame: &Frame<'_>,
    dicts: &BTreeMap<u32, Dictionary>,
    scratch: &mut Option<DecoderScratch>,
    output: &mut Vec<u8>,
    split_blocks: bool,
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
    let max_block_len = frame.window_size.min(MAX_BLOCK_SIZE as usize);
    let content_size = frame.header.frame_content_size();
    output.reserve(frame.room());
    scratch
        .buffer
        .reserve(content_size.min(max_block_len as u64) as usize);

    // Split only where there are two batches at least, for one to execute beside the other's
    // decoding.
    let threads = rayon::current_num_threads();
    if split_blocks && threads > 1 && frame.body.len() >= 2 * threads * BATCH_BYTES_PER_THREAD {
        decode_blocks_split(frame, dict, scratch, output)?;
    } else {
        let mut src = frame.body;
        let mut block_dec = block_decoder::new();
        loop {
            let (header, _) = block_dec
                .read_block_header(&mut src)
                .map_err(FrameDecoderError::FailedToReadBlockHeader)?;
            block_dec
                .decode_block_content(&header, scratch, &mut src)
                .map_err(FrameDecoderError::FailedToReadBlockBody)?;
            collect(scratch, output)?;
            if header.last_block {
                break;
            }
        }
    }

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

/// Move what `scratch` has decoded to `output`.
fn collect(scratch: &mut DecoderScratch, output: &mut Vec<u8>) -> Result<(), FrameDecoderError> {
    scratch
        .buffer
        .collect_to_writer(output)
        .map_err(FrameDecoderError::FailedToDrainDecodebuffer)?;
    Ok(())
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

    /// The third step: the block's sequences, executed into `scratch`'s buffer.
    fn execute(
        &mut self,
        scratch: &mut DecoderScratch,
        max_block_len: usize,
    ) -> Result<(), DecompressBlockError> {
        core::mem::replace(&mut self.decoded, Ok(()))?;
        let literals_len = self.literals.len() - LITERALS_PADDING;
        if self.sections.sequences.num_sequences == 0 {
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
/// up for the frame, and on to `output`.
fn decode_blocks_split(
    frame: &Frame<'_>,
    dict: Option<&Dictionary>,
    scratch: &mut DecoderScratch,
    output: &mut Vec<u8>,
) -> Result<(), FrameDecoderError> {
    // Block_Maximum_Size (RFC 8878 3.1.1.2.4): the smaller of the window and 128 KiB
    let max_block_len = frame.window_size.min(MAX_BLOCK_SIZE as usize);
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
            || execute_works(works, scratch, max_block_len, output),
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

/// The third step for a batch of blocks, in order; returns the buffers they are done with.
fn execute_works(
    works: Vec<Work<'_>>,
    scratch: &mut DecoderScratch,
    max_block_len: usize,
    output: &mut Vec<u8>,
) -> Result<Spare, FrameDecoderError> {
    use FrameDecoderError as err;
    let block_err = |e: DecompressBlockError| {
        err::FailedToReadBlockBody(DecodeBlockContentError::DecompressBlockError(e))
    };
    let mut done = Vec::with_capacity(works.len());
    for work in works {
        match work {
            Work::Raw(data) => {
                BlockDecoder::check_decompressed_size(data.len(), max_block_len)
                    .map_err(block_err)?;
                scratch.buffer.push(data);
            }
            Work::Rle(byte, len) => {
                BlockDecoder::check_decompressed_size(len, max_block_len).map_err(block_err)?;
                scratch.buffer.extend_and_fill(byte, len);
            }
            Work::Compressed(mut c) => {
                c.execute(scratch, max_block_len).map_err(block_err)?;
                done.push((c.literals, c.sequences));
            }
            Work::Failed(e) => return Err(block_err(e)),
        }
        collect(scratch, output)?;
    }
    Ok(done)
}
