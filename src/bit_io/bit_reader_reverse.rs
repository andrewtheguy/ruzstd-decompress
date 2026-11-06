use core::convert::TryInto;

/// The zero bytes before a [`BitReaderReversed::padded`] copy of a source: enough that the
/// index stays at 15 or more, room for two fast refills in a row, until the window is wholly
/// before the source's start, so that a loop of fast refills on the copy reads all of the
/// source.
pub const PADDING: usize = 32;

/// The most bytes of a source a [`BitReaderReversed::padded`] copy takes: those a reader has
/// left when its index falls below 15, too few for two fast refills in a row: at most 14
/// before its window and the window's 8.
pub const PADDED_LEFT: usize = 22;

/// Zstandard encodes some types of data in a way that the data must be read
/// back to front to decode it properly. `BitReaderReversed` provides a
/// convenient interface to do that.
#[derive(Clone)]
pub struct BitReaderReversed<'s> {
    /// The index of the last read byte in the source.
    index: usize,

    /// How many bits have been consumed from `bit_container`.
    bits_consumed: u8,

    /// How many bits have been consumed past the end of the input. Will be zero until all the input
    /// has been read.
    extra_bits: usize,

    /// The source data to read from.
    source: &'s [u8],

    /// The reader doesn't read directly from the source, it reads bits from here, and the container
    /// is "refilled" as it's emptied.
    bit_container: u64,
}

impl<'s> BitReaderReversed<'s> {
    /// How many bits are left to read by the reader.
    pub fn bits_remaining(&self) -> isize {
        self.index as isize * 8 + (64 - self.bits_consumed as isize) - self.extra_bits as isize
    }

    pub fn new(source: &'s [u8]) -> BitReaderReversed<'s> {
        BitReaderReversed {
            index: source.len(),
            bits_consumed: 64,
            source,
            bit_container: 0,
            extra_bits: 0,
        }
    }

    /// We refill the container in full bytes, shifting the still unread portion to the left, and filling the lower bits with new data
    ///
    /// The common case, moving the window down while enough source is left, is inlined; the
    /// end of the source is handled out of line in `refill_slow`.
    #[inline(always)]
    fn refill(&mut self) {
        let bytes_consumed = self.bits_consumed as usize / 8;
        if bytes_consumed != 0 && self.index >= bytes_consumed {
            // We can safely move the window contained in `bit_container` down by `bytes_consumed`
            // If the reader wasn't byte aligned, the byte that was partially read is now in the highest order bits in the `bit_container`
            self.index -= bytes_consumed;
            // Some bits of the `bits_container` might have been consumed already because we read the window byte aligned
            self.bits_consumed &= 7;
            self.bit_container =
                u64::from_le_bytes((&self.source[self.index..][..8]).try_into().unwrap());
        } else if bytes_consumed != 0 {
            self.refill_slow(bytes_consumed);
        }
    }

    /// Whether [`Self::ensure_bits_fast`] can be used: at least 8 bytes of the source are left,
    /// the most a refill moves by, so the next refill cannot reach the source's end.
    #[inline(always)]
    pub fn can_refill_fast(&self) -> bool {
        self.index >= 8
    }

    /// [`Self::ensure_bits`] while [`Self::can_refill_fast`]: the refill never reaches the
    /// source's end, so a loop of these has no call to `refill_slow`, which, taking the reader
    /// by reference, would keep it in memory instead of registers.
    #[inline(always)]
    pub fn ensure_bits_fast(&mut self, n: u8) {
        if self.bits_consumed + n > 64 {
            let bytes_consumed = self.bits_consumed as usize / 8;
            debug_assert!(self.index >= bytes_consumed);
            self.index -= bytes_consumed;
            self.bits_consumed &= 7;
            self.bit_container =
                u64::from_le_bytes((&self.source[self.index..][..8]).try_into().unwrap());
        }
    }

    /// Refill when fewer than `bytes_consumed` bytes of the source are left.
    #[cold]
    #[inline(never)]
    fn refill_slow(&mut self, bytes_consumed: usize) {
        debug_assert!(self.index < bytes_consumed);
        if self.index > 0 {
            // Read the last portion of source into the `bit_container`
            if self.source.len() >= 8 {
                self.bit_container = u64::from_le_bytes((&self.source[..8]).try_into().unwrap());
            } else {
                let mut value = [0; 8];
                value[..self.source.len()].copy_from_slice(self.source);
                self.bit_container = u64::from_le_bytes(value);
            }

            self.bits_consumed -= 8 * self.index as u8;
            self.index = 0;

            self.bit_container <<= self.bits_consumed;
            self.extra_bits += self.bits_consumed as usize;
            self.bits_consumed = 0;
        } else if self.bits_consumed < 64 {
            // Shift out already used bits and fill up with zeroes
            self.bit_container <<= self.bits_consumed;
            self.extra_bits += self.bits_consumed as usize;
            self.bits_consumed = 0;
        } else {
            // All useful bits have already been read and more than 64 bits have been consumed, all we now do is return zeroes
            self.extra_bits += self.bits_consumed as usize;
            self.bits_consumed = 0;
            self.bit_container = 0;
        }

        // Assert that at least `56 = 64 - 8` bits are available to read.
        debug_assert!(self.bits_consumed < 8);
    }

    /// The unread bits of the container, the next one highest, as `peek_bits` reads them, with a
    /// sentinel 1 in place of the container's lowest bit. A caller takes bits off the top by
    /// shifting the window left; the sentinel moves up with them, so its position is then how
    /// many of the container's bits are consumed, which [`Self::consume_to_sentinel`] sets.
    ///
    /// Only after `ensure_bits(57)` (or its `_fast` form), which leaves at most 7 bits consumed,
    /// and for at most 56
    /// bits taken: then the lowest bit ever read is the container's bit 1, and the sentinel,
    /// which ends at most at bit 63, is never shifted out.
    #[inline(always)]
    pub fn sentinel_window(&self) -> u64 {
        debug_assert!(self.bits_consumed <= 7);
        (self.bit_container | 1) << self.bits_consumed
    }

    /// Consume what has been taken off the top of `window`, from [`Self::sentinel_window`].
    #[inline(always)]
    pub fn consume_to_sentinel(&mut self, window: u64) {
        self.bits_consumed = window.trailing_zeros() as u8;
        debug_assert!(self.bits_consumed <= 63);
    }

    /// [`Self::consume_to_sentinel`], then the refill of `ensure_bits_fast(57)` without its
    /// check, then the next [`Self::sentinel_window`]: only while [`Self::can_refill_fast`]. A
    /// refill that has no whole byte to move by reloads the same bytes.
    #[inline(always)]
    pub fn refill_window_fast(&mut self, window: u64) -> u64 {
        let consumed = window.trailing_zeros() as usize;
        debug_assert!(consumed <= 63 && self.index >= 8);
        self.index -= consumed / 8;
        self.bits_consumed = (consumed % 8) as u8;
        self.bit_container =
            u64::from_le_bytes((&self.source[self.index..][..8]).try_into().unwrap());
        (self.bit_container | 1) << self.bits_consumed
    }

    /// The source's start and the index of the container's first byte, for a loop of
    /// [`refill_raw`] while [`Self::can_refill_fast`] holds of the index; the reader is set
    /// from where it leaves them by [`Self::resume_index`].
    #[inline(always)]
    pub fn raw_index(&self) -> (*const u8, usize) {
        (self.source.as_ptr(), self.index)
    }

    /// The reader where a loop of [`refill_raw`] from [`Self::raw_index`] left the index of
    /// its container and its `window`.
    #[inline(always)]
    pub fn resume_index(&mut self, index: usize, window: u64) {
        let bytes = self.source[index..]
            .first_chunk::<8>()
            .expect("a container within the source");
        self.index = index;
        self.bit_container = u64::from_le_bytes(*bytes);
        self.consume_to_sentinel(window);
    }

    /// The reader on a copy of what is left of its source, after [`PADDING`] zero bytes, in
    /// `buf`: it reads on as this one would, the zeros past the source's start included, but
    /// its index stays at 15 or more, room for two fast refills in a row, until its window
    /// starts 17 bytes before the source's start, so that fast refills reach that start and
    /// past it. Its [`Self::bits_remaining`] counts the zero bytes, `PADDING * 8` more than
    /// this one's.
    ///
    /// Only while the container holds the 8 bytes from `index`, as every refill leaves it but
    /// one reaching the source's start, and at most [`PADDED_LEFT`] bytes are left; `None`
    /// otherwise.
    pub fn padded<'b>(
        &self,
        buf: &'b mut [u8; PADDING + PADDED_LEFT],
    ) -> Option<BitReaderReversed<'b>> {
        let left = self.index + 8;
        if self.extra_bits != 0 || left > self.source.len() || left > PADDED_LEFT {
            return None;
        }
        buf[..PADDING].fill(0);
        buf[PADDING..PADDING + left].copy_from_slice(&self.source[..left]);
        let buf: &'b [u8] = buf;
        Some(BitReaderReversed {
            index: self.index + PADDING,
            bits_consumed: self.bits_consumed,
            extra_bits: 0,
            source: &buf[..PADDING + left],
            bit_container: self.bit_container,
        })
    }

    /// Make sure at least `n` bits (at most 57) can be peeked and consumed without refilling.
    #[inline(always)]
    pub fn ensure_bits(&mut self, n: u8) {
        if self.bits_consumed + n > 64 {
            self.refill();
        }
    }

    /// Read `n` number of bits from the source. Will read at most 56 bits.
    /// If there are no more bits to be read from the source zero bits will be returned instead.
    #[inline(always)]
    pub fn get_bits(&mut self, n: u8) -> u64 {
        if self.bits_consumed + n > 64 {
            self.refill();
        }

        let value = self.peek_bits(n);
        self.consume(n);
        value
    }

    /// Get the next `n` bits from the source without consuming them.
    /// Caller is responsible for making sure that `n` many bits have been refilled.
    #[inline(always)]
    pub fn peek_bits(&mut self, n: u8) -> u64 {
        if n == 0 {
            return 0;
        }

        let mask = (1u64 << n) - 1u64;
        let shift_by = 64 - self.bits_consumed - n;
        (self.bit_container >> shift_by) & mask
    }

    /// Get the next `n1` `n2` and `n3` bits from the source without consuming them.
    /// Caller is responsible for making sure that `sum` many bits have been refilled.
    #[inline(always)]
    pub fn peek_bits_triple(&mut self, sum: u8, n1: u8, n2: u8, n3: u8) -> (u64, u64, u64) {
        if sum == 0 {
            return (0, 0, 0);
        }

        // all_three contains bits like this: |XXXX..XXX111122223333|
        // Where XXX are already consumed bytes, 1/2/3 are bits of the respective value
        // Lower bits are to the right
        let all_three = self.bit_container >> (64 - self.bits_consumed - sum);

        let mask1 = (1u64 << n1) - 1u64;
        let shift_by1 = n3 + n2;
        let val1 = (all_three >> shift_by1) & mask1;

        let mask2 = (1u64 << n2) - 1u64;
        let shift_by2 = n3;
        let val2 = (all_three >> shift_by2) & mask2;

        let mask3 = (1u64 << n3) - 1u64;
        let val3 = all_three & mask3;

        (val1, val2, val3)
    }

    /// Consume `n` bits from the source.
    #[inline(always)]
    pub fn consume(&mut self, n: u8) {
        self.bits_consumed += n;
        debug_assert!(self.bits_consumed <= 64);
    }

    /// Same as calling get_bits three times but slightly more performant
    #[inline(always)]
    pub fn get_bits_triple(&mut self, n1: u8, n2: u8, n3: u8) -> (u64, u64, u64) {
        let sum = n1 + n2 + n3;
        if sum <= 56 {
            self.refill();

            let triple = self.peek_bits_triple(sum, n1, n2, n3);
            self.consume(sum);
            return triple;
        }

        (self.get_bits(n1), self.get_bits(n2), self.get_bits(n3))
    }
}

/// [`BitReaderReversed::refill_window_fast`] by a raw pointer to the container's first byte:
/// the window after the bits taken off the top of `window`, and `ip` moved down by the whole
/// bytes taken, at most 7.
///
/// # Safety
///
/// `ip` must be at least 7 bytes after the source's start, and the source hold 8 bytes from it.
#[inline(always)]
pub unsafe fn refill_raw(ip: &mut *const u8, window: u64) -> u64 {
    let consumed = window.trailing_zeros();
    // SAFETY: the caller's; `consumed / 8` is at most 7.
    unsafe {
        *ip = ip.sub(consumed as usize / 8);
        (u64::from_le(core::ptr::read_unaligned(*ip as *const u64)) | 1) << (consumed % 8)
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn it_works() {
        let data = [0b10101010, 0b01010101];
        let mut br = super::BitReaderReversed::new(&data);
        assert_eq!(br.get_bits(1), 0);
        assert_eq!(br.get_bits(1), 1);
        assert_eq!(br.get_bits(1), 0);
        assert_eq!(br.get_bits(4), 0b1010);
        assert_eq!(br.get_bits(4), 0b1101);
        assert_eq!(br.get_bits(4), 0b0101);
        // Last 0 from source, three zeroes filled in
        assert_eq!(br.get_bits(4), 0b0000);
        // All zeroes filled in
        assert_eq!(br.get_bits(4), 0b0000);
        assert_eq!(br.bits_remaining(), -7);
    }

    #[test]
    fn matches_bitwise_model() {
        use alloc::vec::Vec;

        // Reads from the end of the source to the front, most significant bit first,
        // with zeroes once the source is used up.
        fn model_bits(source: &[u8], pos: &mut usize, n: u8) -> u64 {
            let mut value = 0;
            for _ in 0..n {
                let total = source.len() * 8;
                let bit = if *pos < total {
                    let bit_idx = total - 1 - *pos;
                    (source[bit_idx / 8] >> (bit_idx % 8)) & 1
                } else {
                    0
                };
                value = (value << 1) | bit as u64;
                *pos += 1;
            }
            value
        }

        let source: Vec<u8> = (0..40u32).map(|x| (x * 151 + 7) as u8).collect();
        let patterns: [&[u8]; 4] = [&[1, 7, 3], &[11, 9, 8], &[56, 1, 13], &[0, 25, 2, 31]];
        for len in 0..source.len() {
            let source = &source[..len];
            for pattern in patterns {
                let mut br = super::BitReaderReversed::new(source);
                let mut pos = 0;
                for (step, &n) in pattern.iter().cycle().take(60).enumerate() {
                    let expected = model_bits(source, &mut pos, n);
                    assert_eq!(br.get_bits(n), expected, "len {len} step {step}");
                    assert_eq!(br.bits_remaining(), len as isize * 8 - pos as isize);
                }
                // get_bits_triple must agree too
                let mut br = super::BitReaderReversed::new(source);
                let mut pos = 0;
                for (step, &n) in pattern.iter().cycle().take(30).enumerate() {
                    let (n1, n2, n3) = (n % 20, n % 7, n % 13);
                    let expected = (
                        model_bits(source, &mut pos, n1),
                        model_bits(source, &mut pos, n2),
                        model_bits(source, &mut pos, n3),
                    );
                    assert_eq!(
                        br.get_bits_triple(n1, n2, n3),
                        expected,
                        "len {len} step {step}"
                    );
                    assert_eq!(br.bits_remaining(), len as isize * 8 - pos as isize);
                }
            }
        }
    }

    /// A padded copy reads on as the reader would, past the source's start too, whatever is
    /// left: the reader read a step at a time and copied, where it can be, at every step.
    #[test]
    fn padded_reads_on_as_the_reader() {
        use super::{BitReaderReversed, PADDED_LEFT, PADDING};
        let source: alloc::vec::Vec<u8> = (0..60u32).map(|x| (x * 157 + 11) as u8).collect();
        let steps: [u8; 6] = [7, 13, 1, 25, 0, 9];
        let mut copies = 0;
        for len in 0..source.len() {
            let source = &source[..len];
            let mut br = BitReaderReversed::new(source);
            for (at, &n) in steps.iter().cycle().take(40).enumerate() {
                let mut buf = [0xa5; PADDING + PADDED_LEFT];
                if let Some(mut padded) = br.padded(&mut buf) {
                    copies += 1;
                    let mut ahead = br.clone();
                    assert_eq!(
                        padded.bits_remaining(),
                        ahead.bits_remaining() + PADDING as isize * 8
                    );
                    for &m in steps.iter().cycle().skip(at).take(30) {
                        assert_eq!(padded.get_bits(m), ahead.get_bits(m), "len {len} at {at}");
                    }
                }
                br.get_bits(n);
            }
        }
        assert!(copies > 500, "{copies}");
    }
}
