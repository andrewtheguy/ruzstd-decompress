use super::decode_buffer::{DecodeBuffer, LITERALS_PADDING};
use crate::blocks::sequence_section::Sequence;
use crate::decoding::errors::ExecuteSequencesError;

/// Executes sequences one at a time, as they are decoded, so they never have to be stored.
///
/// The fast loop executes them through a [`SequenceRun`], which takes the executor's state as
/// each target compiles it best. Natively, the run holds the buffer's state in locals (the
/// decode buffer's `SequenceWriter`), so that LLVM keeps it in registers, where the stores of
/// the sequences' bytes would otherwise make it reload and store it again for each sequence:
/// silesia 6–9% faster. In WebAssembly, the same run made V8's code 6–12% slower, whose
/// registers it took for values the loop otherwise loads from memory when it needs them,
/// which is cheap there: the run is then the executor itself, as before.
pub struct SequenceExecutor<'a> {
    buffer: &'a mut DecodeBuffer,
    /// The block's literals, then [`LITERALS_PADDING`] bytes, which the buffer's fast copy
    /// reads (the ring buffer's `copy_literals`).
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

    /// A run of sequences (see [`SequenceRun`]).
    #[cfg(not(target_family = "wasm"))]
    #[inline(always)]
    pub fn run(&mut self) -> SequenceRun<'_, 'a> {
        // The writer takes no sequence past the block's most, which the slow path reports.
        let most = self
            .max_block_decoded_len
            .saturating_sub(self.seq_sum as usize);
        SequenceRun {
            out: self.buffer.sequence_writer(most),
            padded_literals: self.padded_literals,
            literals_len: self.literals_len,
            literals_copy_counter: &mut self.literals_copy_counter,
            offset_hist: &mut *self.offset_hist,
            seq_sum: &mut self.seq_sum,
        }
    }

    /// A run of sequences (see [`SequenceRun`]).
    #[cfg(target_family = "wasm")]
    #[inline(always)]
    pub fn run(&mut self) -> SequenceRun<'_, 'a> {
        SequenceRun { executor: self }
    }

    /// One sequence, in a run of its own: for the few decoded outside the fast loop.
    #[inline(never)]
    pub fn execute_out_of_line(&mut self, seq: Sequence) -> Result<(), ExecuteSequencesError> {
        let mut run = self.run();
        let slow = run.execute(seq)?;
        run.finish();
        match slow {
            Some(slow) => self.execute_slow(slow),
            None => Ok(()),
        }
    }

    /// A sequence the buffer's fast copy does not take, once its run has accounted for it and
    /// been finished: the data wraps, the match reaches into the dictionary or overlaps itself
    /// closely, or the buffer needs room.
    #[inline(never)]
    pub fn execute_slow(&mut self, seq: SlowSequence) -> Result<(), ExecuteSequencesError> {
        // Natively, the block's decoded length is checked here, the fast loop's writer taking
        // no sequence past it; in WebAssembly, for each sequence before its copy.
        #[cfg(not(target_family = "wasm"))]
        {
            self.seq_sum += (seq.ll + seq.ml) as u32;
            check_decoded_len(self.seq_sum, self.max_block_decoded_len)?;
        }
        // In WebAssembly, the ring's fast copy takes no match into the dictionary; the
        // writer that natively takes every sequence takes those here (see `DecodeBuffer`).
        #[cfg(target_family = "wasm")]
        if self.buffer.push_from_dict(
            &self.padded_literals[seq.literals_start..],
            seq.ll,
            seq.offset,
            seq.ml,
        ) {
            return Ok(());
        }
        let literals: &[u8] = self.padded_literals;
        let literals = &literals[seq.literals_start..seq.literals_start + seq.ll];
        if !literals.is_empty() {
            self.buffer.push(literals);
        }
        if seq.ml > 0 {
            self.buffer.repeat(seq.offset, seq.ml)?;
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
            check_decoded_len(seq_sum, self.max_block_decoded_len)?;
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
}

/// A sequence for [`SequenceExecutor::execute_slow`]: where its literals start, their count,
/// its offset resolved, and its match length.
pub struct SlowSequence {
    literals_start: usize,
    ll: usize,
    offset: usize,
    ml: usize,
}

/// Sequences executed in a run: natively, with the buffer's state in a [`SequenceWriter`],
/// which [`SequenceRun::finish`] writes back to the buffer. Dropped without it, on an error,
/// the buffer is left as it was when the run began, the block's undoing then setting it back
/// further. A sequence the buffer's fast copy does not take is accounted for and returned, for
/// [`SequenceExecutor::execute_slow`] once the run is finished.
///
/// [`SequenceWriter`]: super::decode_buffer::SequenceWriter
#[cfg(not(target_family = "wasm"))]
pub struct SequenceRun<'x, 'a> {
    out: super::decode_buffer::SequenceWriter<'x>,
    padded_literals: &'a [u8],
    literals_len: usize,
    literals_copy_counter: &'x mut usize,
    offset_hist: &'x mut [u32; 3],
    /// The block's decoded length, which the writer's bytes add to when the run finishes.
    seq_sum: &'x mut u32,
}

#[cfg(not(target_family = "wasm"))]
impl SequenceRun<'_, '_> {
    /// Copy the literals of `seq` and then its match to the end of the buffer, or return what
    /// [`SequenceExecutor::execute_slow`] is to copy.
    #[inline(always)]
    pub fn execute(
        &mut self,
        seq: Sequence,
    ) -> Result<Option<SlowSequence>, ExecuteSequencesError> {
        execute_with(
            seq,
            self.padded_literals,
            self.literals_len,
            self.literals_copy_counter,
            self.offset_hist,
            None,
            &mut self.out,
        )
    }

    /// Write the buffer's state back.
    #[inline(always)]
    pub fn finish(self) {
        *self.seq_sum += self.out.finish() as u32;
    }
}

/// Sequences executed in a run: in WebAssembly, by the executor itself (see
/// [`SequenceExecutor`]), a sequence the buffer's fast copy does not take included.
#[cfg(target_family = "wasm")]
pub struct SequenceRun<'x, 'a> {
    executor: &'x mut SequenceExecutor<'a>,
}

#[cfg(target_family = "wasm")]
impl SequenceRun<'_, '_> {
    /// Copy the literals of `seq` and then its match to the end of the buffer.
    #[inline(always)]
    pub fn execute(
        &mut self,
        seq: Sequence,
    ) -> Result<Option<SlowSequence>, ExecuteSequencesError> {
        let e = &mut *self.executor;
        let slow = execute_with(
            seq,
            e.padded_literals,
            e.literals_len,
            &mut e.literals_copy_counter,
            e.offset_hist,
            Some((&mut e.seq_sum, e.max_block_decoded_len)),
            &mut *e.buffer,
        )?;
        if let Some(slow) = slow {
            self.executor.execute_slow(slow)?;
        }
        Ok(None)
    }

    /// Nothing to write back.
    #[inline(always)]
    pub fn finish(self) {}
}

/// The buffer's fast copy of a sequence, by [`SequenceRun`]'s writer: `false`, having changed
/// nothing, where it does not take it. A trait, not a closure, since a closure cannot be
/// marked to be inlined, and LLVM left one out of line, 36% of silesia's instructions.
trait PushSequence {
    fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool;
}

#[cfg(not(target_family = "wasm"))]
impl PushSequence for super::decode_buffer::SequenceWriter<'_> {
    #[inline(always)]
    fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        self.push_sequence(literals, ll, offset, ml)
    }
}

#[cfg(target_family = "wasm")]
impl PushSequence for DecodeBuffer {
    #[inline(always)]
    fn push_sequence(&mut self, literals: &[u8], ll: usize, offset: usize, ml: usize) -> bool {
        self.push_sequence(literals, ll, offset, ml)
    }
}

/// A sequence's checks and accounting, its literals' bound, its offset resolved and, given
/// `seq_sum`, the block's decoded length and its most, with its copy by `out`, the buffer's
/// fast copy: what is left to [`SequenceExecutor::execute_slow`] where that does not take it.
/// Natively, the writer bounds the block's length instead (see [`SequenceExecutor::run`]).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn execute_with(
    seq: Sequence,
    padded_literals: &[u8],
    literals_len: usize,
    literals_copy_counter: &mut usize,
    offset_hist: &mut [u32; 3],
    seq_sum: Option<(&mut u32, usize)>,
    out: &mut impl PushSequence,
) -> Result<Option<SlowSequence>, ExecuteSequencesError> {
    let (ll, ml) = (seq.ll as usize, seq.ml as usize);
    let start = *literals_copy_counter;
    let high = start + ll;
    if high > literals_len {
        return Err(ExecuteSequencesError::NotEnoughBytesForSequence {
            wanted: high,
            have: literals_len,
        });
    }
    let offset = do_offset_history(seq.of, seq.ll, offset_hist) as usize;
    if offset == 0 {
        return Err(ExecuteSequencesError::ZeroOffset);
    }
    *literals_copy_counter = high;
    if let Some((seq_sum, max)) = seq_sum {
        *seq_sum += seq.ml;
        *seq_sum += seq.ll;
        check_decoded_len(*seq_sum, max)?;
    }
    if out.push_sequence(&padded_literals[start..], ll, offset, ml) {
        Ok(None)
    } else {
        Ok(Some(SlowSequence {
            literals_start: start,
            ll,
            offset,
            ml,
        }))
    }
}

/// Block_Maximum_Size bounds every block; checking after each sequence also keeps the sum of
/// their lengths far from overflowing.
#[inline(always)]
fn check_decoded_len(decoded: u32, max: usize) -> Result<(), ExecuteSequencesError> {
    if decoded as usize > max {
        return Err(ExecuteSequencesError::TooManyBytesGenerated {
            max,
            decoded: decoded as usize,
        });
    }
    Ok(())
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
