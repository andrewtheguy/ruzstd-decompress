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
    /// It takes no sequence that would take it past `most` bytes.
    #[inline(always)]
    pub fn seq_writer<'a>(
        &'a mut self,
        dict: &'a [u8],
        dict_room: usize,
        most: usize,
    ) -> SeqWriter<'a> {
        let (limit, data_start) = self.sequence_bounds();
        // The room check keeps the sequences' bytes `2 * CHUNK` short of the limit.
        let limit = limit.min(self.tail.saturating_add(most).saturating_add(2 * CHUNK));
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

    /// A writer of sequences to the tail by raw pointers, for the fast loop (see
    /// [`RawWriter`]), whose matches may reach into `dict`, the dictionary's content, for
    /// `dict_room` bytes more of the ring. It takes no sequence that would take it past `most`
    /// bytes. Nothing else may touch the ring until [`Self::finish_raw`] takes it back.
    #[cfg(not(target_family = "wasm"))]
    #[inline(always)]
    pub fn raw_writer(&mut self, dict: &[u8], dict_room: usize, most: usize) -> RawWriter {
        let (limit, data_start) = self.sequence_bounds();
        // The room check keeps the sequences' bytes `WILD` short of the limit.
        let limit = limit.min(self.tail.saturating_add(most).saturating_add(WILD));
        let tail = self.tail;
        assert!(data_start <= tail && tail <= limit && limit <= self.buf.len());
        let base = self.buf.as_mut_ptr();
        let base_addr = base as usize;
        // Where the data wraps, a match reaching back past the ring's start reads from the
        // data's first part, from the head to the end of the ring, as libzstd reads its
        // window's older segment: the writer takes it as a dictionary, with no window to keep
        // to, the data being all within it. That part is past `limit`, the head, so the
        // sequences' bytes never overlap it. A match into the frame's own dictionary, past all
        // of the data, is then left to the slow path.
        let (dict_end, dict_len, dict_until) = if tail >= self.head {
            (
                dict.as_ptr_range().end,
                dict.len(),
                base_addr.saturating_add(tail.saturating_add(dict_room)),
            )
        } else {
            assert!(self.head <= self.cap && self.cap <= self.buf.len());
            // SAFETY: `cap <= buf.len()`, within the allocation.
            let end = unsafe { base.add(self.cap) };
            (end.cast_const(), self.cap - self.head, usize::MAX)
        };
        RawWriter {
            base,
            // SAFETY: `data_start <= tail <= limit <= buf.len()`, within the allocation.
            op: unsafe { base.add(tail) },
            wild_end: (base_addr + limit).saturating_sub(WILD),
            // SAFETY: as for the tail.
            data_start: unsafe { base.add(data_start) },
            dict_end,
            dict_len,
            dict_until,
        }
    }

    /// The tail moved on to where `writer`, from [`Self::raw_writer`], left it; the bytes
    /// written.
    #[cfg(not(target_family = "wasm"))]
    #[inline(always)]
    pub fn finish_raw(&mut self, writer: RawWriter) -> usize {
        let tail = writer.op as usize - writer.base as usize;
        debug_assert!(writer.base == self.buf.as_mut_ptr() && tail <= self.buf.len());
        let written = tail - self.tail;
        self.tail = tail;
        written
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
            &mut self.buf[..limit],
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
    /// `literals` must hold the `ll` literals and then at least `CHUNK` bytes more (see
    /// [`copy_short`]).
    #[inline(always)]
    pub fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        let tail = self.tail;
        // Rounded up, each piece takes less than one chunk more. The room left keeps the tail
        // short of `cap`, or of the head, so it needs no wrapping and the sentinel stays free.
        let Some((before, room)) = self.buf.split_at_mut_checked(tail) else {
            return false;
        };
        if ll + ml + 2 * CHUNK > room.len() {
            return false;
        }
        let dst = tail + ll;
        // The data before the match, its literals included.
        let data = dst - self.data_start;
        if offset <= data {
            if offset < CHUNK {
                return false;
            }
            if ll <= CHUNK && ml <= CHUNK && offset >= ll + CHUNK {
                copy_short(before, room, literals, ll, offset - ll);
            } else {
                copy_literals(self.buf, tail, literals, ll);
                copy_match(self.buf, dst, offset, ml);
            }
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
/// buffer up to where the free space from the tail ends, where it is simple: the free space
/// has room for both rounded up to whole chunks, and the match's source is at least [`CHUNK`]
/// back, in the data that runs on from `data_start` to where it is written without wrapping.
/// Returns the tail after them, or `None`, having changed nothing.
///
/// `buf` ends within what is initialised: at all of the ring that is initialised when the data
/// does not wrap, else at the head, the tail being in the part from the start of the ring.
/// `data_start` is then the head, else the start of the ring. `limit` is `buf`'s length.
///
/// `buf` is a slice, not the ring's `Vec`, whose pointer and length LLVM read from memory for
/// each chunk, as the chunks stored before might have changed them: 0.6% more instructions on
/// silesia in V8. `limit` is passed on its own: checked against `buf.len()` instead, the loop
/// ran most of them again.
///
/// `literals` must hold the `ll` literals and then at least `CHUNK - 1` bytes more (see
/// [`copy_literals`]). The copies are those of [`copy_literals`] and [`copy_match`], written
/// out here with the match's chunks by `copy_within`: V8's code for the loop was 2–4% slower
/// on silesia with the two functions in it.
#[cfg(target_family = "wasm")]
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn copy_sequence(
    buf: &mut [u8],
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
            buf[tail + at..tail + at + CHUNK].copy_from_slice(&literals[at..at + CHUNK]);
            at += CHUNK;
        }
    } else {
        buf[tail..tail + ll].copy_from_slice(&literals[..ll]);
    }
    let dst = tail + ll;
    let src = dst - offset;
    if ml <= SHORT_COPY || offset < ml {
        let mut at = 0;
        while at < ml {
            buf.copy_within(src + at..src + at + CHUNK, dst + at);
            at += CHUNK;
        }
    } else {
        buf.copy_within(src..src + ml, dst);
    }
    Some(dst + ml)
}

/// A sequence of at most a chunk of literals and a chunk of match: a chunk of each, without
/// loops. `room` is the free space from the tail, with room for both and a chunk more;
/// `before` is what is before the tail, the match's source `back` bytes from its end and at
/// least a chunk back.
#[inline(always)]
fn copy_short(before: &[u8], room: &mut [u8], literals: &[u8], ll: usize, back: usize) {
    room[..CHUNK].copy_from_slice(&literals[..CHUNK]);
    let src = &before[before.len() - back..];
    room[ll..ll + CHUNK].copy_from_slice(&src[..CHUNK]);
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

/// How far past a sequence's last byte [`RawWriter`]'s copies may write, and past a block's
/// literals they may read: a wild copy rounds up to whole chunks of 32 bytes after its first
/// 16, as libzstd's `ZSTD_wildcopy`.
pub(crate) const WILD: usize = 32;

/// Sequences written to a ring's tail by raw pointers, as libzstd writes them: the literals
/// and the match each copied in whole chunks past their end, into free space, with no checks
/// but the one for room before the sequence. Made by [`RingBuffer::raw_writer`], which bounds
/// it within the ring's initialised free space from the tail, and handed back to
/// [`RingBuffer::finish_raw`]; dropped without that, the ring is left as it was, what it wrote
/// being in the free space.
///
/// `wild_end` is [`WILD`] bytes before the address where the writes must stop, `data_start` where the data before the
/// tail starts, and `dict_end` the end of the dictionary's content, which matches may reach
/// into while their literals end by the address `dict_until`.
#[cfg(not(target_family = "wasm"))]
pub struct RawWriter {
    base: *mut u8,
    op: *mut u8,
    wild_end: usize,
    data_start: *const u8,
    dict_end: *const u8,
    dict_len: usize,
    dict_until: usize,
}

#[cfg(not(target_family = "wasm"))]
impl RawWriter {
    /// A sequence's `ll` literals from `lit`, then its match of `ml` bytes from `offset` back,
    /// where it is simple: the free space has room for both and [`WILD`] bytes more, and the
    /// match's source is in the data, or past it in the dictionary, then from the data's
    /// start, as `DecodeBuffer::repeat_from_dict` takes it. Returns `false`, having changed
    /// nothing, otherwise; a match reaching past the dictionary's start included, which the
    /// slow path reports.
    ///
    /// # Safety
    ///
    /// `lit` must be valid for reads of `ll + WILD` bytes, outside the ring, and `offset` at
    /// least 1. The ring this writer came from ([`RingBuffer::raw_writer`]) and the dictionary
    /// given to it must not have been used, moved, grown or freed since, but through this
    /// writer: it writes and reads through pointers into them.
    #[inline(always)]
    pub unsafe fn push_sequence(
        &mut self,
        lit: *const u8,
        ll: usize,
        offset: usize,
        ml: usize,
    ) -> bool {
        debug_assert!(offset >= 1);
        let op = self.op;
        // `ll` and `ml` are below 2^18.
        if op as usize + ll + ml > self.wild_end {
            return false;
        }
        // SAFETY: within the room just checked.
        let dst = unsafe { op.add(ll) };
        // The data before the match, its literals included.
        let data = dst as usize - self.data_start as usize;
        if offset > data {
            // SAFETY: as for this function, with the room checked and `offset` past `data`.
            return unsafe { self.push_from_dict(lit, ll, offset, ml, data) };
        }
        // SAFETY: the room checked covers each copy rounded up and past by `WILD`; the match's
        // source is in the data, `offset` back from where it is written.
        unsafe {
            wild_copy(op, lit, ll);
            copy_match_raw(dst, dst.sub(offset), offset, ml);
            self.op = dst.add(ml);
        }
        true
    }

    /// [`Self::push_sequence`] for a match whose source starts `offset - data` bytes before
    /// the data, in the dictionary: the frame's, or where the data wraps, the ring's part from
    /// the head to its end (see [`RingBuffer::raw_writer`]). Returns `false`, having changed
    /// nothing, where the dictionary does not reach that far or the frame is past its window.
    ///
    /// # Safety
    ///
    /// As [`Self::push_sequence`]: `lit` valid for reads of `ll + WILD` bytes, outside the ring,
    /// and the ring and the dictionary untouched since the writer was made. Besides, the room
    /// for the sequence must have been checked, `op + ll + ml` at most `wild_end`, so that the
    /// copies may write up to `WILD` bytes past it; and `data` must be the bytes from
    /// `data_start` to `op + ll`, with `offset` more than `data`.
    #[inline(always)]
    unsafe fn push_from_dict(
        &mut self,
        lit: *const u8,
        ll: usize,
        offset: usize,
        ml: usize,
        data: usize,
    ) -> bool {
        let op = self.op;
        let from_dict = offset - data;
        // SAFETY: as in `push_sequence`, which checked the room.
        let dst = unsafe { op.add(ll) };
        // None where the frame has none, or where the data wraps, or once the frame is past its
        // window.
        if from_dict > self.dict_len || dst as usize > self.dict_until {
            return false;
        }
        let in_dict = from_dict.min(ml);
        // SAFETY: the dictionary holds `from_dict` bytes before its end; the ring's room was
        // checked for `ll + ml + WILD` bytes. The rest of the match, from the data's start, is
        // `offset` back from where it is written.
        unsafe {
            wild_copy(op, lit, ll);
            let src = self.dict_end.sub(from_dict);
            if in_dict <= WILD && from_dict >= WILD {
                copy16(dst, src);
                copy16(dst.add(16), src.add(16));
            } else {
                core::ptr::copy_nonoverlapping(src, dst, in_dict);
            }
            let rest = ml - in_dict;
            if rest > 0 {
                copy_match_raw(dst.add(in_dict), self.data_start, offset, rest);
            }
            self.op = dst.add(ml);
        }
        true
    }
}

/// 16 bytes from `src` to `dst`.
///
/// # Safety
///
/// Both valid for 16 bytes, not overlapping.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
unsafe fn copy16(dst: *mut u8, src: *const u8) {
    // SAFETY: the caller's.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, 16) }
}

/// 8 bytes from `src` to `dst`.
///
/// # Safety
///
/// Both valid for 8 bytes, not overlapping.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
unsafe fn copy8(dst: *mut u8, src: *const u8) {
    // SAFETY: the caller's.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, 8) }
}

/// `len` bytes from `src` to `dst`, in a chunk of 16 and then chunks of 32, which may read and
/// write up to [`WILD`] - 1 bytes past `len`.
///
/// # Safety
///
/// `src` valid for reads and `dst` for writes of `len + WILD` bytes; `src` at least 16 bytes
/// before `dst` or in another allocation, so that each chunk reads only bytes before it.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
unsafe fn wild_copy(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: the caller's; every chunk ends before `len` rounded up past by `WILD`.
    unsafe {
        copy16(dst, src);
        if len > 16 {
            let end = dst.add(len);
            let (mut d, mut s) = (dst.add(16), src.add(16));
            loop {
                copy16(d, s);
                copy16(d.add(16), s.add(16));
                d = d.add(32);
                s = s.add(32);
                if d >= end {
                    break;
                }
            }
        }
    }
}

/// A match of `len` bytes from `src`, `offset` bytes before `dst`, to `dst`, as a copy of a
/// byte at a time would: in wild chunks of 16 where the offset is at least that, else 8 bytes
/// spread from the first `offset`, as libzstd's `ZSTD_overlapCopy8`, then chunks of 8.
///
/// # Safety
///
/// `offset` from 1, `src == dst - offset`; `dst` valid for writes of `len + WILD` bytes and
/// `src` for reads to there.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
unsafe fn copy_match_raw(dst: *mut u8, src: *const u8, offset: usize, len: usize) {
    if offset >= 16 {
        // SAFETY: the caller's.
        unsafe { wild_copy(dst, src, len) };
        return;
    }
    // SAFETY: the caller's; each copy reads only bytes before it, already written.
    unsafe { copy_overlapping(dst, src, offset, len) }
}

/// [`copy_match_raw`] for an offset below 16, out of the fast loop's code.
///
/// # Safety
///
/// As [`copy_match_raw`].
#[cfg(not(target_family = "wasm"))]
#[inline(never)]
unsafe fn copy_overlapping(dst: *mut u8, src: *const u8, offset: usize, len: usize) {
    /// After 4 bytes copied one at a time, how far on the next 4 are read from, so that they
    /// are `offset` back (or a multiple of it) and wholly before where they go.
    const DEC32: [usize; 8] = [0, 1, 2, 1, 4, 4, 4, 4];
    /// Then how far back to step, from those 4 on, for a source a multiple of `offset` at
    /// least 8 back.
    const DEC64: [isize; 8] = [8, 8, 8, 7, 8, 9, 10, 11];
    debug_assert!((1..16).contains(&offset));
    // SAFETY: the caller's. The source is always a multiple of `offset` back from where it
    // is written, at least 8 after the first 8 bytes, and never before `src`.
    unsafe {
        let end = dst.add(len);
        let (mut d, mut s) = (dst, src);
        if offset < 8 {
            *d = *s;
            *d.add(1) = *s.add(1);
            *d.add(2) = *s.add(2);
            *d.add(3) = *s.add(3);
            s = s.add(DEC32[offset]);
            core::ptr::copy_nonoverlapping(s, d.add(4), 4);
            s = s.offset(8 - DEC64[offset]);
        } else {
            copy8(d, s);
            s = s.add(8);
        }
        d = d.add(8);
        while d < end {
            copy8(d, s);
            d = d.add(8);
            s = s.add(8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RingBuffer;
    #[cfg(not(target_family = "wasm"))]
    use super::WILD;

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
                    let mut writer = rb.seq_writer(&[], 0, usize::MAX);
                    for _ in 0..1 + next(4) {
                        let ll = next(40);
                        let padded: alloc::vec::Vec<u8> =
                            (0..ll + super::CHUNK).map(|_| next(256) as u8).collect();
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

    /// Sequences through the raw writer, as the native fast loop writes them, checked against a
    /// byte at a time: from rings kept small, so that their data wraps and matches reach back
    /// across the wrap, with offsets below 16 and past the data, and dictionaries now and then.
    /// Few steps under Miri, which checks the writer's pointers.
    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn raw_writer_matches_a_vec() {
        let mut seed = 0x51af_d7ed_558c_cd1du64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % below as u64) as usize
        };
        let steps = if cfg!(miri) { 3_000 } else { 100_000 };
        let dict: alloc::vec::Vec<u8> = (0..300).map(|_| next(256) as u8).collect();
        let mut rb = RingBuffer::new();
        let mut model: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        let (mut taken, mut across, mut short, mut from_dict) = (0, 0, 0, 0);
        for step in 0..steps {
            match next(4) {
                0 | 1 if !model.is_empty() => {
                    let wrapped = rb.tail < rb.head;
                    // A dictionary only while the data does not wrap, as `DecodeBuffer` gives
                    // it, with room for any match.
                    let use_dict = !wrapped && next(4) == 0;
                    let d: &[u8] = if use_dict { &dict } else { &[] };
                    let len_before = model.len();
                    let mut writer = rb.raw_writer(d, usize::MAX, usize::MAX);
                    for _ in 0..1 + next(4) {
                        let ll = next(40);
                        let padded: alloc::vec::Vec<u8> =
                            (0..ll + WILD).map(|_| next(256) as u8).collect();
                        let offset = 1 + next(model.len() + ll + d.len() + 8);
                        let ml = 3 + next(60);
                        // SAFETY: `padded` holds `ll + WILD` bytes; the offset is at least 1;
                        // nothing touches `rb` or `dict` until the writer is finished.
                        let done = unsafe { writer.push_sequence(padded.as_ptr(), ll, offset, ml) };
                        if done {
                            taken += 1;
                            model.extend_from_slice(&padded[..ll]);
                            if offset < 16 {
                                short += 1;
                            }
                            if offset > model.len() {
                                from_dict += 1;
                            } else if wrapped && offset > rb.tail + (model.len() - len_before) {
                                across += 1;
                            }
                            for _ in 0..ml {
                                let at = d.len() + model.len() - offset;
                                let byte = if at < d.len() {
                                    d[at]
                                } else {
                                    model[at - d.len()]
                                };
                                model.push(byte);
                            }
                        }
                    }
                    if next(8) == 0 {
                        // Dropped unfinished: the ring is as it was.
                        let _ = writer;
                        model.truncate(len_before);
                    } else {
                        assert_eq!(rb.finish_raw(writer), model.len() - len_before);
                    }
                }
                2 => {
                    let data: alloc::vec::Vec<u8> =
                        (0..next(40)).map(|_| next(256) as u8).collect();
                    rb.extend(&data);
                    model.extend_from_slice(&data);
                }
                _ if !model.is_empty() => {
                    let n = next(model.len() + 1);
                    rb.drop_first_n(n);
                    model.drain(..n);
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
        let least = steps / 100;
        assert!(
            taken > 10 * least && across > least && short > least && from_dict > least,
            "{taken} taken: {across} across the wrap, {short} short, {from_dict} from the dictionary"
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
            let mut writer = rb.seq_writer(&dict, room, usize::MAX);
            for _ in 0..1 + next(4) {
                let ll = next(40);
                let padded: alloc::vec::Vec<u8> =
                    (0..ll + super::CHUNK).map(|_| next(256) as u8).collect();
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
