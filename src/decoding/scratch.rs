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
}

impl DecoderScratch {
    pub fn new(window_size: usize) -> DecoderScratch {
        DecoderScratch {
            huf: HuffmanScratch {
                table: HuffmanTable::new(),
            },
            fse: FSEScratch::new(),
            buffer: DecodeBuffer::new(window_size),
            offset_hist: [1, 4, 8],

            block_content_buffer: Vec::new(),
            literals_buffer: Vec::new(),
        }
    }

    pub fn reset(&mut self, window_size: usize) {
        self.offset_hist = [1, 4, 8];
        self.literals_buffer.clear();
        self.block_content_buffer.clear();

        self.buffer.reset(window_size);

        self.fse.reset();

        self.huf.table.reset();
    }

    pub fn init_from_dict(&mut self, dict: &Dictionary) {
        self.fse.reinit_from(&dict.fse);
        self.huf.table.reinit_from(&dict.huf.table);
        self.offset_hist = dict.offset_hist;
        self.buffer.dict_content.clear();
        self.buffer
            .dict_content
            .extend_from_slice(&dict.dict_content);
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

    /// Take `other`'s tables, which are all a later block can repeat.
    pub fn reinit_from(&mut self, other: &Self) {
        self.of.reinit_from(&other.of);
        self.ll.reinit_from(&other.ll);
        self.ml.reinit_from(&other.ml);
    }
}

impl Default for FSEScratch {
    fn default() -> Self {
        Self::new()
    }
}
