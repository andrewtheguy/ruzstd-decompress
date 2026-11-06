use crate::io::Read;
use alloc::vec::Vec;

/// A ring buffer of bytes over a `Vec`, without unsafe code.
///
/// The ring is `cap` bytes. `head..tail` (or `head..cap` then `..tail` once it wraps) is
/// the data; the rest is free, one byte of it always kept as the sentinel that tells full
/// from empty. `buf` holds the ring's first `buf.len()` bytes, initialised a step ahead of
/// the furthest the tail has written, so that a new or grown buffer is not zeroed whole
/// before it is used: a frame of a few kilobytes then costs a few kilobytes. Within
/// `buf.len()`, a copy may read past the data it wants and write past the end of the data,
/// into the free space, where that is faster.
pub struct RingBuffer {
    buf: Vec<u8>,
    cap: usize,
    head: usize,
    tail: usize,
}

/// The size a short copy is rounded up to, when there is room for it.
pub(crate) const CHUNK: usize = 16;

/// The longest piece of a sequence [`copy_literals`] and [`copy_match`] copy in whole chunks.
const SHORT_COPY: usize = 2 * CHUNK;

/// How far past what a write needs the ring is initialised, so that it is done in a few
/// large fills rather than one for each write.
const INIT_AHEAD: usize = 16 * 1024;

impl RingBuffer {
    pub fn new() -> Self {
        RingBuffer {
            buf: Vec::new(),
            cap: 0,
            head: 0,
            tail: 0,
        }
    }

    #[cfg(test)]
    fn cap(&self) -> usize {
        self.cap
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
        let cap = self.cap;
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

        // Only the data is copied; the rest is initialised as the tail reaches it.
        let mut new_buf = Vec::with_capacity(new_cap);
        let (s1, s2) = self.as_slices();
        new_buf.extend_from_slice(s1);
        new_buf.extend_from_slice(s2);
        self.tail = new_buf.len();
        self.buf = new_buf;
        self.cap = new_cap;
        self.head = 0;
    }

    /// Initialise the ring up to `end`, at most `cap`, before the tail writes there.
    #[inline(always)]
    fn init_to(&mut self, end: usize) {
        if end > self.buf.len() {
            self.init_ahead(end);
        }
    }

    #[inline(never)]
    #[cold]
    fn init_ahead(&mut self, end: usize) {
        debug_assert!(end <= self.cap);
        // Within `buf`'s capacity, which is `cap`: no reallocation.
        self.buf.resize(usize::min(end + INIT_AHEAD, self.cap), 0);
    }

    #[allow(dead_code)]
    pub fn push_back(&mut self, byte: u8) {
        self.reserve(1);
        self.init_to(self.tail + 1);
        self.buf[self.tail] = byte;
        self.advance_tail(1);
    }

    /// Fetch the byte stored at the selected index from the buffer, returning it, or
    /// `None` if the index is out of bounds.
    #[allow(dead_code)]
    pub fn get(&self, idx: usize) -> Option<u8> {
        if idx < self.len() {
            Some(self.buf[(self.head + idx) % self.cap])
        } else {
            None
        }
    }

    /// `tail` moved on by `len`, wrapping at the end of the buffer.
    #[inline(always)]
    fn advance_tail(&mut self, len: usize) {
        let tail = self.tail + len;
        self.tail = if tail >= self.cap {
            tail - self.cap
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
        let tail = self.tail;
        // Where a write from the tail may run to in one piece, initialised: the reserve leaves
        // room up to the head if that is ahead.
        let limit = if tail >= self.head {
            self.buf.len()
        } else {
            self.head
        };
        if tail + len <= limit {
            self.buf[tail..tail + len].copy_from_slice(data);
            self.advance_tail(len);
        } else {
            self.extend_wrapping(data);
        }
    }

    /// [`Self::extend`] where the data wraps at the end of the ring, or reaches past what is
    /// initialised.
    #[cold]
    fn extend_wrapping(&mut self, data: &[u8]) {
        let len = data.len();
        let (f1, _) = self.free_slice_lengths_ordered();
        let in_f1 = usize::min(len, f1);
        let tail = self.tail;
        self.init_to(tail + in_f1);
        self.buf[tail..tail + in_f1].copy_from_slice(&data[..in_f1]);
        if in_f1 < len {
            self.buf[..len - in_f1].copy_from_slice(&data[in_f1..]);
        }
        self.advance_tail(len);
    }

    /// Advance head past `amount` elements, effectively removing
    /// them from the buffer.
    /// Drop the newest bytes, keeping the first `len`.
    pub fn truncate(&mut self, len: usize) {
        debug_assert!(len <= self.len());
        if len < self.len() {
            self.tail = (self.head + len) % self.cap;
        }
    }

    pub fn drop_first_n(&mut self, amount: usize) {
        debug_assert!(amount <= self.len());
        let amount = usize::min(amount, self.len());
        self.head = (self.head + amount) % self.cap.max(1);
    }

    /// Return the size of the two contiguous occupied sections of memory used
    /// by the buffer.
    fn data_slice_lengths(&self) -> (usize, usize) {
        if self.tail >= self.head {
            (self.tail - self.head, 0)
        } else {
            (self.cap - self.head, self.tail)
        }
    }

    /// Return references to each part of the ring buffer.
    pub fn as_slices(&self) -> (&[u8], &[u8]) {
        if self.tail >= self.head {
            (&self.buf[self.head..self.tail], &[])
        } else {
            (&self.buf[self.head..self.cap], &self.buf[..self.tail])
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
            (self.cap - self.tail, self.head)
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

        let cap = self.cap;
        let mut src = self.head + start;
        if src >= cap {
            src -= cap;
        }
        let dst = self.tail;
        // How far a write from the tail may run without reaching the data, or past what is
        // initialised.
        let room = if dst >= self.head {
            self.buf.len() - dst
        } else {
            self.head - dst
        };

        let rounded = len.next_multiple_of(CHUNK);
        if src + rounded <= cap && rounded <= room {
            // Contiguous on both sides, with room to round the copy up: whole
            // chunks, the bytes past `len` read from anywhere in the buffer and
            // written into free space. A chunk never reads what an earlier one
            // wrote within `len`, as the source ends at or before `dst`. It reads
            // within what is initialised: a source before `dst` ends before
            // `dst + rounded`, and one after it is in the wrapped part, which the
            // tail has passed.
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

    /// Where the free space from the tail ends, within what is initialised (all of the ring
    /// once the data wraps), and where the data before the tail starts: the bounds of a
    /// sequence's fast copy.
    #[inline(always)]
    fn sequence_bounds(&self) -> (usize, usize) {
        if self.tail >= self.head {
            (self.buf.len(), self.head)
        } else {
            (self.head, 0)
        }
    }

    /// A writer of sequences to the tail, for a run of them (see [`SeqWriter`]), whose matches
    /// may reach into `dict`, the dictionary's content, for `dict_room` bytes more of the ring.
    #[inline(always)]
    pub fn seq_writer<'a>(&'a mut self, dict: &'a [u8], dict_room: usize) -> SeqWriter<'a> {
        let (limit, data_start) = self.sequence_bounds();
        // A match reaches into the dictionary only past all of the data; the writer takes
        // that only where the data does not wrap, so that it is all before the tail.
        let tail = self.tail;
        let dict = if tail >= self.head { dict } else { &[] };
        SeqWriter {
            buf: &mut self.buf[..limit],
            tail,
            tail_slot: &mut self.tail,
            data_start,
            dict,
            dict_until: tail.saturating_add(dict_room),
        }
    }

    /// A sequence's literals and then its match, by [`copy_sequence`], the ring's state read
    /// from it as it is needed (see [`SequenceExecutor`]). Returns `false`, having changed
    /// nothing, where that does not take it.
    ///
    /// [`SequenceExecutor`]: super::sequence_execution::SequenceExecutor
    #[cfg(target_family = "wasm")]
    #[inline(always)]
    pub fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        let (limit, data_start) = self.sequence_bounds();
        match copy_sequence(
            &mut self.buf,
            limit,
            data_start,
            self.tail,
            literals,
            ll,
            offset,
            ml,
        ) {
            Some(tail) => {
                self.tail = tail;
                true
            }
            None => false,
        }
    }

    /// `len` bytes from `src` to the tail, either side wrapping at the end of
    /// the buffer. The source is data and the destination free, so they do not
    /// overlap.
    #[cold]
    fn copy_wrapping(&mut self, mut src: usize, len: usize) {
        let cap = self.cap;
        let mut dst = self.tail;
        // Up to the end of the ring if it wraps, and what it wraps to was data before.
        self.init_to(usize::min(dst + len, cap));
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
        self.init_to(tail + fill1);
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
        self.init_to(tail + fill1);
        read.read_exact(&mut self.buf[tail..tail + fill1])?;
        if fill1 < fill_length {
            read.read_exact(&mut self.buf[..fill_length - fill1])?;
        }
        self.advance_tail(fill_length);
        Ok(())
    }
}

/// Sequences written to a ring's tail, its buffer borrowed as a slice and the tail kept in
/// a local, so that neither goes through memory for each sequence. [`SeqWriter::finish`]
/// writes the tail back; a writer dropped without it leaves the ring as it was, what it wrote
/// being in the free space.
///
/// `buf` is the ring up to where the free space from the tail ends, and `data_start` where
/// the data before the tail starts (see [`RingBuffer::sequence_bounds`]). `dict` is the
/// dictionary's content, which matches may reach into while their literals end by
/// `dict_until`: empty if none, or where the data wraps.
///
/// Natively, the fast loop executes every sequence by one; in WebAssembly, the slow path
/// takes the matches into the dictionary by one, which the ring's fast copy does not take
/// (see `DecodeBuffer::push_from_dict`).
pub struct SeqWriter<'a> {
    buf: &'a mut [u8],
    tail_slot: &'a mut usize,
    tail: usize,
    data_start: usize,
    dict: &'a [u8],
    dict_until: usize,
}

impl SeqWriter<'_> {
    /// A sequence's literals and then its match, in one go, where it is simple: the free
    /// space has room for both rounded up to whole chunks, and the match's source is either at
    /// least [`CHUNK`] back in the data, or past the data in the dictionary, then from the
    /// data's start, as `DecodeBuffer::repeat_from_dict` takes it. Returns `false`, having
    /// changed nothing, otherwise; a match reaching past the dictionary's start included, which
    /// the slow path reports.
    ///
    /// `literals` must hold the `ll` literals and then at least `CHUNK - 1` bytes more (see
    /// [`copy_literals`]).
    #[inline(always)]
    pub fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        let tail = self.tail;
        // Rounded up, each piece takes less than one chunk more. The room left keeps the tail
        // short of `cap`, or of the head, so it needs no wrapping and the sentinel stays free.
        if tail + ll + ml + 2 * CHUNK > self.buf.len() {
            return false;
        }
        let dst = tail + ll;
        // The data before the match, its literals included.
        let data = dst - self.data_start;
        if offset <= data {
            if offset < CHUNK {
                return false;
            }
            copy_literals(self.buf, tail, literals, ll);
            copy_match(self.buf, dst, offset, ml);
        } else {
            // Past the data, into the dictionary: none where the frame has none, or where the
            // data wraps, or once the frame is past its window.
            let from_dict = offset - data;
            if from_dict > self.dict.len() || dst > self.dict_until {
                return false;
            }
            let in_dict = from_dict.min(ml);
            // The rest from the data's start, `offset` back: in one copy, which must not
            // overlap.
            let rest = ml - in_dict;
            if rest > offset {
                return false;
            }
            copy_literals(self.buf, tail, literals, ll);
            copy_from_dict(self.buf, dst, self.dict, from_dict, in_dict);
            if rest > 0 {
                self.buf
                    .copy_within(self.data_start..self.data_start + rest, dst + in_dict);
            }
        }
        self.tail = dst + ml;
        true
    }

    /// The bytes written, the ring's tail moved on past them.
    #[inline(always)]
    pub fn finish(self) -> usize {
        let written = self.tail - *self.tail_slot;
        *self.tail_slot = self.tail;
        written
    }
}

/// A sequence's literals and then its match, in one go, from `tail` in `buf`, the ring's
/// buffer, where it is simple: the free space from the tail, up to `limit`, has room for both
/// rounded up to whole chunks, and the match's source is at least [`CHUNK`] back, in the data
/// that runs on from `data_start` to where it is written without wrapping. Returns the tail
/// after them, or `None`, having changed nothing.
///
/// `limit` is where the free space from the tail ends, within what is initialised: all of the
/// ring that is initialised when the data does not wrap, else the head, the tail being in the
/// part from the start of the ring. `data_start` is then the head, else the start of the
/// ring. `buf` is a slice or the ring's `Vec`, taken as a slice at each use: for the `Vec`,
/// its pointer and length are read from it each time (see [`SequenceExecutor`]).
///
/// `literals` must hold the `ll` literals and then at least `CHUNK - 1` bytes more (see
/// [`copy_literals`]). The copies are those of [`copy_literals`] and [`copy_match`], written
/// out here with the match's chunks by `copy_within`: V8's code for the loop was 2–4% slower
/// on silesia with the two functions in it.
///
/// [`SequenceExecutor`]: super::sequence_execution::SequenceExecutor
#[cfg(target_family = "wasm")]
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn copy_sequence<B: AsMut<[u8]> + ?Sized>(
    buf: &mut B,
    limit: usize,
    data_start: usize,
    tail: usize,
    literals: &[u8],
    ll: usize,
    offset: usize,
    ml: usize,
) -> Option<usize> {
    // Rounded up, each piece takes less than one chunk more. The room left keeps the tail
    // short of `cap`, or of the head, so it needs no wrapping and the sentinel stays free.
    if tail + ll + ml + 2 * CHUNK > limit {
        return None;
    }
    if offset > tail - data_start + ll || offset < CHUNK {
        return None;
    }
    // Short pieces in whole chunks; long ones in one copy each, which is faster than chunks
    // once there are a few, where the match does not overlap itself.
    if ll <= SHORT_COPY {
        let mut at = 0;
        while at < ll {
            buf.as_mut()[tail + at..tail + at + CHUNK].copy_from_slice(&literals[at..at + CHUNK]);
            at += CHUNK;
        }
    } else {
        buf.as_mut()[tail..tail + ll].copy_from_slice(&literals[..ll]);
    }
    let dst = tail + ll;
    let src = dst - offset;
    if ml <= SHORT_COPY || offset < ml {
        let mut at = 0;
        while at < ml {
            buf.as_mut()
                .copy_within(src + at..src + at + CHUNK, dst + at);
            at += CHUNK;
        }
    } else {
        buf.as_mut().copy_within(src..src + ml, dst);
    }
    Some(dst + ml)
}

/// The `ll` literals at the start of `literals` to `tail` in `buf`: short ones in whole
/// chunks, long ones in one copy, which is faster than chunks once there are a few.
/// `literals` must hold at least `CHUNK - 1` bytes after the `ll`, which the rounded copy
/// reads, and `buf` room for the literals rounded up. The rounded copy writes past them,
/// into free space, which the match then overwrites from where the literals end.
#[inline(always)]
fn copy_literals<B: AsMut<[u8]> + ?Sized>(buf: &mut B, tail: usize, literals: &[u8], ll: usize) {
    if ll <= SHORT_COPY {
        let mut at = 0;
        while at < ll {
            buf.as_mut()[tail + at..tail + at + CHUNK].copy_from_slice(&literals[at..at + CHUNK]);
            at += CHUNK;
        }
    } else {
        buf.as_mut()[tail..tail + ll].copy_from_slice(&literals[..ll]);
    }
}

/// The match from `offset` back to `dst` in `buf`: in whole chunks where it is short or
/// overlaps itself, else in one copy. `offset` must be at least [`CHUNK`], and `buf` have room
/// for the match rounded up: each chunk then reads only what is before it, the earlier chunks'
/// bytes included, as a copy of a byte at a time would.
#[inline(always)]
fn copy_match<B: AsMut<[u8]> + ?Sized>(buf: &mut B, dst: usize, offset: usize, ml: usize) {
    let src = dst - offset;
    if ml <= SHORT_COPY || offset < ml {
        // Each chunk's source wholly before it, the offset being at least a chunk: copied
        // across a split, not by `copy_within`, which LLVM may leave out of line.
        let mut at = 0;
        while at < ml {
            let (before, from) = buf.as_mut().split_at_mut(dst + at);
            from[..CHUNK].copy_from_slice(&before[src + at..src + at + CHUNK]);
            at += CHUNK;
        }
    } else {
        buf.as_mut().copy_within(src..src + ml, dst);
    }
}

/// The `in_dict` bytes a match takes of `dict`, the dictionary's content, from `from_dict`
/// bytes before its end, to `dst` in `buf`: at most twice [`SHORT_COPY`] in one or two
/// copies of that size where the dictionary has them, which write past the match into free
/// space as the chunks of [`copy_match`] do; else in one copy. `buf` must have room for the
/// match and [`SHORT_COPY`] bytes more.
#[inline(always)]
fn copy_from_dict(buf: &mut [u8], dst: usize, dict: &[u8], from_dict: usize, in_dict: usize) {
    let low = dict.len() - from_dict;
    if in_dict <= 2 * SHORT_COPY && from_dict >= 2 * SHORT_COPY {
        buf[dst..dst + SHORT_COPY].copy_from_slice(&dict[low..low + SHORT_COPY]);
        if in_dict > SHORT_COPY {
            buf[dst + SHORT_COPY..dst + 2 * SHORT_COPY]
                .copy_from_slice(&dict[low + SHORT_COPY..low + 2 * SHORT_COPY]);
        }
    } else {
        buf[dst..dst + in_dict].copy_from_slice(&dict[low..low + in_dict]);
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

    /// A reserve allocates, but initialises only as far as the tail writes, a step ahead.
    #[test]
    fn initialised_as_written() {
        let mut rb = RingBuffer::new();
        rb.reserve(1 << 20);
        assert!(rb.cap() > 1 << 20);
        assert_eq!(rb.buf.len(), 0);
        rb.extend(b"0123456789");
        rb.extend_from_within(0, 10);
        rb.extend_and_fill(7, 5);
        assert_eq!(rb.buf.len(), 10 + super::INIT_AHEAD);
        assert_eq!(
            rb.as_slices().0,
            b"01234567890123456789\x07\x07\x07\x07\x07"
        );
        // Grown, only the data is copied.
        rb.reserve(1 << 21);
        assert_eq!(rb.buf.len(), 25);
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
        let mut sequences = 0;
        for step in 0..200_000 {
            match next(6) {
                // A few sequences in a run: when the fast copy takes one, the literals then the
                // match, byte by byte; when it does not, nothing. Natively, they go through one
                // writer, which is now and then dropped unfinished, leaving the ring as it was.
                3 if !model.is_empty() => {
                    let len_before = model.len();
                    #[cfg(not(target_family = "wasm"))]
                    let mut writer = rb.seq_writer(&[], 0);
                    for _ in 0..1 + next(4) {
                        let ll = next(40);
                        let padded: alloc::vec::Vec<u8> = (0..ll + super::CHUNK - 1)
                            .map(|_| next(256) as u8)
                            .collect();
                        let offset = 1 + next(model.len() + ll + 8);
                        let ml = 3 + next(60);
                        #[cfg(not(target_family = "wasm"))]
                        let done = writer.push_sequence(&padded, ll, offset, ml);
                        #[cfg(target_family = "wasm")]
                        let done = rb.push_sequence(&padded, ll, offset, ml);
                        if done {
                            sequences += 1;
                            model.extend_from_slice(&padded[..ll]);
                            for _ in 0..ml {
                                model.push(model[model.len() - offset]);
                            }
                        }
                    }
                    #[cfg(not(target_family = "wasm"))]
                    if next(8) == 0 {
                        model.truncate(len_before);
                    } else {
                        assert_eq!(writer.finish(), model.len() - len_before);
                    }
                    #[cfg(target_family = "wasm")]
                    let _ = len_before;
                }
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
                2 if !model.is_empty() => {
                    let n = next(model.len() + 1);
                    rb.truncate(n);
                    model.truncate(n);
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
        assert!(
            sequences > 5_000,
            "only {sequences} sequences took the fast copy"
        );
    }

    /// Sequences whose matches reach into a dictionary, through the native writer, checked
    /// against a byte at a time from the dictionary's content then the data: from rings whose
    /// head has moved on, with the window's room for the dictionary now and then short.
    #[test]
    fn dict_matches_a_vec() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % below as u64) as usize
        };
        let dict: alloc::vec::Vec<u8> = (0..300).map(|_| next(256) as u8).collect();
        let (mut from_dict, mut refused) = (0, 0);
        for round in 0..3000 {
            // Room reserved first, so that the extend initialises the ring ahead of the data.
            let mut rb = RingBuffer::new();
            rb.reserve(4096);
            let mut model: alloc::vec::Vec<u8> = (0..next(50)).map(|_| next(256) as u8).collect();
            rb.extend(&model);
            if next(2) == 0 {
                let n = next(model.len() + 1);
                rb.drop_first_n(n);
                model.drain(..n);
            }
            let room = if next(4) == 0 { next(100) } else { usize::MAX };
            let start = model.len();
            let mut writer = rb.seq_writer(&dict, room);
            for _ in 0..1 + next(4) {
                let ll = next(40);
                let padded: alloc::vec::Vec<u8> = (0..ll + super::CHUNK - 1)
                    .map(|_| next(256) as u8)
                    .collect();
                let offset = 1 + next(model.len() + ll + dict.len() + 10);
                let ml = 3 + next(60);
                if !writer.push_sequence(&padded, ll, offset, ml) {
                    refused += 1;
                    continue;
                }
                model.extend_from_slice(&padded[..ll]);
                if offset > model.len() {
                    from_dict += 1;
                    // Within the window's room, the literals included.
                    assert!(model.len() - start <= room, "round {round}: past the room");
                }
                for _ in 0..ml {
                    let at = dict.len() + model.len() - offset;
                    let byte = if at < dict.len() {
                        dict[at]
                    } else {
                        model[at - dict.len()]
                    };
                    model.push(byte);
                }
            }
            assert_eq!(writer.finish(), model.len() - start);
            let (a, b) = rb.as_slices();
            assert_eq!([a, b].concat(), model, "round {round}");
        }
        assert!(
            from_dict > 1_000 && refused > 1_000,
            "{from_dict} from the dictionary, {refused} refused"
        );
    }
}
