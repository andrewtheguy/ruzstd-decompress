use crate::io::Read;
use alloc::vec;
use alloc::vec::Vec;

/// A ring buffer of bytes over an initialised `Vec`, without unsafe code.
///
/// `head..tail` (or `head..` then `..tail` once it wraps) is the data; the rest is
/// free, one byte of it always kept as the sentinel that tells full from empty.
/// The whole of `buf` is initialised, so a copy may read past the data it wants
/// and write past the end of the data, into the free space, where that is faster.
pub struct RingBuffer {
    buf: Vec<u8>,
    head: usize,
    tail: usize,
}

/// The size a short copy is rounded up to, when there is room for it.
const CHUNK: usize = 16;

impl RingBuffer {
    pub fn new() -> Self {
        RingBuffer {
            buf: Vec::new(),
            head: 0,
            tail: 0,
        }
    }

    #[cfg(test)]
    fn cap(&self) -> usize {
        self.buf.len()
    }

    /// Return the number of bytes in the buffer.
    pub fn len(&self) -> usize {
        let (x, y) = self.data_slice_lengths();
        x + y
    }

    /// Return the amount of available space (in bytes) of the buffer.
    pub fn free(&self) -> usize {
        let (x, y) = self.free_slice_lengths();
        // We always keep one free slot as a sentinel
        (x + y).saturating_sub(1)
    }

    /// Empty the buffer and reset the head and tail.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
    }

    /// Ensure that there's space for `amount` elements in the buffer.
    pub fn reserve(&mut self, amount: usize) {
        let free = self.free();
        if free >= amount {
            return;
        }
        self.reserve_amortized(amount - free);
    }

    #[inline(never)]
    #[cold]
    fn reserve_amortized(&mut self, amount: usize) {
        let cap = self.buf.len();
        // We always need at least 1 unused element as the sentinel, so add 1 to the required capacity
        let required = cap + amount;

        // We still grow exponentionally but only when the demand exceeds 2^k + 2 blocks
        // This is where the demand commonly settles when we have reached the window size (commonly a 2^k)
        // And the user code is regularly collecting chunks of decoded data
        const SLACK: usize = 2 * crate::common::MAX_BLOCK_SIZE as usize;
        let new_cap = if required <= SLACK {
            usize::max(cap.next_power_of_two(), required.next_power_of_two()) + 1
        } else {
            (required - SLACK).next_power_of_two() + SLACK + 1
        };

        vprintln!("Reallocate ringbuffer to fit: {} KB", new_cap / 1024);

        let mut new_buf = vec![0u8; new_cap];
        let (s1, s2) = self.as_slices();
        let (l1, l2) = (s1.len(), s2.len());
        new_buf[..l1].copy_from_slice(s1);
        new_buf[l1..l1 + l2].copy_from_slice(s2);
        self.buf = new_buf;
        self.head = 0;
        self.tail = l1 + l2;
    }

    #[allow(dead_code)]
    pub fn push_back(&mut self, byte: u8) {
        self.reserve(1);
        self.buf[self.tail] = byte;
        self.tail = (self.tail + 1) % self.buf.len();
    }

    /// Fetch the byte stored at the selected index from the buffer, returning it, or
    /// `None` if the index is out of bounds.
    #[allow(dead_code)]
    pub fn get(&self, idx: usize) -> Option<u8> {
        if idx < self.len() {
            Some(self.buf[(self.head + idx) % self.buf.len()])
        } else {
            None
        }
    }

    /// `tail` moved on by `len`, wrapping at the end of the buffer.
    #[inline(always)]
    fn advance_tail(&mut self, len: usize) {
        let tail = self.tail + len;
        self.tail = if tail >= self.buf.len() {
            tail - self.buf.len()
        } else {
            tail
        };
    }

    /// Append the provided data to the end of `self`.
    pub fn extend(&mut self, data: &[u8]) {
        let len = data.len();
        if len == 0 {
            return;
        }
        self.reserve(len);
        let (f1, _) = self.free_slice_lengths_ordered();
        let in_f1 = usize::min(len, f1);
        let tail = self.tail;
        self.buf[tail..tail + in_f1].copy_from_slice(&data[..in_f1]);
        if in_f1 < len {
            self.buf[..len - in_f1].copy_from_slice(&data[in_f1..]);
        }
        self.advance_tail(len);
    }

    /// Advance head past `amount` elements, effectively removing
    /// them from the buffer.
    pub fn drop_first_n(&mut self, amount: usize) {
        debug_assert!(amount <= self.len());
        let amount = usize::min(amount, self.len());
        self.head = (self.head + amount) % self.buf.len().max(1);
    }

    /// Return the size of the two contiguous occupied sections of memory used
    /// by the buffer.
    fn data_slice_lengths(&self) -> (usize, usize) {
        if self.tail >= self.head {
            (self.tail - self.head, 0)
        } else {
            (self.buf.len() - self.head, self.tail)
        }
    }

    /// Return references to each part of the ring buffer.
    pub fn as_slices(&self) -> (&[u8], &[u8]) {
        if self.tail >= self.head {
            (&self.buf[self.head..self.tail], &[])
        } else {
            (&self.buf[self.head..], &self.buf[..self.tail])
        }
    }

    /// Returns the size of the two unoccupied sections of memory used by the buffer,
    /// the one before the head and the one after the tail.
    fn free_slice_lengths(&self) -> (usize, usize) {
        let (after_tail, to_head) = self.free_slice_lengths_ordered();
        (to_head, after_tail)
    }

    /// The free space in the order it is written: from the tail on, then from
    /// the start of the buffer.
    #[inline(always)]
    fn free_slice_lengths_ordered(&self) -> (usize, usize) {
        if self.tail < self.head {
            (self.head - self.tail, 0)
        } else {
            (self.buf.len() - self.tail, self.head)
        }
    }

    /// Copies elements from the provided range to the end of the buffer.
    ///
    /// `start + len` must be at most `self.len()`; a wrong range is a wrong
    /// output, never undefined behaviour.
    #[inline(always)]
    pub fn extend_from_within(&mut self, start: usize, len: usize) {
        debug_assert!(start + len <= self.len());
        self.reserve(len);

        let cap = self.buf.len();
        let mut src = self.head + start;
        if src >= cap {
            src -= cap;
        }
        let dst = self.tail;
        // How far a write from the tail may run without reaching the data.
        let room = if dst >= self.head {
            cap - dst
        } else {
            self.head - dst
        };

        let rounded = len.next_multiple_of(CHUNK);
        if src + rounded <= cap && rounded <= room {
            // Contiguous on both sides, with room to round the copy up: whole
            // chunks, the bytes past `len` read from anywhere in the buffer and
            // written into free space. A chunk never reads what an earlier one
            // wrote within `len`, as the source ends at or before `dst`.
            if rounded == CHUNK {
                self.buf.copy_within(src..src + CHUNK, dst);
            } else {
                let mut at = 0;
                while at < rounded {
                    self.buf.copy_within(src + at..src + at + CHUNK, dst + at);
                    at += CHUNK;
                }
            }
        } else {
            self.copy_wrapping(src, len);
        }
        self.advance_tail(len);
    }

    /// `len` bytes from `src` to the tail, either side wrapping at the end of
    /// the buffer. The source is data and the destination free, so they do not
    /// overlap.
    #[cold]
    fn copy_wrapping(&mut self, mut src: usize, len: usize) {
        let cap = self.buf.len();
        let mut dst = self.tail;
        let mut left = len;
        while left > 0 {
            let n = left.min(cap - src).min(cap - dst);
            self.buf.copy_within(src..src + n, dst);
            src += n;
            if src == cap {
                src = 0;
            }
            dst += n;
            if dst == cap {
                dst = 0;
            }
            left -= n;
        }
    }

    pub fn extend_and_fill(&mut self, fill_with: u8, fill_length: usize) {
        if fill_length == 0 {
            return;
        }
        self.reserve(fill_length);
        let (f1, _) = self.free_slice_lengths_ordered();
        let fill1 = usize::min(f1, fill_length);
        let tail = self.tail;
        self.buf[tail..tail + fill1].fill(fill_with);
        if fill1 < fill_length {
            self.buf[..fill_length - fill1].fill(fill_with);
        }
        self.advance_tail(fill_length);
    }

    pub fn extend_from_reader<R: Read>(
        &mut self,
        mut read: R,
        fill_length: usize,
    ) -> Result<(), crate::io::Error> {
        if fill_length == 0 {
            return Ok(());
        }
        self.reserve(fill_length);
        let (f1, _) = self.free_slice_lengths_ordered();
        let fill1 = usize::min(f1, fill_length);
        let tail = self.tail;
        read.read_exact(&mut self.buf[tail..tail + fill1])?;
        if fill1 < fill_length {
            read.read_exact(&mut self.buf[..fill_length - fill1])?;
        }
        self.advance_tail(fill_length);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RingBuffer;

    #[test]
    fn smoke() {
        let mut rb = RingBuffer::new();

        rb.reserve(15);
        assert_eq!(17, rb.cap());

        rb.extend(b"0123456789");
        assert_eq!(rb.len(), 10);
        assert_eq!(rb.as_slices().0, b"0123456789");
        assert_eq!(rb.as_slices().1, b"");

        rb.drop_first_n(5);
        assert_eq!(rb.len(), 5);
        assert_eq!(rb.as_slices().0, b"56789");
        assert_eq!(rb.as_slices().1, b"");

        rb.extend_from_within(2, 3);
        assert_eq!(rb.len(), 8);
        assert_eq!(rb.as_slices().0, b"56789789");
        assert_eq!(rb.as_slices().1, b"");

        rb.extend_from_within(0, 3);
        assert_eq!(rb.len(), 11);
        assert_eq!(rb.as_slices().0, b"56789789567");
        assert_eq!(rb.as_slices().1, b"");

        rb.extend_from_within(0, 2);
        assert_eq!(rb.len(), 13);
        assert_eq!(rb.as_slices().0, b"567897895675");
        assert_eq!(rb.as_slices().1, b"6");

        rb.drop_first_n(11);
        assert_eq!(rb.len(), 2);
        assert_eq!(rb.as_slices().0, b"5");
        assert_eq!(rb.as_slices().1, b"6");

        rb.extend(b"0123456789");
        assert_eq!(rb.len(), 12);
        assert_eq!(rb.as_slices().0, b"5");
        assert_eq!(rb.as_slices().1, b"60123456789");

        rb.drop_first_n(11);
        assert_eq!(rb.len(), 1);
        assert_eq!(rb.as_slices().0, b"9");
        assert_eq!(rb.as_slices().1, b"");

        rb.extend(b"0123456789");
        assert_eq!(rb.len(), 11);
        assert_eq!(rb.as_slices().0, b"9012345");
        assert_eq!(rb.as_slices().1, b"6789");
    }

    #[test]
    fn edge_cases() {
        // Fill exactly, then empty then fill again
        let mut rb = RingBuffer::new();
        rb.reserve(16);
        assert_eq!(17, rb.cap());
        rb.extend(b"0123456789012345");
        assert_eq!(17, rb.cap());
        assert_eq!(16, rb.len());
        assert_eq!(0, rb.free());
        rb.drop_first_n(16);
        assert_eq!(0, rb.len());
        assert_eq!(16, rb.free());
        rb.extend(b"0123456789012345");
        assert_eq!(16, rb.len());
        assert_eq!(0, rb.free());
        assert_eq!(17, rb.cap());
        assert_eq!(1, rb.as_slices().0.len());
        assert_eq!(15, rb.as_slices().1.len());

        rb.clear();

        // data in both slices and then reserve
        rb.extend(b"0123456789012345");
        rb.drop_first_n(8);
        rb.extend(b"67890123");
        assert_eq!(16, rb.len());
        assert_eq!(0, rb.free());
        assert_eq!(17, rb.cap());
        assert_eq!(9, rb.as_slices().0.len());
        assert_eq!(7, rb.as_slices().1.len());
        rb.reserve(1);
        assert_eq!(16, rb.len());
        assert_eq!(16, rb.free());
        assert_eq!(33, rb.cap());
        assert_eq!(16, rb.as_slices().0.len());
        assert_eq!(0, rb.as_slices().1.len());

        rb.clear();

        // fill exactly, then extend from within
        rb.extend(b"0123456789012345");
        rb.extend_from_within(0, 16);
        assert_eq!(32, rb.len());
        assert_eq!(0, rb.free());
        assert_eq!(33, rb.cap());
        assert_eq!(32, rb.as_slices().0.len());
        assert_eq!(0, rb.as_slices().1.len());

        // extend from within cases
        let mut rb = RingBuffer::new();
        rb.reserve(8);
        rb.extend(b"01234567");
        rb.drop_first_n(5);
        rb.extend_from_within(0, 3);
        assert_eq!(4, rb.as_slices().0.len());
        assert_eq!(2, rb.as_slices().1.len());

        rb.drop_first_n(2);
        assert_eq!(2, rb.as_slices().0.len());
        assert_eq!(2, rb.as_slices().1.len());
        rb.extend_from_within(0, 4);
        assert_eq!(2, rb.as_slices().0.len());
        assert_eq!(6, rb.as_slices().1.len());

        rb.drop_first_n(2);
        assert_eq!(6, rb.as_slices().0.len());
        assert_eq!(0, rb.as_slices().1.len());
        rb.drop_first_n(2);
        assert_eq!(4, rb.as_slices().0.len());
        assert_eq!(0, rb.as_slices().1.len());
        rb.extend_from_within(0, 4);
        assert_eq!(7, rb.as_slices().0.len());
        assert_eq!(1, rb.as_slices().1.len());

        let mut rb = RingBuffer::new();
        rb.reserve(8);
        rb.extend(b"11111111");
        rb.drop_first_n(7);
        rb.extend(b"111");
        assert_eq!(2, rb.as_slices().0.len());
        assert_eq!(2, rb.as_slices().1.len());
        rb.extend_from_within(0, 4);
        assert_eq!(b"11", rb.as_slices().0);
        assert_eq!(b"111111", rb.as_slices().1);
    }

    /// Random extends, drops and copies from within, checked against a plain
    /// `Vec` of the same bytes: every wrap of the source and of the
    /// destination, and copies both shorter and longer than a chunk.
    #[test]
    fn matches_a_vec() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % below as u64) as usize
        };
        let mut rb = RingBuffer::new();
        let mut model: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        let steps = if cfg!(miri) { 2_000 } else { 200_000 };
        for step in 0..steps {
            match next(4) {
                0 => {
                    let data: alloc::vec::Vec<u8> =
                        (0..next(40)).map(|_| next(256) as u8).collect();
                    rb.extend(&data);
                    model.extend_from_slice(&data);
                }
                1 if !model.is_empty() => {
                    let n = next(model.len() + 1);
                    rb.drop_first_n(n);
                    model.drain(..n);
                }
                _ if !model.is_empty() => {
                    let start = next(model.len());
                    let len = next(model.len() - start + 1).min(70);
                    rb.extend_from_within(start, len);
                    model.extend_from_within(start..start + len);
                }
                _ => {}
            }
            // Keep it small, so it wraps often.
            if model.len() > 300 {
                let n = model.len() - 100;
                rb.drop_first_n(n);
                model.drain(..n);
            }
            let (a, b) = rb.as_slices();
            assert_eq!([a, b].concat(), model, "step {step}");
        }
    }
}
