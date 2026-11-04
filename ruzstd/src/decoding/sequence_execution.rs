use super::decode_buffer::DecodeBuffer;
use crate::blocks::sequence_section::Sequence;
use crate::decoding::errors::ExecuteSequencesError;

/// Executes sequences one at a time, as they are decoded, so they never have to be stored.
pub struct SequenceExecutor<'a> {
    buffer: &'a mut DecodeBuffer,
    literals: &'a [u8],
    offset_hist: &'a mut [u32; 3],
    literals_copy_counter: usize,
    old_buffer_size: usize,
    seq_sum: u32,
    max_block_decoded_len: usize,
}

impl<'a> SequenceExecutor<'a> {
    pub fn new(
        buffer: &'a mut DecodeBuffer,
        literals: &'a [u8],
        offset_hist: &'a mut [u32; 3],
        max_block_decoded_len: usize,
    ) -> Self {
        let old_buffer_size = buffer.len();
        SequenceExecutor {
            buffer,
            literals,
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
        if seq.ll > 0 {
            let high = self.literals_copy_counter + seq.ll as usize;
            if high > self.literals.len() {
                return Err(ExecuteSequencesError::NotEnoughBytesForSequence {
                    wanted: high,
                    have: self.literals.len(),
                });
            }
            self.buffer
                .push(&self.literals[self.literals_copy_counter..high]);
            self.literals_copy_counter += seq.ll as usize;
        }

        let actual_offset = do_offset_history(seq.of, seq.ll, self.offset_hist);
        if actual_offset == 0 {
            return Err(ExecuteSequencesError::ZeroOffset);
        }
        if seq.ml > 0 {
            self.buffer
                .repeat(actual_offset as usize, seq.ml as usize)?;
        }

        self.seq_sum += seq.ml;
        self.seq_sum += seq.ll;
        self.check_decoded_len(self.seq_sum)
    }

    /// Copy the literals left over after the last sequence.
    pub fn finish(self) -> Result<(), ExecuteSequencesError> {
        let mut seq_sum = self.seq_sum;
        if self.literals_copy_counter < self.literals.len() {
            let rest_literals = &self.literals[self.literals_copy_counter..];
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
fn do_offset_history(offset_value: u32, lit_len: u32, scratch: &mut [u32; 3]) -> u32 {
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

#[cfg(test)]
mod tests {
    use super::do_offset_history;

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
