//! Utilities for decoding Huff0 encoded huffman data.

use crate::bit_io::BitReaderReversed;
use crate::decoding::errors::HuffmanTableError;
use crate::fse::{FSETable, build_states};
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
    /// The dictionary whose table `decode` holds as copied, kept through resets, so that a frame
    /// using it again need not copy it in (see [`Self::reinit_from_dict`]); `None` once anything
    /// else may have been built in it.
    dict: Option<u32>,
    /// Whether `pairs` is made from that dictionary's table, kept through resets as `dict` is.
    dict_pairs: bool,
    /// The maximum size in bits a prefix code in the encoded data can be.
    /// This value is used so that the decoder knows how many bits
    /// to read from the bitstream before checking the table. This
    /// value must be 11 or lower.
    pub max_num_bits: u8,
    /// The description of the FSE table the weights are compressed with, when they are; kept
    /// only so that its allocation is reused from one build to the next.
    fse_table: FSETable,
}

impl HuffmanTable {
    /// Create a new, empty table.
    pub fn new() -> HuffmanTable {
        HuffmanTable {
            decode: Vec::new(),
            pairs: Vec::new(),
            pairs_built: false,
            dict: None,
            dict_pairs: false,
            max_num_bits: 0,
            fse_table: FSETable::new(255),
        }
    }

    /// Empty the table, then make it a replica of `other`, the table of dictionary `dict_id`.
    /// Its lookup table, and the table of pairs made from it, are copied only if this table no
    /// longer holds them from an earlier frame.
    pub fn reinit_from_dict(&mut self, other: &Self, dict_id: u32) {
        self.reset();
        if self.dict != Some(dict_id) {
            self.decode.clear();
            self.decode.extend_from_slice(&other.decode);
            self.dict = Some(dict_id);
            self.dict_pairs = false;
        }
        self.pairs_built = self.dict_pairs;
        self.max_num_bits = other.max_num_bits;
    }

    /// Leave no table to decode with, as a new one has none.
    pub fn reset(&mut self) {
        self.max_num_bits = 0;
        self.pairs_built = false;
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
        let [symbol, num_bits] = entry;
        br.consume(num_bits);
        symbol
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
        let [symbol, num_bits] = entry;
        *window <<= num_bits;
        symbol
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
            let [first, first_bits] = single[index];
            let [second, second_bits] = single[(index << first_bits) & (TABLE_SIZE - 1)];
            let both = first_bits + second_bits;
            *pair = if both <= MAX_MAX_NUM_BITS {
                PairEntry::new(first, second, both, 2)
            } else {
                PairEntry::new(first, 0, first_bits, 1)
            };
        }
        self.pairs_built = true;
        self.dict_pairs = self.dict.is_some();
    }

    /// Forget which dictionary's table this one holds, so that the next frame using one copies
    /// it in: for when a dictionary of the same id may have replaced it.
    pub fn forget_dict(&mut self) {
        self.dict = None;
        self.dict_pairs = false;
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
    fn decode_pair_aligned<const N: usize>(
        pairs: &[PairEntry; TABLE_SIZE],
        window: &mut u64,
        out: &mut [u8; N],
        at: &mut usize,
    ) {
        let entry = pairs[(*window >> (64 - MAX_MAX_NUM_BITS)) as usize];
        out[*at..*at + 2].copy_from_slice(&entry.symbols());
        *window = entry.shift(*window);
        *at += entry.len();
    }

    /// [`Self::decode_4_streams`] from the table of pairs, in rounds of [`PAIRS_ROUND`] refills
    /// and [`PAIRS_A_REFILL`] lookups a stream after each, while every stream has the output a
    /// round's lookups may write to and can refill fast that many times; then each stream on its
    /// own (see [`Self::decode_stream_pairs`]).
    fn decode_4_streams_pairs(&self, brs: &mut [BitReaderReversed<'_>; 4], outs: [&mut [u8]; 4]) {
        let pairs = self.pairs();
        let [out0, out1, out2, out3] = outs;
        let [mut b0, mut b1, mut b2, mut b3] = brs.clone();
        let (mut at0, mut at1, mut at2, mut at3) = (0, 0, 0, 0);
        let can_refill =
            |b: [&BitReaderReversed<'_>; 4]| b.iter().all(|b| b.can_refill_fast_times(PAIRS_ROUND));
        // The last offset of each stream's output from which a round's lookups have the room
        // they may write to: one compare a stream a round, from which the compiler also knows
        // the round's slice of the output is in bounds.
        const ROUND_OUT: usize = PAIRS_ROUND * PAIRS_OUT;
        // The tail a stream needs for a round's refills, as `refill_tail` counts them.
        const ROUND_TAIL: usize = 8 + 8 * PAIRS_ROUND;
        let limits = (
            out0.len().checked_sub(ROUND_OUT),
            out1.len().checked_sub(ROUND_OUT),
            out2.len().checked_sub(ROUND_OUT),
            out3.len().checked_sub(ROUND_OUT),
        );
        if let (Some(l0), Some(l1), Some(l2), Some(l3)) = limits
            && can_refill([&b0, &b1, &b2, &b3])
        {
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
            // The refills from each source's tail, checked against its length once a round.
            let (Some(mut t0), Some(mut t1), Some(mut t2), Some(mut t3)) =
                (b0.tail(), b1.tail(), b2.tail(), b3.tail())
            else {
                unreachable!("each container holds its 8 bytes after a fast refill")
            };
            // While every stream has the output a round's lookups may write to.
            while at0 <= l0 && at1 <= l1 && at2 <= l2 && at3 <= l3 {
                let (Some(c0), Some(c1), Some(c2), Some(c3)) = (
                    room::<ROUND_OUT>(out0, at0),
                    room::<ROUND_OUT>(out1, at1),
                    room::<ROUND_OUT>(out2, at2),
                    room::<ROUND_OUT>(out3, at3),
                ) else {
                    unreachable!("each offset is at most its limit")
                };
                let (mut d0, mut d1, mut d2, mut d3) = (0, 0, 0, 0);
                for refill in 0..PAIRS_ROUND {
                    // The check before the round's first refill was for all of its refills.
                    if refill > 0 {
                        w0 = BitReaderReversed::refill_tail(&mut t0, w0);
                        w1 = BitReaderReversed::refill_tail(&mut t1, w1);
                        w2 = BitReaderReversed::refill_tail(&mut t2, w2);
                        w3 = BitReaderReversed::refill_tail(&mut t3, w3);
                    }
                    for _ in 0..PAIRS_A_REFILL {
                        Self::decode_pair_aligned(pairs, &mut w0, c0, &mut d0);
                        Self::decode_pair_aligned(pairs, &mut w1, c1, &mut d1);
                        Self::decode_pair_aligned(pairs, &mut w2, c2, &mut d2);
                        Self::decode_pair_aligned(pairs, &mut w3, c3, &mut d3);
                    }
                }
                at0 += d0;
                at1 += d1;
                at2 += d2;
                at3 += d3;
                if !(t0.len() >= ROUND_TAIL
                    && t1.len() >= ROUND_TAIL
                    && t2.len() >= ROUND_TAIL
                    && t3.len() >= ROUND_TAIL)
                {
                    break;
                }
                w0 = BitReaderReversed::refill_tail(&mut t0, w0);
                w1 = BitReaderReversed::refill_tail(&mut t1, w1);
                w2 = BitReaderReversed::refill_tail(&mut t2, w2);
                w3 = BitReaderReversed::refill_tail(&mut t3, w3);
            }
            b0.resume_tail(t0, w0);
            b1.resume_tail(t1, w1);
            b2.resume_tail(t2, w2);
            b3.resume_tail(t3, w3);
        }

        *brs = [b0, b1, b2, b3];
        let [br0, br1, br2, br3] = brs;
        self.decode_stream_pairs(br0, &mut out0[at0..]);
        self.decode_stream_pairs(br1, &mut out1[at1..]);
        self.decode_stream_pairs(br2, &mut out2[at2..]);
        self.decode_stream_pairs(br3, &mut out3[at3..]);
    }

    /// The rest of one stream after the four together, which stop at the first to near its
    /// source's end: from the table of pairs, as they do, while it can refill fast and has the
    /// output; the rest one symbol at a time from `decode`.
    fn decode_stream_pairs(&self, br: &mut BitReaderReversed<'_>, out: &mut [u8]) {
        let pairs = self.pairs();
        let mut b = br.clone();
        let mut at = 0;
        if b.can_refill_fast() {
            b.ensure_bits_fast(57);
            let mut w = b.sentinel_window();
            while let Some(c) = room::<PAIRS_OUT>(out, at) {
                let mut d = 0;
                for _ in 0..PAIRS_A_REFILL {
                    Self::decode_pair_aligned(pairs, &mut w, c, &mut d);
                }
                at += d;
                if !b.can_refill_fast() {
                    break;
                }
                w = b.refill_window_fast(w);
            }
            b.consume_to_sentinel(w);
        }
        *br = b;
        self.decode_stream(br, &mut out[at..]);
    }

    /// Read from `source` and decode the input, populating the huffman decoding table.
    ///
    /// Returns the number of bytes read.
    pub fn build_decoder(&mut self, source: &[u8]) -> Result<u32, HuffmanTableError> {
        // No table until this one is built, so that a block repeating a table after a failed
        // build is refused as having none, not decoded from half of one.
        self.max_num_bits = 0;
        self.pairs_built = false;
        self.dict = None;
        self.dict_pairs = false;
        let mut weights = [0; WEIGHTS_MOST];
        let (bytes_used, count) = self.read_weights(source, &mut weights)?;
        self.build_table_from_weights(&weights[..count])?;
        Ok(bytes_used)
    }

    /// Read the weights described at the start of `source` into `weights`: a symbol's weight
    /// gives the length of its code, and the last symbol's is left implied.
    ///
    /// Returns the number of bytes read and the number of weights.
    fn read_weights(
        &mut self,
        source: &[u8],
        weights: &mut [u8; WEIGHTS_MOST],
    ) -> Result<(u32, usize), HuffmanTableError> {
        use HuffmanTableError as err;

        let Some(&header) = source.first() else {
            return Err(err::SourceIsEmpty);
        };
        if header < 128 {
            // `header` bytes of two interleaved FSE streams sharing one table, after the
            // table's description: the first state decodes the even weights, the second the
            // odd ones.
            let fse_stream = &source[1..];
            if header as usize > fse_stream.len() {
                return Err(err::NotEnoughBytesForWeights {
                    got_bytes: fse_stream.len(),
                    expected_bytes: header,
                });
            }
            let description = self.fse_table.read_distribution(fse_stream, 6)?;
            if description > header as usize {
                return Err(err::FSETableUsedTooManyBytes {
                    used: description,
                    available_bytes: header,
                });
            }
            let mut states = [WeightState::default(); WEIGHT_STATES];
            build_states(
                self.fse_table.accuracy_log,
                &self.fse_table.symbol_probabilities,
                |state, symbol, num_bits, base| {
                    states[state & (WEIGHT_STATES - 1)] = WeightState {
                        base: base as u8,
                        num_bits,
                        symbol,
                    };
                },
            )?;
            let count = decode_weights(
                &fse_stream[description..header as usize],
                &states,
                self.fse_table.accuracy_log,
                weights,
            )?;
            Ok((1 + u32::from(header), count))
        } else {
            // `header - 127` weights of 4 bits each, two to a byte, the first in the high bits.
            let count = usize::from(header - 127);
            let bytes = count.div_ceil(2);
            let Some(packed) = source.get(1..1 + bytes) else {
                return Err(err::NotEnoughBytesInSource {
                    got: source.len() - 1,
                    need: bytes,
                });
            };
            for (two, &byte) in weights.as_chunks_mut::<2>().0.iter_mut().zip(packed) {
                *two = [byte >> 4, byte & 0xF];
            }
            Ok((1 + bytes as u32, count))
        }
    }

    /// Build the table from the weights read, the last symbol's left implied.
    fn build_table_from_weights(&mut self, weights: &[u8]) -> Result<(), HuffmanTableError> {
        use HuffmanTableError as err;

        // At most 255 weights, the last symbol's implied: a symbol is a byte. The weights'
        // reader stops past 255, but its last states can still give one or two more.
        if weights.len() > 255 {
            return Err(err::TooManyWeights { got: weights.len() });
        }

        // The symbols with a code, in order: most weights are 0, and counting or sorting them
        // all would make a chain of increments of one count through memory, each waiting on the
        // one before. The list's end is a register, moved on past a symbol with a weight.
        let mut coded = [0u8; 256];
        let mut coded_len = 0;
        let mut highest = 0;
        for (symbol, &w) in weights.iter().enumerate() {
            coded[coded_len & 255] = symbol as u8;
            coded_len += usize::from(w != 0);
            highest = highest.max(w);
        }
        if highest > MAX_MAX_NUM_BITS {
            return Err(err::WeightBiggerThanMaxNumBits { got: highest });
        }
        let coded = &coded[..coded_len];

        // How many symbols have each weight: the codes' lengths, their counts and the sum all
        // follow from these.
        let mut counts = [0u32; MAX_MAX_NUM_BITS as usize + 1];
        for &symbol in coded {
            counts[usize::from(weights[usize::from(symbol)])] += 1;
        }
        let weight_sum: u32 = (1..=MAX_MAX_NUM_BITS)
            .map(|w| counts[usize::from(w)] << (w - 1))
            .sum();
        if weight_sum == 0 {
            return Err(err::MissingWeights);
        }

        // The weights sum, with the last symbol's, to the next power of two, which gives the
        // longest code: a weight `w` is a code of `max_bits + 1 - w` bits.
        let max_bits = highest_bit_set(weight_sum) as u8;
        let left_over = (1 << max_bits) - weight_sum;
        if !left_over.is_power_of_two() {
            return Err(err::LeftoverIsNotAPowerOf2 { got: left_over });
        }
        if max_bits > MAX_MAX_NUM_BITS {
            return Err(err::MaxBitsTooHigh { got: max_bits });
        }
        let last_weight = highest_bit_set(left_over) as u8;
        counts[usize::from(last_weight)] += 1;

        // The symbols by weight, in order within each, so that a code length's are filled in a
        // loop of their own, every code the same number of entries: no branch on a symbol's
        // length, and stores as wide as its entries (see `fill_rank`).
        let mut sorted = [0u8; 256];
        let mut next = [0usize; MAX_MAX_NUM_BITS as usize + 1];
        let mut start = 0;
        for (next, &count) in next.iter_mut().zip(&counts).skip(1) {
            *next = start;
            start += count as usize;
        }
        for &symbol in coded {
            let next = &mut next[usize::from(weights[usize::from(symbol)])];
            sorted[*next & 255] = symbol;
            *next += 1;
        }
        sorted[next[usize::from(last_weight)] & 255] = weights.len() as u8;

        // The table is laid out at MAX_MAX_NUM_BITS bits whatever max_bits is (see TABLE_SIZE):
        // a code of `bits` bits spans the entries of every value of the bits after it. The
        // longest codes, of weight 1, come first.
        if self.decode.len() != TABLE_SIZE {
            self.decode.resize(TABLE_SIZE, [0; 2]);
        }
        let mut from = 0;
        let mut at = 0;
        for w in 1..=max_bits {
            let count = counts[usize::from(w)] as usize;
            let symbols = &sorted[from..from + count];
            from += count;
            let num_bits = max_bits + 1 - w;
            let len = count << (MAX_MAX_NUM_BITS - num_bits);
            let entries = &mut self.decode[at..at + len];
            at += len;
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
        debug_assert_eq!(at, TABLE_SIZE);

        self.max_num_bits = max_bits;
        Ok(())
    }
}

/// Room for the weights read: at most 255 and the two final states', which
/// [`HuffmanTable::build_table_from_weights`] then refuses.
const WEIGHTS_MOST: usize = 258;

/// The FSE table the weights are compressed with has an accuracy log of at most 6.
const WEIGHT_STATES: usize = 1 << 6;

/// A state of the weights' FSE table: its weight, and the bits read for the next state and the
/// base they are added to. The base is below the table's size, so a byte.
#[derive(Copy, Clone, Default)]
struct WeightState {
    base: u8,
    num_bits: u8,
    symbol: u8,
}

/// Decode the weights' two interleaved FSE streams, `stream`, with `states`, the table of
/// accuracy log `acc_log`, into `weights`, and return how many.
///
/// The stream is read back to front from a copy after 8 zero bytes, which read as the zeros
/// past its start. The bits are taken off the top of a window, the next one highest, loaded
/// from the 8 bytes up to the position and reloaded when fewer than the two decoders' next
/// reads are left in it: the position, the window and the states stay in registers. The
/// decoders stop at the first read that takes a bit past the stream's start; until then a
/// state reads at most 6 bits, and the first two `acc_log` each, so no read reaches past the
/// zeros.
fn decode_weights(
    stream: &[u8],
    states: &[WeightState; WEIGHT_STATES],
    acc_log: u8,
    weights: &mut [u8; WEIGHTS_MOST],
) -> Result<usize, HuffmanTableError> {
    const ZEROS: usize = 8;
    /// The bits two reads take at most.
    const TWO_READS: u32 = 12;
    // At most 127 bytes are the weights' and their table's.
    let mut buf = [0u8; ZEROS + 128];
    buf[ZEROS..ZEROS + stream.len()].copy_from_slice(stream);
    // The stream ends with a 1 bit after its last bit, in its last byte.
    let last = stream.last().copied().unwrap_or(0);
    if last == 0 {
        return Err(HuffmanTableError::ExtraPadding { skipped_bits: 9 });
    }
    // The bits left, from the copy's start: past the stream's start once below `8 * ZEROS`.
    let mut pos = (ZEROS + stream.len()) * 8 - (last.leading_zeros() as usize + 1);
    // The 8 bytes whose highest holds the next bit, shifted so that it is the window's top
    // one; at least 57 of the window's bits are then the stream's. Only while `pos` is at
    // least `8 * ZEROS`, so that those bytes are in the copy.
    let load = |pos: usize| {
        let at = (pos - 1) / 8 - 7;
        let word = u64::from_le_bytes(buf[at..at + 8].try_into().unwrap());
        let left = (pos - 8 * at) as u32;
        (word << (64 - left), left)
    };
    let (mut window, mut left) = load(pos);
    let read = |window: &mut u64, left: &mut u32, pos: &mut usize, n: u8| {
        // Two shifts, so that a read of 0 bits shifts by at most 63.
        let value = (*window >> 1) >> (63 - n);
        *window <<= n;
        *left -= u32::from(n);
        *pos -= usize::from(n);
        value as usize
    };
    let past_start = |pos: usize| pos < ZEROS * 8;

    let mut state1 = states[read(&mut window, &mut left, &mut pos, acc_log) & (WEIGHT_STATES - 1)];
    let mut state2 = states[read(&mut window, &mut left, &mut pos, acc_log) & (WEIGHT_STATES - 1)];
    let mut count = 0;
    loop {
        if left < TWO_READS {
            (window, left) = load(pos);
        }
        weights[count] = state1.symbol;
        count += 1;
        let bits = read(&mut window, &mut left, &mut pos, state1.num_bits);
        state1 = states[(usize::from(state1.base) + bits) & (WEIGHT_STATES - 1)];
        if past_start(pos) {
            weights[count] = state2.symbol;
            count += 1;
            break;
        }

        weights[count] = state2.symbol;
        count += 1;
        let bits = read(&mut window, &mut left, &mut pos, state2.num_bits);
        state2 = states[(usize::from(state2.base) + bits) & (WEIGHT_STATES - 1)];
        if past_start(pos) {
            weights[count] = state1.symbol;
            count += 1;
            break;
        }
        // At most 255 weights, as a symbol is a byte and the last is implied.
        if count > 255 {
            return Err(HuffmanTableError::TooManyWeights { got: count });
        }
    }
    Ok(count)
}

/// Fill `entries` with the codes of `symbols`, each `num_bits` long and so `L` entries.
///
/// Four entries at a time, as a `u64`, where a code has that many: LLVM stores an array of
/// entries, or of `u16`s, two bytes at a time where it unrolls the copies.
#[inline(always)]
fn fill_rank<const L: usize>(entries: &mut [Entry], symbols: &[u8], num_bits: u8) {
    for (code, &symbol) in entries.as_chunks_mut::<L>().0.iter_mut().zip(symbols) {
        if L >= 4 {
            let four = u64::from(u16::from_le_bytes([symbol, num_bits])) * 0x0001_0001_0001_0001;
            for part in code.as_flattened_mut().as_chunks_mut::<8>().0 {
                *part = four.to_le_bytes();
            }
        } else {
            *code = [[symbol, num_bits]; L];
        }
    }
}

impl Default for HuffmanTable {
    fn default() -> Self {
        Self::new()
    }
}

/// An entry of the table: `[symbol, num_bits]`, the byte a code stands for and the number of
/// bits of the code. Bytes, not a struct's fields, so that the table's build can take the
/// entries as bytes (`as_flattened_mut`) and fill a code's with 8-byte stores (see
/// [`fill_rank`]); LLVM stores two bytes' fields, or a `u16`, two bytes at a time.
type Entry = [u8; 2];

/// An entry of the table of pairs: the one or two symbols an index starts with, and their codes'
/// bits together, in one `u32`, so that the table's build writes it in one store and a lookup
/// takes each part with one or two operations. From the lowest bit: the first symbol and the
/// second (or anything when there is none), 8 bits each; at bit 16, 1 when there is a second;
/// and from bit 20, the bits, or in wasm two to their power (see [`Self::shift`]).
#[derive(Copy, Clone, Debug, Default)]
struct PairEntry(u32);

impl PairEntry {
    fn new(first: u8, second: u8, num_bits: u8, len: u8) -> PairEntry {
        let bits = if cfg!(target_family = "wasm") {
            1 << num_bits
        } else {
            u32::from(num_bits)
        };
        PairEntry(u32::from(first) | u32::from(second) << 8 | u32::from(len - 1) << 16 | bits << 20)
    }

    /// The first symbol, then the second.
    #[inline(always)]
    fn symbols(self) -> [u8; 2] {
        (self.0 as u16).to_le_bytes()
    }

    /// `window` with the entry's bits shifted out. In wasm by a multiplication: V8's code for
    /// x86-64 puts a variable shift's count in `cl`, moving registers around for each, and a
    /// multiplication can take any of them.
    #[inline(always)]
    fn shift(self, window: u64) -> u64 {
        if cfg!(target_family = "wasm") {
            window.wrapping_mul(u64::from(self.0 >> 20))
        } else {
            window << (self.0 >> 20 & 63)
        }
    }

    /// How many symbols the entry holds, 1 or 2; masked so that the compiler knows, and the
    /// stores of a refill's lookups need no bounds checks.
    #[inline(always)]
    fn len(self) -> usize {
        1 + (self.0 >> 16 & 1) as usize
    }
}

/// Lookups a stream from the table of pairs between refills: each takes at most
/// `MAX_MAX_NUM_BITS` bits, of the 56 a refill leaves.
const PAIRS_A_REFILL: usize = 56 / MAX_MAX_NUM_BITS as usize;

/// The refills of a round of the four streams' loop from the table of pairs, which checks the
/// output's room and the sources' bytes left once a round: natively 4, with which x-ray and
/// mixed's tiles decoded 7% and 3–5% faster than with 1 (lab2, 2026-11-05); in wasm 1, V8's
/// code for 2 or more being 10–17% slower.
const PAIRS_ROUND: usize = if cfg!(target_family = "wasm") { 1 } else { 4 };

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

/// The `N` bytes of `out` from `at`, if it has them.
#[inline(always)]
fn room<const N: usize>(out: &mut [u8], at: usize) -> Option<&mut [u8; N]> {
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
        assert!(matches!(
            table.build_table_from_weights(&[1; 256]),
            Err(HuffmanTableError::TooManyWeights { got: 256 })
        ));
        let mut weights = [1; 255];
        weights[0] = 2;
        assert!(table.build_table_from_weights(&weights).is_ok());
    }

    /// The FSE-coded weights decode as the two decoders did on a [`BitReaderReversed`], a bit
    /// at a time for the padding, for random tables and streams: the same weights, or both
    /// refused.
    #[test]
    fn weights_decode_as_a_bit_reader() {
        /// The weights as read before, or `None` where they were refused.
        fn model(stream: &[u8], states: &[WeightState], acc_log: u8) -> Option<Vec<u8>> {
            let mut br = BitReaderReversed::new(stream);
            let mut skipped_bits = 0;
            loop {
                skipped_bits += 1;
                if br.get_bits(1) == 1 || skipped_bits > 8 {
                    break;
                }
            }
            if skipped_bits > 8 {
                return None;
            }
            let state = |at: u64| states[at as usize];
            let mut dec1 = state(br.get_bits(acc_log));
            let mut dec2 = state(br.get_bits(acc_log));
            let mut weights = Vec::new();
            loop {
                weights.push(dec1.symbol);
                dec1 = state(u64::from(dec1.base) + br.get_bits(dec1.num_bits));
                if br.bits_remaining() <= -1 {
                    weights.push(dec2.symbol);
                    break;
                }
                weights.push(dec2.symbol);
                dec2 = state(u64::from(dec2.base) + br.get_bits(dec2.num_bits));
                if br.bits_remaining() <= -1 {
                    weights.push(dec1.symbol);
                    break;
                }
                if weights.len() > 255 {
                    return None;
                }
            }
            Some(weights)
        }

        let mut seed = 0x853c_49e6_748f_ea9bu64;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % u64::from(n)) as u32
        };
        let mut long = 0;
        for _ in 0..20000 {
            // Probabilities of up to 13 weights summing to the table's size, a "less than
            // one" counting one.
            let acc_log = 5 + next(2) as u8;
            let mut left = 1i32 << acc_log;
            let mut probs = Vec::new();
            while left > 0 && probs.len() < 12 {
                let prob = match next(5) {
                    0 => 0,
                    1 => -1,
                    _ => 1 + next(left as u32) as i32,
                };
                left -= prob.max(0) + i32::from(prob == -1);
                probs.push(prob);
            }
            if left > 0 {
                probs.push(left);
            }
            let mut states = [WeightState::default(); WEIGHT_STATES];
            build_states(acc_log, &probs, |state, symbol, num_bits, base| {
                states[state] = WeightState {
                    base: base as u8,
                    num_bits,
                    symbol,
                };
            })
            .unwrap();
            let stream: Vec<u8> = (0..next(127)).map(|_| next(256) as u8).collect();

            let mut weights = [0; WEIGHTS_MOST];
            let got = decode_weights(&stream, &states, acc_log, &mut weights)
                .ok()
                .map(|count| weights[..count].to_vec());
            let expected = model(&stream, &states[..1 << acc_log], acc_log);
            assert_eq!(got, expected, "{acc_log} {probs:?} {stream:?}");
            long += usize::from(expected.is_some_and(|w| w.len() > 100));
        }
        assert!(long > 100, "{long}");
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
                for (symbol, _) in weights[..=last]
                    .iter()
                    .enumerate()
                    .filter(|&(_, &w)| w > 0 && max_bits + 1 - w == bits)
                {
                    let len = 1 << (MAX_MAX_NUM_BITS - bits);
                    expected.extend(core::iter::repeat_n((symbol as u8, bits), len));
                }
            }
            let got: Vec<_> = table
                .decode
                .iter()
                .map(|&[symbol, num_bits]| (symbol, num_bits))
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
