//! Structures that wrap around various decoders to make decoding easier.

use super::decode_buffer::DecodeBuffer;
use super::sequence_table::SeqTable;
use crate::decoding::dictionary::Dictionary;
use crate::fse::FSETable;
use crate::huff0::HuffmanTable;
use alloc::vec::Vec;

use crate::blocks::sequence_section::{
    MAX_LITERAL_LENGTH_CODE, MAX_MATCH_LENGTH_CODE, MAX_OFFSET_CODE,
};

/// A block level decoding buffer.
pub struct DecoderScratch {
    /// The decoder used for Huffman blocks.
    pub huf: HuffmanScratch,
    /// The decoder used for FSE blocks.
    pub fse: FSEScratch,

    pub buffer: DecodeBuffer,
    pub offset_hist: [u32; 3],

    pub literals_buffer: Vec<u8>,
    pub block_content_buffer: Vec<u8>,

    /// The id of the dictionary whose content `buffer.dict_content` holds, which a frame using
    /// it again need not copy in; `None` once a dictionary may have been replaced. The tables
    /// keep their own (see [`SeqTable::reinit_from_dict`] and
    /// [`HuffmanTable::reinit_from_dict`]).
    dict_loaded: Option<u32>,
}

impl DecoderScratch {
    pub fn new(window_size: usize, checksum: bool) -> DecoderScratch {
        DecoderScratch {
            huf: HuffmanScratch {
                table: HuffmanTable::new(),
            },
            fse: FSEScratch::new(),
            buffer: DecodeBuffer::new(window_size, checksum),
            offset_hist: [1, 4, 8],

            block_content_buffer: Vec::new(),
            literals_buffer: Vec::new(),
            dict_loaded: None,
        }
    }

    pub fn reset(&mut self, window_size: usize, checksum: bool) {
        self.offset_hist = [1, 4, 8];
        self.literals_buffer.clear();
        self.block_content_buffer.clear();

        self.buffer.reset(window_size, checksum);

        self.fse.reset();

        self.huf.table.reset();
    }

    /// Sets the frame up to decode with `dict`: its tables, offsets and content. The content is
    /// copied in only if the buffer does not hold it from an earlier frame already: a dictionary's
    /// content is often a hundred times the size of the small frames it is made for.
    pub fn init_from_dict(&mut self, dict: &Dictionary) {
        self.fse.reinit_from_dict(&dict.fse, dict.id);
        self.huf.table.reinit_from_dict(&dict.huf.table, dict.id);
        self.offset_hist = dict.offset_hist;
        if self.dict_loaded != Some(dict.id) {
            self.buffer.dict_content.clear();
            self.buffer
                .dict_content
                .extend_from_slice(&dict.dict_content);
            self.dict_loaded = Some(dict.id);
        }
        self.buffer.dict_active = true;
    }

    /// Forgets which dictionary's content the buffer holds, so that the next frame using one
    /// copies it in again: for when a dictionary of the same id may have replaced it.
    pub fn forget_dict(&mut self) {
        self.dict_loaded = None;
        self.fse.forget_dict();
        self.huf.table.forget_dict();
    }
}

pub struct HuffmanScratch {
    pub table: HuffmanTable,
}

impl HuffmanScratch {
    pub fn new() -> HuffmanScratch {
        HuffmanScratch {
            table: HuffmanTable::new(),
        }
    }
}

impl Default for HuffmanScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// The sequences section's tables, and the FSE tables their descriptions are read into.
pub struct FSEScratch {
    pub offsets: FSETable,
    pub literal_lengths: FSETable,
    pub match_lengths: FSETable,
    pub of: SeqTable,
    pub ll: SeqTable,
    pub ml: SeqTable,
}

impl FSEScratch {
    pub fn new() -> FSEScratch {
        FSEScratch {
            offsets: FSETable::new(MAX_OFFSET_CODE),
            literal_lengths: FSETable::new(MAX_LITERAL_LENGTH_CODE),
            match_lengths: FSETable::new(MAX_MATCH_LENGTH_CODE),
            of: SeqTable::new(),
            ll: SeqTable::new(),
            ml: SeqTable::new(),
        }
    }

    pub fn reset(&mut self) {
        self.offsets.reset();
        self.literal_lengths.reset();
        self.match_lengths.reset();
        self.of.reset();
        self.ll.reset();
        self.ml.reset();
    }

    /// Take `other`'s tables, dictionary `dict_id`'s, which are all a later block can repeat:
    /// copied only where these no longer hold them from an earlier frame.
    pub fn reinit_from_dict(&mut self, other: &Self, dict_id: u32) {
        self.of.reinit_from_dict(&other.of, dict_id);
        self.ll.reinit_from_dict(&other.ll, dict_id);
        self.ml.reinit_from_dict(&other.ml, dict_id);
    }

    /// Forget which dictionary's tables these hold (see [`SeqTable::forget_dict`]).
    pub fn forget_dict(&mut self) {
        self.of.forget_dict();
        self.ll.forget_dict();
        self.ml.forget_dict();
    }
}

impl Default for FSEScratch {
    fn default() -> Self {
        Self::new()
    }
}
