//! Utilities for decoding Huff0 encoded huffman data.

use crate::bit_io::BitReaderReversed;
use crate::decoding::errors::HuffmanTableError;
use crate::fse::{FSEDecoder, FSETable};
use alloc::vec::Vec;

/// The Zstandard specification limits the maximum length of a code to 11 bits.
pub(crate) const MAX_MAX_NUM_BITS: u8 = 11;

/// The table always has `1 << MAX_MAX_NUM_BITS` entries, whatever the codes' longest: one of
/// `max_num_bits` bits is repeated for every value of the bits after it. So a lookup is the next
/// `MAX_MAX_NUM_BITS` bits, a fixed shift, and needs no bounds check.
const TABLE_SIZE: usize = 1 << MAX_MAX_NUM_BITS;

/// A Huffman decoding table contains a list of Huffman prefix codes and their associated values
pub struct HuffmanTable {
    /// Indexed by the next `MAX_MAX_NUM_BITS` bits (see [`TABLE_SIZE`]): empty until the first
    /// table is built, so that a new decoder allocates nothing it may not use, then always
    /// `TABLE_SIZE` long, each build writing all of it.
    decode: Vec<Entry>,
    /// The same codes two symbols to an entry where both fit in `MAX_MAX_NUM_BITS` bits (see
    /// [`PairEntry`]), made from `decode` only when a block chooses it (see [`choose_pairs`]):
    /// empty until then, and `pairs_built` says whether it is `decode`'s.
    pairs: Vec<PairEntry>,
    pairs_built: bool,
    /// The weight of a symbol is the number of occurences in a table.
    /// This value is used in constructing a binary tree referred to as
    /// a Huffman tree. Once this tree is constructed, it can be used to build the
    /// lookup table
    weights: Vec<u8>,
    /// The maximum size in bits a prefix code in the encoded data can be.
    /// This value is used so that the decoder knows how many bits
    /// to read from the bitstream before checking the table. This
    /// value must be 11 or lower.
    pub max_num_bits: u8,
    bits: Vec<u8>,
    bit_ranks: Vec<u32>,
    rank_indexes: Vec<usize>,
    /// In some cases, the list of weights is compressed using FSE compression.
    fse_table: FSETable,
}

impl HuffmanTable {
    /// Create a new, empty table.
    pub fn new() -> HuffmanTable {
        HuffmanTable {
            decode: Vec::new(),
            pairs: Vec::new(),
            pairs_built: false,

            weights: Vec::with_capacity(256),
            max_num_bits: 0,
            bits: Vec::with_capacity(256),
            bit_ranks: Vec::with_capacity(11),
            rank_indexes: Vec::with_capacity(11),
            fse_table: FSETable::new(255),
        }
    }

    /// Completely empty the table then repopulate as a replica
    /// of `other`.
    pub fn reinit_from(&mut self, other: &Self) {
        self.reset();
        self.decode.clear();
        self.decode.extend_from_slice(&other.decode);
        self.pairs_built = false;
        self.weights.extend_from_slice(&other.weights);
        self.max_num_bits = other.max_num_bits;
        self.bits.extend_from_slice(&other.bits);
        self.rank_indexes.extend_from_slice(&other.rank_indexes);
        self.fse_table.reinit_from(&other.fse_table);
    }

    /// Completely empty the table of all data.
    pub fn reset(&mut self) {
        self.weights.clear();
        self.max_num_bits = 0;
        self.pairs_built = false;
        self.bits.clear();
        self.bit_ranks.clear();
        self.rank_indexes.clear();
        self.fse_table.reset();
    }

    /// The table, as an array, so that its lookups need no bounds check. Only once one is built,
    /// which the literals section checks (`max_num_bits` is 0 until then).
    #[inline(always)]
    fn table(&self) -> &[Entry; TABLE_SIZE] {
        self.decode
            .as_slice()
            .try_into()
            .expect("a table is built before it decodes")
    }

    /// Decode the symbol at the front of `br` and consume its bits.
    ///
    /// The next `MAX_MAX_NUM_BITS` bits of the stream index the table directly. `br` must have
    /// them available (see [`BitReaderReversed::ensure_bits`]); past the stream's end they are
    /// zeros, which only the bits after the code's are.
    #[inline(always)]
    fn decode_symbol(table: &[Entry; TABLE_SIZE], br: &mut BitReaderReversed<'_>) -> u8 {
        let entry = table[br.peek_bits(MAX_MAX_NUM_BITS) as usize & (TABLE_SIZE - 1)];
        br.consume(entry.num_bits);
        entry.symbol
    }

    /// Decode exactly `out.len()` symbols from `br` into `out`.
    pub(crate) fn decode_stream(&self, br: &mut BitReaderReversed<'_>, out: &mut [u8]) {
        let max_bits = self.max_num_bits;
        match max_bits {
            ..=7 => self.decode_stream_n::<8>(br, out),
            8 => self.decode_stream_n::<7>(br, out),
            9 => self.decode_stream_n::<6>(br, out),
            _ => self.decode_stream_n::<5>(br, out),
        }
    }

    /// Decode the symbol at the top of `window`, the reader's bits left aligned (see
    /// [`BitReaderReversed::sentinel_window`]), and shift its bits out.
    ///
    /// The index is a fixed shift of `window`, and the next window one shift by the entry's
    /// length. The window is all a stream needs in the loop: the bits used are counted by the
    /// sentinel, so the four streams' state fits in registers, wasm's included.
    #[inline(always)]
    fn decode_aligned(table: &[Entry; TABLE_SIZE], window: &mut u64) -> u8 {
        let entry = table[(*window >> (64 - MAX_MAX_NUM_BITS)) as usize];
        *window <<= entry.num_bits;
        entry.symbol
    }

    /// [`Self::decode_stream`], `N` symbols a refill: `N * max_num_bits` must be at most 56.
    #[inline(always)]
    fn decode_stream_n<const N: usize>(&self, br: &mut BitReaderReversed<'_>, out: &mut [u8]) {
        let max_bits = self.max_num_bits;
        debug_assert!(N * max_bits as usize <= 56);
        let table = self.table();
        // A copy, so its state can stay in registers, while a refill cannot reach the source's
        // end; nothing takes its address.
        let mut b = br.clone();
        let mut done = 0;
        for chunk in out.as_chunks_mut::<N>().0 {
            if !b.can_refill_fast() {
                break;
            }
            b.ensure_bits_fast(57);
            let mut window = b.sentinel_window();
            for byte in chunk {
                *byte = Self::decode_aligned(table, &mut window);
            }
            b.consume_to_sentinel(window);
            done += N;
        }
        *br = b;
        // The rest, near the source's end or past the last whole chunk, on `br` itself.
        for byte in &mut out[done..] {
            br.ensure_bits(MAX_MAX_NUM_BITS);
            *byte = Self::decode_symbol(table, br);
        }
    }

    /// Decode exactly `outs[i].len()` symbols from `brs[i]` into `outs[i]`, for all four streams.
    ///
    /// The four streams are independent, so decoding them in one loop lets the CPU overlap them.
    /// After a refill at least 56 bits are available, enough for `56 / max_num_bits` symbols
    /// per stream before the next refill check.
    pub(crate) fn decode_4_streams(
        &self,
        brs: &mut [BitReaderReversed<'_>; 4],
        outs: [&mut [u8]; 4],
    ) {
        debug_assert!(self.max_num_bits > 0);
        match self.max_num_bits {
            ..=7 => self.decode_4_streams_n::<8>(brs, outs),
            8 => self.decode_4_streams_n::<7>(brs, outs),
            9 => self.decode_4_streams_n::<6>(brs, outs),
            _ => self.decode_4_streams_n::<5>(brs, outs),
        }
    }

    /// [`Self::decode_4_streams`], `N` symbols a stream a refill: `N * max_num_bits` must be at
    /// most 56. The readers are copied into locals for the loop, so that their state can stay in
    /// registers, and the output is taken `N` bytes at a time, so its stores need no checks.
    #[inline(always)]
    fn decode_4_streams_n<const N: usize>(
        &self,
        brs: &mut [BitReaderReversed<'_>; 4],
        outs: [&mut [u8]; 4],
    ) {
        let max_bits = self.max_num_bits;
        debug_assert!(N * max_bits as usize <= 56);
        let table = self.table();
        let [out0, out1, out2, out3] = outs;
        let common = out0.len().min(out1.len()).min(out2.len()).min(out3.len());
        let whole = common - common % N;
        let [mut b0, mut b1, mut b2, mut b3] = brs.clone();

        let chunks = out0[..whole]
            .as_chunks_mut::<N>()
            .0
            .iter_mut()
            .zip(out1[..whole].as_chunks_mut::<N>().0)
            .zip(out2[..whole].as_chunks_mut::<N>().0)
            .zip(out3[..whole].as_chunks_mut::<N>().0);
        // While no stream's refill can reach its source's end: so the loop has no call to the slow
        // refill, which would keep the readers in memory (see `ensure_bits_fast`).
        let mut done = 0;
        for (((c0, c1), c2), c3) in chunks {
            if !(b0.can_refill_fast()
                && b1.can_refill_fast()
                && b2.can_refill_fast()
                && b3.can_refill_fast())
            {
                break;
            }
            b0.ensure_bits_fast(57);
            b1.ensure_bits_fast(57);
            b2.ensure_bits_fast(57);
            b3.ensure_bits_fast(57);
            let (mut w0, mut w1, mut w2, mut w3) = (
                b0.sentinel_window(),
                b1.sentinel_window(),
                b2.sentinel_window(),
                b3.sentinel_window(),
            );
            for k in 0..N {
                c0[k] = Self::decode_aligned(table, &mut w0);
                c1[k] = Self::decode_aligned(table, &mut w1);
                c2[k] = Self::decode_aligned(table, &mut w2);
                c3[k] = Self::decode_aligned(table, &mut w3);
            }
            b0.consume_to_sentinel(w0);
            b1.consume_to_sentinel(w1);
            b2.consume_to_sentinel(w2);
            b3.consume_to_sentinel(w3);
            done += N;
        }

        *brs = [b0, b1, b2, b3];
        let [br0, br1, br2, br3] = brs;
        self.decode_stream_n::<N>(br0, &mut out0[done..]);
        self.decode_stream_n::<N>(br1, &mut out1[done..]);
        self.decode_stream_n::<N>(br2, &mut out2[done..]);
        self.decode_stream_n::<N>(br3, &mut out3[done..]);
    }

    /// [`Self::decode_4_streams`], from the table of pairs when [`choose_pairs`] says it pays for
    /// its build: `compressed_size` is the literals' size in the block, the table's description
    /// included.
    pub(crate) fn decode_4_streams_sized(
        &mut self,
        brs: &mut [BitReaderReversed<'_>; 4],
        outs: [&mut [u8]; 4],
        compressed_size: usize,
    ) {
        let regenerated: usize = outs.iter().map(|out| out.len()).sum();
        if choose_pairs(regenerated, compressed_size) {
            self.build_pairs();
            self.decode_4_streams_pairs(brs, outs);
        } else {
            self.decode_4_streams(brs, outs);
        }
    }

    /// Make the table of pairs from `decode`, unless it already is.
    ///
    /// An index's first symbol is `decode`'s entry for it; the bits after that symbol's code,
    /// shifted to the top, index `decode` again for the second. That entry is the symbol whose
    /// code those bits start with if its code is no longer than they are, which is when the two
    /// codes together fit in the index.
    fn build_pairs(&mut self) {
        if self.pairs_built {
            return;
        }
        if self.pairs.len() != TABLE_SIZE {
            self.pairs.resize(TABLE_SIZE, PairEntry::default());
        }
        let single: &[Entry; TABLE_SIZE] = self
            .decode
            .as_slice()
            .try_into()
            .expect("a table is built before it decodes");
        let pairs: &mut [PairEntry; TABLE_SIZE] =
            self.pairs.as_mut_slice().try_into().expect("sized above");
        for (index, pair) in pairs.iter_mut().enumerate() {
            let first = single[index];
            let second = single[(index << first.num_bits) & (TABLE_SIZE - 1)];
            let both = first.num_bits + second.num_bits;
            *pair = if both <= MAX_MAX_NUM_BITS {
                PairEntry::new(first.symbol, second.symbol, both, 2)
            } else {
                PairEntry::new(first.symbol, 0, first.num_bits, 1)
            };
        }
        self.pairs_built = true;
    }

    /// The table of pairs, as an array; only once [`Self::build_pairs`] has built it.
    #[inline(always)]
    fn pairs(&self) -> &[PairEntry; TABLE_SIZE] {
        debug_assert!(self.pairs_built);
        self.pairs
            .as_slice()
            .try_into()
            .expect("the pairs are built before they decode")
    }

    /// Decode the one or two symbols at the top of `window` into `out` at `*at`, and shift their
    /// bits out. Both bytes of the entry are written; the second is only kept, `*at` moved past
    /// it, when the entry holds two symbols, else the next entry overwrites it.
    #[inline(always)]
    fn decode_pair_aligned(
        pairs: &[PairEntry; TABLE_SIZE],
        window: &mut u64,
        out: &mut [u8; PAIRS_OUT],
        at: &mut usize,
    ) {
        let entry = pairs[(*window >> (64 - MAX_MAX_NUM_BITS)) as usize];
        out[*at..*at + 2].copy_from_slice(&entry.symbols);
        *window <<= entry.num_bits;
        *at += entry.len();
    }

    /// [`Self::decode_4_streams`] from the table of pairs, [`PAIRS_A_REFILL`] lookups a stream a
    /// refill, while every stream has [`PAIRS_OUT`] bytes of output left to take them; the rest
    /// one symbol at a time from `decode`.
    fn decode_4_streams_pairs(&self, brs: &mut [BitReaderReversed<'_>; 4], outs: [&mut [u8]; 4]) {
        let pairs = self.pairs();
        let [out0, out1, out2, out3] = outs;
        let [mut b0, mut b1, mut b2, mut b3] = brs.clone();
        let (mut at0, mut at1, mut at2, mut at3) = (0, 0, 0, 0);
        let can_refill = |b: [&BitReaderReversed<'_>; 4]| b.iter().all(|b| b.can_refill_fast());
        if can_refill([&b0, &b1, &b2, &b3]) {
            b0.ensure_bits_fast(57);
            b1.ensure_bits_fast(57);
            b2.ensure_bits_fast(57);
            b3.ensure_bits_fast(57);
            let (mut w0, mut w1, mut w2, mut w3) = (
                b0.sentinel_window(),
                b1.sentinel_window(),
                b2.sentinel_window(),
                b3.sentinel_window(),
            );
            // While every stream has the output a refill's lookups may write to.
            while let (Some(c0), Some(c1), Some(c2), Some(c3)) = (
                room(out0, at0),
                room(out1, at1),
                room(out2, at2),
                room(out3, at3),
            ) {
                let (mut d0, mut d1, mut d2, mut d3) = (0, 0, 0, 0);
                for _ in 0..PAIRS_A_REFILL {
                    Self::decode_pair_aligned(pairs, &mut w0, c0, &mut d0);
                    Self::decode_pair_aligned(pairs, &mut w1, c1, &mut d1);
                    Self::decode_pair_aligned(pairs, &mut w2, c2, &mut d2);
                    Self::decode_pair_aligned(pairs, &mut w3, c3, &mut d3);
                }
                at0 += d0;
                at1 += d1;
                at2 += d2;
                at3 += d3;
                if !can_refill([&b0, &b1, &b2, &b3]) {
                    break;
                }
                w0 = b0.refill_window_fast(w0);
                w1 = b1.refill_window_fast(w1);
                w2 = b2.refill_window_fast(w2);
                w3 = b3.refill_window_fast(w3);
            }
            b0.consume_to_sentinel(w0);
            b1.consume_to_sentinel(w1);
            b2.consume_to_sentinel(w2);
            b3.consume_to_sentinel(w3);
        }

        *brs = [b0, b1, b2, b3];
        let [br0, br1, br2, br3] = brs;
        self.decode_stream(br0, &mut out0[at0..]);
        self.decode_stream(br1, &mut out1[at1..]);
        self.decode_stream(br2, &mut out2[at2..]);
        self.decode_stream(br3, &mut out3[at3..]);
    }

    /// Read from `source` and decode the input, populating the huffman decoding table.
    ///
    /// Returns the number of bytes read.
    pub fn build_decoder(&mut self, source: &[u8]) -> Result<u32, HuffmanTableError> {
        // No table until this one is built, so that a block repeating a table after a failed
        // build is refused as having none, not decoded from half of one.
        self.max_num_bits = 0;
        self.pairs_built = false;
        let bytes_used = self.read_weights(source)?;
        self.build_table_from_weights()?;
        Ok(bytes_used)
    }

    /// Read weights from the provided source.
    ///
    /// The huffman table is represented in the input data as a list of weights.
    /// After the header, weights are read, then a Huffman decoding table
    /// can be constructed using that list of weights.
    ///
    /// Returns the number of bytes read.
    fn read_weights(&mut self, source: &[u8]) -> Result<u32, HuffmanTableError> {
        use HuffmanTableError as err;

        if source.is_empty() {
            return Err(err::SourceIsEmpty);
        }
        let header = source[0];
        let mut bits_read = 8;

        match header {
            // If the header byte is less than 128, the series of weights
            // is compressed using two interleaved FSE streams that share
            // a distribution table.
            0..=127 => {
                let fse_stream = &source[1..];
                if header as usize > fse_stream.len() {
                    return Err(err::NotEnoughBytesForWeights {
                        got_bytes: fse_stream.len(),
                        expected_bytes: header,
                    });
                }
                //fse decompress weights
                let bytes_used_by_fse_header = self.fse_table.build_decoder(fse_stream, 6)?;

                if bytes_used_by_fse_header > header as usize {
                    return Err(err::FSETableUsedTooManyBytes {
                        used: bytes_used_by_fse_header,
                        available_bytes: header,
                    });
                }

                vprintln!(
                    "Building fse table for huffman weights used: {}",
                    bytes_used_by_fse_header
                );
                // Huffman headers are compressed using two interleaved
                // FSE bitstreams, where the first state (decoder) handles
                // even symbols, and the second handles odd symbols.
                let mut dec1 = FSEDecoder::new(&self.fse_table);
                let mut dec2 = FSEDecoder::new(&self.fse_table);

                let compressed_start = bytes_used_by_fse_header;
                let compressed_length = header as usize - bytes_used_by_fse_header;

                let compressed_weights = &fse_stream[compressed_start..];
                if compressed_weights.len() < compressed_length {
                    return Err(err::NotEnoughBytesToDecompressWeights {
                        have: compressed_weights.len(),
                        need: compressed_length,
                    });
                }
                let compressed_weights = &compressed_weights[..compressed_length];
                let mut br = BitReaderReversed::new(compressed_weights);

                bits_read += (bytes_used_by_fse_header + compressed_length) * 8;

                //skip the 0 padding at the end of the last byte of the bit stream and throw away the first 1 found
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
                    return Err(err::ExtraPadding { skipped_bits });
                }

                dec1.init_state(&mut br)?;
                dec2.init_state(&mut br)?;

                self.weights.clear();

                // The two decoders take turns decoding a single symbol and updating their state.
                loop {
                    let w = dec1.decode_symbol();
                    self.weights.push(w);
                    dec1.update_state(&mut br);

                    if br.bits_remaining() <= -1 {
                        //collect final states
                        self.weights.push(dec2.decode_symbol());
                        break;
                    }

                    let w = dec2.decode_symbol();
                    self.weights.push(w);
                    dec2.update_state(&mut br);

                    if br.bits_remaining() <= -1 {
                        //collect final states
                        self.weights.push(dec1.decode_symbol());
                        break;
                    }
                    //maximum number of weights is 255 because we use u8 symbols and the last weight is inferred from the sum of all others
                    if self.weights.len() > 255 {
                        return Err(err::TooManyWeights {
                            got: self.weights.len(),
                        });
                    }
                }
            }
            // If the header byte is greater than or equal to 128,
            // weights are directly represented, where each weight is
            // encoded directly as a 4 bit field. The weights will
            // always be encoded with full bytes, meaning if there's
            // an odd number of weights, the last weight will still
            // occupy a full byte.
            _ => {
                // weights are directly encoded
                let weights_raw = &source[1..];
                let num_weights = header - 127;
                self.weights.resize(num_weights as usize, 0);

                let bytes_needed = if num_weights.is_multiple_of(2) {
                    num_weights as usize / 2
                } else {
                    (num_weights as usize / 2) + 1
                };

                if weights_raw.len() < bytes_needed {
                    return Err(err::NotEnoughBytesInSource {
                        got: weights_raw.len(),
                        need: bytes_needed,
                    });
                }

                for idx in 0..num_weights {
                    if idx % 2 == 0 {
                        self.weights[idx as usize] = weights_raw[idx as usize / 2] >> 4;
                    } else {
                        self.weights[idx as usize] = weights_raw[idx as usize / 2] & 0xF;
                    }
                    bits_read += 4;
                }
            }
        }

        let bytes_read = if bits_read % 8 == 0 {
            bits_read / 8
        } else {
            (bits_read / 8) + 1
        };
        Ok(bytes_read as u32)
    }

    /// Once the weights have been read from the data, you can decode the weights
    /// into a table, and use that table to decode the actual compressed data.
    ///
    /// This function populates the rest of the table from the series of weights.
    fn build_table_from_weights(&mut self) -> Result<(), HuffmanTableError> {
        use HuffmanTableError as err;

        // At most 255 weights, the last symbol's implied: a symbol is a byte. The weights'
        // reader stops past 255, but its last states can still give one or two more.
        if self.weights.len() > 255 {
            return Err(err::TooManyWeights {
                got: self.weights.len(),
            });
        }
        self.bits.clear();
        self.bits.resize(self.weights.len() + 1, 0);

        let mut weight_sum: u32 = 0;
        for w in &self.weights {
            if *w > MAX_MAX_NUM_BITS {
                return Err(err::WeightBiggerThanMaxNumBits { got: *w });
            }
            weight_sum += if *w > 0 { 1_u32 << (*w - 1) } else { 0 };
        }

        if weight_sum == 0 {
            return Err(err::MissingWeights);
        }

        let max_bits = highest_bit_set(weight_sum) as u8;
        let left_over = (1 << max_bits) - weight_sum;

        //left_over must be power of two
        if !left_over.is_power_of_two() {
            return Err(err::LeftoverIsNotAPowerOf2 { got: left_over });
        }

        let last_weight = highest_bit_set(left_over) as u8;

        for symbol in 0..self.weights.len() {
            let bits = if self.weights[symbol] > 0 {
                max_bits + 1 - self.weights[symbol]
            } else {
                0
            };
            self.bits[symbol] = bits;
        }

        self.bits[self.weights.len()] = max_bits + 1 - last_weight;

        if max_bits > MAX_MAX_NUM_BITS {
            return Err(err::MaxBitsTooHigh { got: max_bits });
        }

        self.bit_ranks.clear();
        self.bit_ranks.resize((max_bits + 1) as usize, 0);
        for num_bits in &self.bits {
            self.bit_ranks[(*num_bits) as usize] += 1;
        }

        //starting codes for each rank
        self.rank_indexes.clear();
        self.rank_indexes.resize((max_bits + 1) as usize, 0);

        // The table is laid out at MAX_MAX_NUM_BITS bits whatever max_bits is (see TABLE_SIZE):
        // a code of `bits` bits spans the entries of every value of the bits after it.
        self.rank_indexes[max_bits as usize] = 0;
        for bits in (1..self.rank_indexes.len() as u8).rev() {
            self.rank_indexes[bits as usize - 1] = self.rank_indexes[bits as usize]
                + self.bit_ranks[bits as usize] as usize * (1 << (MAX_MAX_NUM_BITS - bits));
        }

        assert!(
            self.rank_indexes[0] == TABLE_SIZE,
            "rank_idx[0]: {} should be: {}",
            self.rank_indexes[0],
            TABLE_SIZE
        );

        // Sized once; every later build overwrites all of it.
        if self.decode.len() != TABLE_SIZE {
            self.decode.resize(TABLE_SIZE, Entry::default());
        }
        // The symbols by their codes' lengths, in order within each, so that a length's are
        // filled in a loop of their own, every code the same number of entries: no branch on
        // a symbol's length, and stores as wide as its entries (see `fill_rank`).
        let mut sorted = [0u8; 256];
        let mut next = [0usize; MAX_MAX_NUM_BITS as usize + 1];
        let mut start = 0;
        for (next, &count) in next.iter_mut().zip(&self.bit_ranks).skip(1) {
            *next = start;
            start += count as usize;
        }
        for (symbol, &bits) in self.bits.iter().enumerate() {
            if bits != 0 {
                let next = &mut next[bits as usize];
                sorted[*next] = symbol as u8;
                *next += 1;
            }
        }
        let mut from = 0;
        for num_bits in 1..=max_bits {
            let count = self.bit_ranks[num_bits as usize] as usize;
            let symbols = &sorted[from..from + count];
            from += count;
            let at = self.rank_indexes[num_bits as usize];
            let entries = &mut self.decode[at..at + (count << (MAX_MAX_NUM_BITS - num_bits))];
            match num_bits {
                11 => fill_rank::<1>(entries, symbols, num_bits),
                10 => fill_rank::<2>(entries, symbols, num_bits),
                9 => fill_rank::<4>(entries, symbols, num_bits),
                8 => fill_rank::<8>(entries, symbols, num_bits),
                7 => fill_rank::<16>(entries, symbols, num_bits),
                6 => fill_rank::<32>(entries, symbols, num_bits),
                5 => fill_rank::<64>(entries, symbols, num_bits),
                4 => fill_rank::<128>(entries, symbols, num_bits),
                3 => fill_rank::<256>(entries, symbols, num_bits),
                2 => fill_rank::<512>(entries, symbols, num_bits),
                _ => fill_rank::<1024>(entries, symbols, num_bits),
            }
        }

        self.max_num_bits = max_bits;
        Ok(())
    }
}

/// Fill `entries` with the codes of `symbols`, each `num_bits` long and so `L` entries.
#[inline(always)]
fn fill_rank<const L: usize>(entries: &mut [Entry], symbols: &[u8], num_bits: u8) {
    for (code, &symbol) in entries.as_chunks_mut::<L>().0.iter_mut().zip(symbols) {
        *code = [Entry { symbol, num_bits }; L];
    }
}

impl Default for HuffmanTable {
    fn default() -> Self {
        Self::new()
    }
}

/// A single entry in the table contains the decoded symbol/literal and the
/// size of the prefix code.
#[derive(Copy, Clone, Debug, Default)]
pub struct Entry {
    /// The byte that the prefix code replaces during encoding.
    symbol: u8,
    /// The number of bits the prefix code occupies.
    num_bits: u8,
}

/// An entry of the table of pairs: the one or two symbols an index starts with, and their codes'
/// bits together.
#[derive(Copy, Clone, Debug, Default)]
struct PairEntry {
    /// The first symbol, then the second, or anything when there is one.
    symbols: [u8; 2],
    num_bits: u8,
    /// 1 when the entry holds two symbols, else 0.
    has_second: u8,
}

impl PairEntry {
    fn new(first: u8, second: u8, num_bits: u8, len: u8) -> PairEntry {
        PairEntry {
            symbols: [first, second],
            num_bits,
            has_second: len - 1,
        }
    }

    /// How many symbols the entry holds, 1 or 2; masked so that the compiler knows, and the
    /// stores of a refill's lookups need no bounds checks.
    #[inline(always)]
    fn len(self) -> usize {
        1 + usize::from(self.has_second & 1)
    }
}

/// Lookups a stream from the table of pairs between refills: each takes at most
/// `MAX_MAX_NUM_BITS` bits, of the 56 a refill leaves.
const PAIRS_A_REFILL: usize = 56 / MAX_MAX_NUM_BITS as usize;

/// The output a stream needs left for a refill's lookups from the table of pairs: two bytes
/// each, and every one written at most two past the last.
const PAIRS_OUT: usize = 2 * PAIRS_A_REFILL;

/// Whether the four streams of a block's literals decode faster from the table of pairs, its
/// build included, than one symbol at a time: libzstd's `HUF_selectDecoder`, from the literals'
/// size and how far they are compressed. The table of pairs costs more to build and saves more,
/// the more of the codes are short, which is when the literals compress well.
fn choose_pairs(regenerated: usize, compressed: usize) -> bool {
    // libzstd's estimates, for each sixteenth of the ratio of the compressed size to the
    // regenerated: the time to build each table, then to decode 256 bytes from it, for the
    // table of single symbols and then of pairs.
    const COSTS: [[(u32, u32); 2]; 16] = [
        [(0, 0), (1, 1)],
        [(0, 0), (1, 1)],
        [(150, 216), (381, 119)],
        [(170, 205), (514, 112)],
        [(177, 199), (539, 110)],
        [(197, 194), (644, 107)],
        [(221, 192), (735, 107)],
        [(256, 189), (881, 106)],
        [(359, 188), (1167, 109)],
        [(582, 187), (1570, 114)],
        [(688, 187), (1712, 122)],
        [(825, 186), (1965, 136)],
        [(976, 185), (2131, 150)],
        [(1180, 186), (2070, 175)],
        [(1377, 185), (1731, 202)],
        [(1412, 185), (1695, 202)],
    ];
    if regenerated == 0 {
        return false;
    }
    let ratio = if compressed >= regenerated {
        15
    } else {
        compressed * 16 / regenerated
    };
    let blocks = (regenerated >> 8) as u32;
    let [(build1, decode1), (build2, decode2)] = COSTS[ratio];
    let single = build1 + decode1 * blocks;
    let pairs = build2 + decode2 * blocks;
    // A small advantage to the single symbols' table, which is smaller, as libzstd gives it.
    pairs + (pairs >> 5) < single
}

/// The [`PAIRS_OUT`] bytes of `out` from `at`, if it has them.
#[inline(always)]
fn room(out: &mut [u8], at: usize) -> Option<&mut [u8; PAIRS_OUT]> {
    out.get_mut(at..)?.first_chunk_mut()
}

/// Assert that the provided value is greater than zero, and returns the
/// 32 - the number of leading zeros
fn highest_bit_set(x: u32) -> u32 {
    assert!(x > 0);
    u32::BITS - x.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 256 weights, which with the implied last are a symbol past a byte's, are refused, even
    /// when they make a whole code (as 256 of weight 1 do); the FSE-coded weights can come to
    /// that many, as a fuzzed dictionary's did.
    #[test]
    fn too_many_weights() {
        let mut table = HuffmanTable::new();
        table.weights = alloc::vec![1; 256];
        assert!(matches!(
            table.build_table_from_weights(),
            Err(HuffmanTableError::TooManyWeights { got: 256 })
        ));
        table.weights = alloc::vec![1; 255];
        table.weights[0] = 2;
        assert!(table.build_table_from_weights().is_ok());
    }

    /// The table of pairs decodes what the single symbols' table does, bit for bit, whatever
    /// the code, the streams' lengths and the output's.
    #[test]
    fn pairs_decode_as_singles() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..2000 {
            // A complete prefix code: leaves split until there are enough, none over 11 bits.
            let leaves = 2 + next(60);
            let mut lengths = alloc::vec![0u8];
            while lengths.len() < leaves {
                let at = next(lengths.len());
                if lengths[at] < MAX_MAX_NUM_BITS {
                    lengths[at] += 1;
                    lengths.push(lengths[at]);
                }
            }
            let max_bits = *lengths.iter().max().unwrap();
            // Given to distinct symbols up to 128, the highest one's weight left implied.
            let mut weights = alloc::vec![0u8; 129];
            let mut symbols: Vec<usize> = (0..129).collect();
            for &len in &lengths {
                let symbol = symbols.swap_remove(next(symbols.len()));
                weights[symbol] = max_bits + 1 - len;
            }
            let last = weights.iter().rposition(|&w| w > 0).unwrap();
            let mut description = alloc::vec![127 + last as u8];
            for two in weights[..last].chunks(2) {
                description.push(two[0] << 4 | two.get(1).copied().unwrap_or(0));
            }

            let mut table = HuffmanTable::new();
            table.build_decoder(&description).unwrap();
            assert_eq!(table.max_num_bits, max_bits);
            // The entries as the table was filled before, a symbol at a time.
            let mut expected = Vec::new();
            for bits in (1..=MAX_MAX_NUM_BITS).rev() {
                for (symbol, _) in table.bits.iter().enumerate().filter(|(_, b)| **b == bits) {
                    let len = 1 << (MAX_MAX_NUM_BITS - bits);
                    expected.extend(core::iter::repeat_n((symbol as u8, bits), len));
                }
            }
            let got: Vec<_> = table
                .decode
                .iter()
                .map(|e| (e.symbol, e.num_bits))
                .collect();
            assert_eq!(got, expected);
            table.build_pairs();

            let sources: [Vec<u8>; 4] =
                core::array::from_fn(|_| (0..next(200)).map(|_| next(256) as u8).collect());
            let lens: [usize; 4] = core::array::from_fn(|_| next(400));
            let readers = || sources.each_ref().map(|s| BitReaderReversed::new(s));
            let mut singles = lens.map(|len| alloc::vec![0u8; len]);
            let mut pairs = singles.clone();
            let mut by_singles = readers();
            let mut by_pairs = readers();
            table.decode_4_streams(
                &mut by_singles,
                singles.each_mut().map(|v| v.as_mut_slice()),
            );
            table.decode_4_streams_pairs(&mut by_pairs, pairs.each_mut().map(|v| v.as_mut_slice()));
            assert_eq!(singles, pairs);
            for (a, b) in by_singles.iter().zip(&by_pairs) {
                assert_eq!(a.bits_remaining(), b.bits_remaining());
            }
        }
    }
}
