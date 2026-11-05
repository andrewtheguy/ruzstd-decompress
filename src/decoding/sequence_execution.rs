use super::decode_buffer::{DecodeBuffer, LITERALS_PADDING};
use crate::blocks::sequence_section::Sequence;
use crate::decoding::errors::ExecuteSequencesError;

/// Executes sequences one at a time, as they are decoded, so they never have to be stored.
pub struct SequenceExecutor<'a> {
    buffer: &'a mut DecodeBuffer,
    /// The block's literals, then [`LITERALS_PADDING`] bytes, which the buffer's fast copy
    /// reads (see [`DecodeBuffer::push_sequence`]).
    padded_literals: &'a [u8],
    literals_len: usize,
    offset_hist: &'a mut [u32; 3],
    literals_copy_counter: usize,
    old_buffer_size: usize,
    seq_sum: u32,
    max_block_decoded_len: usize,
}

impl<'a> SequenceExecutor<'a> {
    /// `padded_literals` is the block's literals followed by [`LITERALS_PADDING`] bytes.
    pub fn new(
        buffer: &'a mut DecodeBuffer,
        padded_literals: &'a [u8],
        offset_hist: &'a mut [u32; 3],
        max_block_decoded_len: usize,
    ) -> Self {
        let old_buffer_size = buffer.len();
        assert!(padded_literals.len() >= LITERALS_PADDING);
        SequenceExecutor {
            buffer,
            padded_literals,
            literals_len: padded_literals.len() - LITERALS_PADDING,
            offset_hist,
            literals_copy_counter: 0,
            old_buffer_size,
            seq_sum: 0,
            max_block_decoded_len,
        }
    }

    /// Copy the literals of `seq` and then its match to the end of the buffer.
    #[inline(always)]
    pub fn execute(&mut self, seq: Sequence) -> Result<(), ExecuteSequencesError> {
        let (ll, ml) = (seq.ll as usize, seq.ml as usize);
        let start = self.literals_copy_counter;
        let high = start + ll;
        if high > self.literals_len {
            return Err(ExecuteSequencesError::NotEnoughBytesForSequence {
                wanted: high,
                have: self.literals_len,
            });
        }
        let actual_offset = do_offset_history(seq.of, seq.ll, self.offset_hist);
        if actual_offset == 0 {
            return Err(ExecuteSequencesError::ZeroOffset);
        }
        let literals = &self.padded_literals[start..];
        if !self
            .buffer
            .push_sequence(literals, ll, actual_offset as usize, ml)
        {
            self.execute_slow(&literals[..ll], actual_offset as usize, ml)?;
        }
        self.literals_copy_counter = high;

        self.seq_sum += seq.ml;
        self.seq_sum += seq.ll;
        self.check_decoded_len(self.seq_sum)
    }

    /// [`Self::execute`], not inlined: for the few sequences decoded outside the fast loop.
    #[inline(never)]
    pub fn execute_out_of_line(&mut self, seq: Sequence) -> Result<(), ExecuteSequencesError> {
        self.execute(seq)
    }

    /// [`Self::execute`] where the buffer's fast copy does not apply: the data wraps, the match
    /// reaches into the dictionary or overlaps itself closely, or the buffer needs room.
    #[inline(never)]
    fn execute_slow(
        &mut self,
        literals: &[u8],
        offset: usize,
        ml: usize,
    ) -> Result<(), ExecuteSequencesError> {
        if !literals.is_empty() {
            self.buffer.push(literals);
        }
        if ml > 0 {
            self.buffer.repeat(offset, ml)?;
        }
        Ok(())
    }

    /// Copy the literals left over after the last sequence.
    pub fn finish(self) -> Result<(), ExecuteSequencesError> {
        let mut seq_sum = self.seq_sum;
        if self.literals_copy_counter < self.literals_len {
            let rest_literals =
                &self.padded_literals[self.literals_copy_counter..self.literals_len];
            self.buffer.push(rest_literals);
            seq_sum += rest_literals.len() as u32;
            self.check_decoded_len(seq_sum)?;
        }

        let diff = self.buffer.len() - self.old_buffer_size;
        assert!(
            seq_sum as usize == diff,
            "Seq_sum: {} is different from the difference in buffersize: {}",
            seq_sum,
            diff
        );
        Ok(())
    }

    /// Block_Maximum_Size bounds every block; checking after each sequence also keeps
    /// `seq_sum` far from overflowing.
    #[inline(always)]
    fn check_decoded_len(&self, decoded: u32) -> Result<(), ExecuteSequencesError> {
        if decoded as usize > self.max_block_decoded_len {
            return Err(ExecuteSequencesError::TooManyBytesGenerated {
                max: self.max_block_decoded_len,
                decoded: decoded as usize,
            });
        }
        Ok(())
    }
}

/// Update the most recently used offsets to reflect the provided offset value, and return the
/// "actual" offset needed because offsets are not stored in a raw way, some transformations are needed
/// before you get a functional number.
///
/// A value over 3 is a new offset. Otherwise it picks a repeat: the value, one more when the
/// sequence has no literals, chooses the most recent offset, the second, the third, or the
/// most recent minus one; the one chosen moves to the front, the others keeping their order.
/// The pick is an index and the reorder selects, so that a block mixing repeats costs no
/// mispredicted branches.
#[inline(always)]
fn do_offset_history(offset_value: u32, lit_len: u32, scratch: &mut [u32; 3]) -> u32 {
    let [h0, h1, h2] = *scratch;
    if offset_value > 3 {
        let actual = offset_value - 3;
        *scratch = [actual, h0, h1];
        return actual;
    }
    // 0..=3 for the four repeats. A malformed dictionary can seed the most recent offset with
    // 0; the minus one saturates, so that it resolves to 0, which the caller rejects as
    // ZeroOffset, instead of underflowing. See #115.
    let pick = (offset_value as usize + usize::from(lit_len == 0) - 1) & 3;
    let actual = [h0, h1, h2, h0.saturating_sub(1)][pick];
    // The first repeat leaves the history as it is; the second swaps the first two; the
    // others push the chosen one in front of the first two.
    let third = if pick == 1 { h2 } else { h1 };
    *scratch = if pick == 0 {
        [h0, h1, h2]
    } else {
        [actual, h0, third]
    };
    actual
}

#[cfg(test)]
mod tests {
    use super::do_offset_history;

    /// The history update as it was written before it was made branchless, case by case.
    fn model(offset_value: u32, lit_len: u32, scratch: &mut [u32; 3]) -> u32 {
        let actual_offset = if lit_len > 0 {
            match offset_value {
                1..=3 => scratch[offset_value as usize - 1],
                _ => {
                    //new offset
                    offset_value - 3
                }
            }
        } else {
            match offset_value {
                1..=2 => scratch[offset_value as usize],
                // A malformed dictionary can seed scratch[0] with 0; saturate so this
                // resolves to 0 (rejected upstream as ZeroOffset) instead of
                // underflowing. See #115.
                3 => scratch[0].saturating_sub(1),
                _ => {
                    //new offset
                    offset_value - 3
                }
            }
        };

        //update history
        if lit_len > 0 {
            match offset_value {
                1 => {
                    //nothing
                }
                2 => {
                    scratch[1] = scratch[0];
                    scratch[0] = actual_offset;
                }
                _ => {
                    scratch[2] = scratch[1];
                    scratch[1] = scratch[0];
                    scratch[0] = actual_offset;
                }
            }
        } else {
            match offset_value {
                1 => {
                    scratch[1] = scratch[0];
                    scratch[0] = actual_offset;
                }
                2 => {
                    scratch[2] = scratch[1];
                    scratch[1] = scratch[0];
                    scratch[0] = actual_offset;
                }
                _ => {
                    scratch[2] = scratch[1];
                    scratch[1] = scratch[0];
                    scratch[0] = actual_offset;
                }
            }
        }

        actual_offset
    }

    #[test]
    fn matches_model() {
        let histories = [[1, 4, 8], [0, 4, 8], [7, 7, 7], [1, 1, 2], [100, 3, 50000]];
        for history in histories {
            for offset_value in 1..12 {
                for lit_len in [0, 1, 5] {
                    let (mut got, mut want) = (history, history);
                    assert_eq!(
                        do_offset_history(offset_value, lit_len, &mut got),
                        model(offset_value, lit_len, &mut want),
                        "{history:?} {offset_value} {lit_len}"
                    );
                    assert_eq!(got, want, "{history:?} {offset_value} {lit_len}");
                }
            }
        }
    }

    #[test]
    fn repeat_offset_minus_one_with_zero_history_does_not_underflow() {
        // A malformed dictionary can seed offset history slot 0 with 0. With
        // literal length 0 and offset code 3 ("repeat the most recent offset,
        // minus one"), `scratch[0] - 1` must not underflow; it should resolve to
        // 0, which the caller rejects as ExecuteSequencesError::ZeroOffset rather
        // than panicking (debug) or wrapping to u32::MAX (release). See #115.
        let mut scratch = [0u32, 4, 8];
        assert_eq!(do_offset_history(3, 0, &mut scratch), 0);
    }
}
