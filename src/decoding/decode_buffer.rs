use crate::io::{Error, Read, Write};
use alloc::vec::Vec;
#[cfg(feature = "hash")]
use core::hash::Hasher;

use super::ringbuffer::RingBuffer;
#[cfg(not(target_family = "wasm"))]
use super::ringbuffer::SeqWriter;
use crate::decoding::errors::DecodeBufferError;

/// The bytes a block's literals are followed by for the buffer's fast copy of a sequence, which
/// reads whole chunks of them.
pub const LITERALS_PADDING: usize = 16;
const _: () = assert!(LITERALS_PADDING >= super::ringbuffer::CHUNK);

/// The frame's decoded bytes: those not yet collected, and before them as many of the
/// collected ones as the window needs, which blocks to come may copy from.
pub struct DecodeBuffer {
    buffer: RingBuffer,
    /// How many of the buffer's first bytes have been collected: kept only for the window.
    collected: usize,
    /// A dictionary's content, kept from frame to frame so that frames using the same
    /// dictionary do not copy it in again (see [`DecoderScratch::init_from_dict`]); the frame
    /// copies from it only if `dict_active`.
    ///
    /// [`DecoderScratch::init_from_dict`]: super::scratch::DecoderScratch::init_from_dict
    pub dict_content: Vec<u8>,
    /// Whether the frame decoded uses `dict_content`: set by its dictionary, cleared by a reset.
    pub dict_active: bool,

    pub window_size: usize,
    total_output_counter: u64,
    /// The hash of the bytes dropped from the buffer, when the frame has a checksum to check
    /// them against: none is made for one that has not, as libzstd makes none. The bytes still
    /// in the buffer are hashed when the checksum is asked for (see [`DecodeBuffer::checksum`]),
    /// so that collecting them early costs nothing more.
    #[cfg(feature = "hash")]
    hash: Option<twox_hash::XxHash64>,
}

/// Sequences written to a [`DecodeBuffer`] in its simple case, the ring's state kept in
/// locals over the run (see [`SeqWriter`]). [`SequenceWriter::finish`] writes it back; a
/// writer dropped without it leaves the buffer as it was.
#[cfg(not(target_family = "wasm"))]
pub struct SequenceWriter<'a> {
    ring: SeqWriter<'a>,
    total_output_counter: &'a mut u64,
}

#[cfg(not(target_family = "wasm"))]
impl SequenceWriter<'_> {
    /// A sequence's `ll` literals, from the start of `literals`, then its match, if the buffer
    /// can take them in its simple case (see [`SeqWriter::push_sequence`]): `literals` must
    /// have [`LITERALS_PADDING`] bytes after the `ll`. Returns `false`, having changed nothing,
    /// otherwise.
    #[inline(always)]
    pub fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        self.ring.push_sequence(literals, ll, offset, ml)
    }

    /// Write the run's sequences to the buffer; the bytes they took.
    #[inline(always)]
    pub fn finish(self) -> usize {
        let written = self.ring.finish();
        *self.total_output_counter += written as u64;
        written
    }
}

/// Reads the bytes not yet collected, as many as fit.
impl Read for DecodeBuffer {
    fn read(&mut self, target: &mut [u8]) -> Result<usize, Error> {
        let mut written = 0;
        self.collect_with(target.len(), |buf| {
            target[written..][..buf.len()].copy_from_slice(buf);
            written += buf.len();
            (buf.len(), Ok(()))
        })
    }
}

impl DecodeBuffer {
    /// A buffer for a frame of `window_size`, hashing what it decodes if `checksum`, the frame
    /// having one.
    pub fn new(window_size: usize, checksum: bool) -> DecodeBuffer {
        let mut buffer = DecodeBuffer {
            buffer: RingBuffer::new(),
            collected: 0,
            dict_content: Vec::new(),
            dict_active: false,
            window_size,
            total_output_counter: 0,
            #[cfg(feature = "hash")]
            hash: None,
        };
        buffer.reset(window_size, checksum);
        buffer
    }

    /// Empty the buffer for a new frame, as [`Self::new`] makes it.
    pub fn reset(&mut self, window_size: usize, checksum: bool) {
        self.window_size = window_size;
        self.buffer.clear();
        self.collected = 0;
        self.dict_active = false;
        self.total_output_counter = 0;
        #[cfg(feature = "hash")]
        {
            self.hash = checksum.then(|| twox_hash::XxHash64::with_seed(0));
        }
        #[cfg(not(feature = "hash"))]
        let _ = checksum;
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Make room for `amount` more bytes.
    pub fn reserve(&mut self, amount: usize) {
        self.buffer.reserve(amount);
    }

    /// Undo the bytes added since the buffer was `len` long, which have not been
    /// collected (and so not hashed) yet.
    pub fn truncate(&mut self, len: usize) {
        debug_assert!(len >= self.collected);
        let dropped = self.buffer.len() - len;
        self.buffer.truncate(len);
        self.total_output_counter -= dropped as u64;
    }

    pub fn extend_and_fill(&mut self, fill_with: u8, fill_length: usize) {
        self.buffer.extend_and_fill(fill_with, fill_length);
    }

    pub fn extend_from_reader<R: Read>(
        &mut self,
        read: R,
        fill_length: usize,
    ) -> Result<(), crate::io::Error> {
        self.buffer.extend_from_reader(read, fill_length)
    }

    /// A writer of sequences to the buffer, for a run of them (see [`SequenceWriter`]), of at
    /// most `most` bytes.
    #[cfg(not(target_family = "wasm"))]
    #[inline(always)]
    pub fn sequence_writer(&mut self, most: usize) -> SequenceWriter<'_> {
        let (dict, dict_room): (&[u8], usize) = match self.dict_room() {
            Some(room) => (&self.dict_content, room),
            None => (&[], 0),
        };
        SequenceWriter {
            ring: self.buffer.seq_writer(dict, dict_room, most),
            total_output_counter: &mut self.total_output_counter,
        }
    }

    /// How many bytes more of the ring matches may reach past into the dictionary: while the
    /// frame is within its window (see `repeat_from_dict`), as many as are left of it. `None`
    /// where the frame uses no dictionary or is past its window.
    #[inline(always)]
    fn dict_room(&self) -> Option<usize> {
        let window_left = (self.window_size as u64).checked_sub(self.total_output_counter)?;
        self.dict_active
            .then(|| usize::try_from(window_left).unwrap_or(usize::MAX))
    }

    /// A sequence's `ll` literals, from the start of `literals`, then its match, if the buffer
    /// can take them in its simple case (see [`RingBuffer::push_sequence`]): `literals` must
    /// have [`LITERALS_PADDING`] bytes after the `ll`. Returns `false`, having changed nothing,
    /// otherwise.
    #[cfg(target_family = "wasm")]
    #[inline(always)]
    pub fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        let done = self.buffer.push_sequence(literals, ll, offset, ml);
        if done {
            self.total_output_counter += (ll + ml) as u64;
        }
        done
    }

    /// [`Self::push_sequence`] by the ring's writer, which natively takes every sequence, for
    /// one the ring's fast copy has not taken: a match into the dictionary, as most of a small
    /// frame's that uses one are, in its simple case. Returns `false`, having changed nothing,
    /// otherwise. From the slow path, out of the fast loop: V8's code for the loop was 2–4%
    /// slower on silesia with the check in it.
    #[cfg(target_family = "wasm")]
    #[inline(always)]
    pub fn push_from_dict(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        if !self.dict_active {
            return false;
        }
        let Some(room) = self.dict_room() else {
            return false;
        };
        let mut writer = self.buffer.seq_writer(&self.dict_content, room, usize::MAX);
        let done = writer.push_sequence(literals, ll, offset, ml);
        writer.finish();
        if done {
            self.total_output_counter += (ll + ml) as u64;
        }
        done
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend(data);
        self.total_output_counter += data.len() as u64;
    }

    #[inline(always)]
    pub fn repeat(&mut self, offset: usize, match_length: usize) -> Result<(), DecodeBufferError> {
        if offset > self.buffer.len() {
            self.repeat_from_dict(offset, match_length)
        } else {
            let buf_len = self.buffer.len();
            let start_idx = buf_len - offset;
            let end_idx = start_idx + match_length;

            self.buffer.reserve(match_length);
            if end_idx > buf_len {
                // We need to copy in chunks.
                self.repeat_in_chunks(offset, match_length, start_idx);
            } else {
                self.buffer.extend_from_within(start_idx, match_length);
            }

            self.total_output_counter += match_length as u64;
            Ok(())
        }
    }

    fn repeat_in_chunks(&mut self, offset: usize, match_length: usize, start_idx: usize) {
        // The match overlaps what it writes: it repeats the last `offset` bytes.
        // Once one copy of them is written, `start_idx..` holds two whole periods,
        // so each copy can take twice what the one before took, from the same
        // start, until the match is written.
        let mut available = offset;
        let mut left = match_length;
        while left > 0 {
            let chunk = usize::min(available, left);
            self.buffer.extend_from_within(start_idx, chunk);
            left -= chunk;
            available += chunk;
        }
    }

    #[cold]
    fn repeat_from_dict(
        &mut self,
        offset: usize,
        match_length: usize,
    ) -> Result<(), DecodeBufferError> {
        if self.total_output_counter <= self.window_size as u64 {
            // at least part of that repeat is from the dictionary content
            let bytes_from_dict = offset - self.buffer.len();
            let dict: &[u8] = if self.dict_active {
                &self.dict_content
            } else {
                &[]
            };

            if bytes_from_dict > dict.len() {
                return Err(DecodeBufferError::NotEnoughBytesInDictionary {
                    got: dict.len(),
                    need: bytes_from_dict,
                });
            }

            if bytes_from_dict < match_length {
                self.buffer.extend(&dict[dict.len() - bytes_from_dict..]);

                self.total_output_counter += bytes_from_dict as u64;
                return self.repeat(self.buffer.len(), match_length - bytes_from_dict);
            } else {
                let low = dict.len() - bytes_from_dict;
                self.buffer.extend(&dict[low..low + match_length]);
                self.total_output_counter += match_length as u64;
            }
            Ok(())
        } else {
            Err(DecodeBufferError::OffsetTooBig {
                offset,
                buf_len: self.buffer.len(),
            })
        }
    }

    /// The XXH64 of every byte decoded since the reset, collected or not; `None` unless the
    /// buffer was made for a frame with a checksum.
    #[cfg(feature = "hash")]
    pub fn checksum(&self) -> Option<u64> {
        let mut hash = self.hash.clone()?;
        let (a, b) = self.buffer.as_slices();
        hash.write(a);
        hash.write(b);
        Some(hash.finish())
    }

    /// How many bytes are decoded and not yet collected.
    pub fn can_collect(&self) -> usize {
        self.buffer.len() - self.collected
    }

    /// Every byte not yet collected.
    pub fn collect(&mut self) -> Vec<u8> {
        let mut vec = Vec::with_capacity(self.can_collect());
        let collected = self.collect_with(usize::MAX, |buf| {
            vec.extend_from_slice(buf);
            (buf.len(), Ok(()))
        });
        debug_assert!(collected.is_ok());
        vec
    }

    /// Write every byte not yet collected to `sink`, and return how many it took.
    pub fn collect_to_writer(&mut self, mut sink: impl Write) -> Result<usize, Error> {
        self.collect_with(usize::MAX, |buf| write_all_bytes(&mut sink, buf))
    }

    /// Hand up to `amount` of the bytes not yet collected, in order, to `write_bytes`, which
    /// takes as many of those it is given as it can and says how many and whether it failed;
    /// then drop the collected bytes that are past the window. Returns how many were taken.
    fn collect_with(
        &mut self,
        amount: usize,
        mut write_bytes: impl FnMut(&[u8]) -> (usize, Result<(), Error>),
    ) -> Result<usize, Error> {
        let (a, b) = self.buffer.as_slices();
        let in_a = self.collected.min(a.len());
        let parts = [&a[in_a..], &b[self.collected - in_a..]];
        let mut written = 0;
        let mut result = Ok(());
        for part in parts {
            let n = part.len().min(amount - written);
            if n == 0 {
                continue;
            }
            let (taken, part_result) = write_bytes(&part[..n]);
            written += taken;
            if part_result.is_err() || taken < n {
                result = part_result;
                break;
            }
        }
        self.collected += written;

        // The window is the newest `window_size` bytes, collected or not.
        let past_window = self.buffer.len().saturating_sub(self.window_size);
        let dropped = past_window.min(self.collected);
        if dropped > 0 {
            #[cfg(feature = "hash")]
            if let Some(hash) = &mut self.hash {
                let (a, b) = self.buffer.as_slices();
                let in_a = dropped.min(a.len());
                hash.write(&a[..in_a]);
                hash.write(&b[..dropped - in_a]);
            }
            self.buffer.drop_first_n(dropped);
            self.collected -= dropped;
        }
        result.map(|()| written)
    }
}

/// Like Write::write_all but returns partial write length even on error
fn write_all_bytes(mut sink: impl Write, buf: &[u8]) -> (usize, Result<(), Error>) {
    let mut written = 0;
    while written < buf.len() {
        match sink.write(&buf[written..]) {
            Ok(0) => return (written, Ok(())),
            Ok(w) => written += w,
            Err(e) => return (written, Err(e)),
        }
    }
    (written, Ok(()))
}

#[cfg(test)]
mod tests {
    use super::DecodeBuffer;
    use crate::io::{Error, ErrorKind, Write};

    extern crate std;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn short_writer() {
        struct ShortWriter {
            buf: Vec<u8>,
            write_len: usize,
        }

        impl Write for ShortWriter {
            fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, Error> {
                if buf.len() > self.write_len {
                    self.buf.extend_from_slice(&buf[..self.write_len]);
                    Ok(self.write_len)
                } else {
                    self.buf.extend_from_slice(buf);
                    Ok(buf.len())
                }
            }

            fn flush(&mut self) -> std::result::Result<(), Error> {
                Ok(())
            }
        }

        let mut short_writer = ShortWriter {
            buf: vec![],
            write_len: 10,
        };

        // Each collect writes every new byte, and the buffer keeps the window of 100.
        let mut decode_buf = DecodeBuffer::new(100, true);
        decode_buf.push(b"0123456789");
        decode_buf.repeat(10, 90).unwrap();
        let repeats = 1000;
        for _ in 0..repeats {
            decode_buf.repeat(10, 50).unwrap();
            decode_buf.collect_to_writer(&mut short_writer).unwrap();
            assert_eq!(decode_buf.can_collect(), 0);
            assert_eq!(decode_buf.len(), 100);
        }

        assert_eq!(short_writer.buf.len(), 100 + repeats * 50);
        assert!(short_writer.buf.chunks(10).all(|ten| ten == b"0123456789"));
    }

    #[test]
    fn wouldblock_writer() {
        struct WouldblockWriter {
            buf: Vec<u8>,
            last_blocked: usize,
            block_every: usize,
        }

        impl Write for WouldblockWriter {
            fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, Error> {
                if self.last_blocked < self.block_every {
                    self.buf.extend_from_slice(buf);
                    self.last_blocked += 1;
                    Ok(buf.len())
                } else {
                    self.last_blocked = 0;
                    Err(Error::from(ErrorKind::WouldBlock))
                }
            }

            fn flush(&mut self) -> std::result::Result<(), Error> {
                Ok(())
            }
        }

        let mut short_writer = WouldblockWriter {
            buf: vec![],
            last_blocked: 0,
            block_every: 5,
        };

        // A write that fails leaves what was not written to be collected again.
        let mut decode_buf = DecodeBuffer::new(100, true);
        decode_buf.push(b"0123456789");
        decode_buf.repeat(10, 90).unwrap();
        let repeats = 1000;
        for _ in 0..repeats {
            decode_buf.repeat(10, 50).unwrap();
            while decode_buf.can_collect() > 0 {
                match decode_buf.collect_to_writer(&mut short_writer) {
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(e) => panic!("Unexpected error {:?}", e),
                }
            }
            assert_eq!(decode_buf.len(), 100);
        }

        assert_eq!(short_writer.buf.len(), 100 + repeats * 50);
        assert!(short_writer.buf.chunks(10).all(|ten| ten == b"0123456789"));
    }

    /// Bytes pushed and repeated, with collects and reads of every size between, against a
    /// `Vec` of the output: each byte is collected once, in order, as soon as it is added,
    /// and the window is kept, as far back as a match may reach, however much was collected.
    #[test]
    fn collects_as_decoded_and_keeps_the_window() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..300 {
            let window = 1 + next(300);
            let mut decode_buf = DecodeBuffer::new(window, true);
            let mut model: Vec<u8> = Vec::new();
            let mut out: Vec<u8> = Vec::new();
            for _ in 0..200 {
                let op = next(4);
                match op {
                    0 => {
                        let data: Vec<u8> = (0..next(100)).map(|_| next(256) as u8).collect();
                        decode_buf.push(&data);
                        model.extend_from_slice(&data);
                    }
                    1 if !model.is_empty() => {
                        let offset = 1 + next(model.len().min(window));
                        let length = next(150);
                        decode_buf.repeat(offset, length).unwrap();
                        for _ in 0..length {
                            model.push(model[model.len() - offset]);
                        }
                    }
                    2 => out.extend_from_slice(&decode_buf.collect()),
                    _ => {
                        let mut target = vec![0; next(80)];
                        let n = crate::io::Read::read(&mut decode_buf, &mut target).unwrap();
                        out.extend_from_slice(&target[..n]);
                    }
                }
                assert_eq!(out, model[..out.len()], "round {round}");
                assert_eq!(decode_buf.can_collect(), model.len() - out.len());
                // The window and what is not yet collected; once collected, no more.
                let kept = window.max(decode_buf.can_collect()).min(model.len());
                assert!(decode_buf.len() >= kept, "round {round}");
                if op >= 2 {
                    assert_eq!(decode_buf.len(), kept, "round {round}");
                }
                #[cfg(feature = "hash")]
                {
                    use core::hash::Hasher;
                    let mut hash = twox_hash::XxHash64::with_seed(0);
                    hash.write(&model);
                    assert_eq!(decode_buf.checksum(), Some(hash.finish()), "round {round}");
                }
            }
            out.extend_from_slice(&decode_buf.collect());
            assert_eq!(out, model, "round {round}");
        }
    }

    /// A repeat whose match overlaps what it writes, at every offset and
    /// length up to a few chunks, checked against copying a byte at a time.
    #[test]
    fn repeat_matches_bytewise() {
        let prefix: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(37) ^ 0x5a).collect();
        for offset in 1..=64 {
            for match_length in 1..=200 {
                let mut decode_buf = DecodeBuffer::new(1 << 20, true);
                decode_buf.push(&prefix);
                decode_buf.repeat(offset, match_length).unwrap();
                let mut expected = prefix.clone();
                for _ in 0..match_length {
                    expected.push(expected[expected.len() - offset]);
                }
                assert_eq!(
                    decode_buf.collect(),
                    expected,
                    "offset {offset}, length {match_length}"
                );
            }
        }
    }

    /// Every byte a match takes from the dictionary counts towards the window, so that once the
    /// frame is past it, no match reaches into the dictionary: through the slow path's repeat,
    /// and natively through the sequence writer's copy too, in turn.
    #[test]
    fn dict_matches_count_towards_the_window() {
        use super::super::errors::DecodeBufferError;

        let dict: Vec<u8> = (0..100u8).collect();
        let window = 40;
        let mut buf = DecodeBuffer::new(window, false);
        buf.dict_content = dict.clone();
        buf.dict_active = true;
        buf.reserve(4096);
        buf.push(b"ab");
        let mut expected = b"ab".to_vec();
        // Each match: 8 bytes, 50 back into the dictionary, past the data.
        let mut matches = 0;
        loop {
            let offset = buf.len() + 50;
            let fits = buf.total_output_counter <= window as u64;
            #[cfg(not(target_family = "wasm"))]
            if matches % 2 == 1 {
                let mut writer = buf.sequence_writer(usize::MAX);
                let done = writer.push_sequence(&[0; super::LITERALS_PADDING], 0, offset, 8);
                writer.finish();
                if !fits {
                    assert!(!done, "past the window, the writer took a dictionary match");
                    break;
                }
                assert!(done);
                expected.extend_from_slice(&dict[50..58]);
                matches += 1;
                assert_eq!(buf.total_output_counter, expected.len() as u64);
                continue;
            }
            let result = buf.repeat(offset, 8);
            if !fits {
                assert!(matches!(
                    result,
                    Err(DecodeBufferError::OffsetTooBig { .. })
                ));
                break;
            }
            result.unwrap();
            expected.extend_from_slice(&dict[50..58]);
            matches += 1;
            assert_eq!(buf.total_output_counter, expected.len() as u64);
        }
        assert!(matches >= 5, "{matches} matches");
        let (a, b) = buf.buffer.as_slices();
        assert_eq!([a, b].concat(), expected);
    }
}
