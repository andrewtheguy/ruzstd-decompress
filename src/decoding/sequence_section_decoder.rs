use super::super::blocks::sequence_section::ModeType;
use super::super::blocks::sequence_section::Sequence;
use super::super::blocks::sequence_section::SequencesHeader;
use super::scratch::FSEScratch;
#[cfg(test)]
use super::sequence_table::{LL_DEFAULT_ACC_LOG, LL_DEFAULT_DISTRIBUTION};
use super::sequence_table::{SIZE, SeqEntry, SeqKind, at};
use crate::bit_io::BitReaderReversed;
#[cfg(not(target_family = "wasm"))]
use crate::bit_io::refill_raw;
use crate::bit_io::{PADDED_LEFT, PADDING};
use crate::blocks::sequence_section::{
    MAX_LITERAL_LENGTH_CODE, MAX_MATCH_LENGTH_CODE, MAX_OFFSET_CODE,
};
use crate::decoding::errors::{DecodeSequenceError, ExecuteSequencesError};
use crate::decoding::sequence_execution::SequenceExecutor;

/// Decode the provided source as a series of sequences, executing each one as soon as it is
/// decoded.
pub fn decode_sequences<E: From<DecodeSequenceError> + From<ExecuteSequencesError>>(
    section: &SequencesHeader,
    source: &[u8],
    scratch: &mut FSEScratch,
    executor: &mut SequenceExecutor<'_>,
) -> Result<(), E> {
    let bytes_read = maybe_update_fse_tables(section, source, scratch)?;

    vprintln!("Updating tables used {} bytes", bytes_read);

    let bit_stream = &source[bytes_read..];

    // Where the stream's last bytes are copied after zeros, so that the fast loop goes on to
    // its end (see `BitReaderReversed::padded`), and what that adds to the bits remaining.
    let mut padded = [0u8; PADDING + PADDED_LEFT];
    let mut padding_bits = 0;
    let mut br = BitReaderReversed::new(bit_stream);

    // Skip the zeros at the top of the stream's last byte and the 1 after them; a last byte of
    // zeros, or none, is not the stream's end (the reader goes on into the byte before, so the
    // error counts 9 bits, as it read them one at a time).
    match bit_stream.last() {
        Some(&last) if last != 0 => {
            br.get_bits(last.leading_zeros() as u8 + 1);
        }
        _ => return Err(DecodeSequenceError::ExtraPadding { skipped_bits: 9 }.into()),
    }

    // An RLE table is a table of one state that reads no bits, so it needs no case of its own.
    let (ll_log, ll_table) = scratch.ll.states()?;
    let (of_log, of_table) = scratch.of.states()?;
    let (ml_log, ml_table) = scratch.ml.states()?;
    let ll_state = br.get_bits(ll_log) as usize;
    let of_state = br.get_bits(of_log) as usize;
    let ml_state = br.get_bits(ml_log) as usize;

    // Every sequence but the last, while the source has room for two refills, from the reader's
    // bits as a window (see `BitReaderReversed::sentinel_window`): a refill after each sequence
    // and none of the reader's checks. The source is never used up here, so the stream's end
    // is checked only after the loop. What a sequence reads: the offset's and the match
    // length's extra bits, at most 31 + 16, then the literal length's and the three next
    // states', at most 16 + 9 + 9 + 8; a refill between them when the three lengths' extra
    // bits are over 56 - 26, as libzstd reloads.
    //
    // The loop works on a copy of the reader, whose address is not taken, so that its state
    // stays in registers, and executes each sequence inline. Near the stream's start it goes on
    // on a padded copy of what is left, which may read past that start: what it decodes from
    // there is undone with the block when the bits remaining are checked after it.
    let mut seq_idx = 0;
    let tables = [ll_table, ml_table, of_table];
    let mut states = [ll_state, ml_state, of_state];
    // Natively, one call for both, so that a small block pays for the loop's setup once.
    #[cfg(not(target_family = "wasm"))]
    if fast_sequences::<E>(
        &mut br,
        &mut padded,
        tables,
        &mut states,
        executor,
        &mut seq_idx,
        section.num_sequences,
    )? {
        padding_bits = PADDING as isize * 8;
    }
    #[cfg(target_family = "wasm")]
    if section.num_sequences > 1 && br.can_refill_fast() {
        fast_sequences::<E>(
            &mut br,
            tables,
            &mut states,
            executor,
            &mut seq_idx,
            section.num_sequences,
        )?;
    }
    #[cfg(target_family = "wasm")]
    if seq_idx + 1 < section.num_sequences
        && let Some(copy) = br.padded(&mut padded)
    {
        br = copy;
        padding_bits = PADDING as isize * 8;
        fast_sequences::<E>(
            &mut br,
            tables,
            &mut states,
            executor,
            &mut seq_idx,
            section.num_sequences,
        )?;
    }
    let [mut ll_state, mut ml_state, mut of_state] = states;

    for seq_idx in seq_idx..section.num_sequences {
        let ll = at(ll_table, ll_state);
        let ml = at(ml_table, ml_state);
        let of = at(of_table, of_state);

        let (of_add, ml_add, ll_add) =
            br.get_bits_triple(of.nb_add_bits, ml.nb_add_bits, ll.nb_add_bits);
        // An offset code's value is 1 << code, so offsets are never 0.
        let seq = Sequence {
            ll: ll.base_value + ll_add as u32,
            ml: ml.base_value + ml_add as u32,
            of: of.base_value + of_add as u32,
        };

        if seq_idx + 1 < section.num_sequences {
            let (ll_next, ml_next, of_next) =
                br.get_bits_triple(ll.nb_bits, ml.nb_bits, of.nb_bits);
            ll_state = usize::from(ll.next_state) + ll_next as usize;
            ml_state = usize::from(ml.next_state) + ml_next as usize;
            of_state = usize::from(of.next_state) + of_next as usize;
        }

        if br.bits_remaining() - padding_bits < 0 {
            return Err(DecodeSequenceError::NotEnoughBytesForNumSequences.into());
        }

        executor.execute_out_of_line(seq)?;
    }

    let bits_remaining = br.bits_remaining() - padding_bits;
    if bits_remaining > 0 {
        Err(DecodeSequenceError::ExtraBits { bits_remaining }.into())
    } else if bits_remaining < 0 {
        // The fast loop read past the stream's start.
        Err(DecodeSequenceError::NotEnoughBytesForNumSequences.into())
    } else {
        Ok(())
    }
}

/// The fast loop of [`decode_sequences`]: sequences while `br` can refill twice and one is
/// left after them, the states and the count moved on.
///
/// Natively, as libzstd's loop: the reader's position a raw pointer and the executor's state
/// in a [`RawRun`] of locals, so that the loop keeps them in registers and checks no bounds
/// but its own.
///
/// [`RawRun`]: crate::decoding::sequence_execution::RawRun
///
/// On x86-64 it runs in a build with BMI2 where the processor has it, as libzstd's does: its
/// shifts take their count from any register, where the others' take it only from `cl`.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
fn fast_sequences<'x, E: From<DecodeSequenceError> + From<ExecuteSequencesError>>(
    br: &mut BitReaderReversed<'x>,
    padded: &'x mut [u8; PADDING + PADDED_LEFT],
    tables: [&[SeqEntry; SIZE]; 3],
    states: &mut [usize; 3],
    executor: &mut SequenceExecutor<'_>,
    seq_idx: &mut u32,
    num_sequences: u32,
) -> Result<bool, E> {
    #[cfg(all(target_arch = "x86_64", not(target_feature = "bmi2")))]
    if crate::common::cpu::has_bmi2() {
        // SAFETY: the processor has BMI2.
        return unsafe {
            fast_sequences_bmi2(br, padded, tables, states, executor, seq_idx, num_sequences)
        };
    }
    fast_sequences_body(br, padded, tables, states, executor, seq_idx, num_sequences)
}

/// [`fast_sequences_body`] built with BMI2.
#[cfg(all(target_arch = "x86_64", not(target_feature = "bmi2")))]
#[target_feature(enable = "bmi2")]
fn fast_sequences_bmi2<'x, E: From<DecodeSequenceError> + From<ExecuteSequencesError>>(
    br: &mut BitReaderReversed<'x>,
    padded: &'x mut [u8; PADDING + PADDED_LEFT],
    tables: [&[SeqEntry; SIZE]; 3],
    states: &mut [usize; 3],
    executor: &mut SequenceExecutor<'_>,
    seq_idx: &mut u32,
    num_sequences: u32,
) -> Result<bool, E> {
    fast_sequences_body(br, padded, tables, states, executor, seq_idx, num_sequences)
}

/// The native [`fast_sequences`]: on `br` while it can refill fast, then on its padded copy in
/// `padded` (see [`BitReaderReversed::padded`]), to which it sets `br`, returning `true`.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
fn fast_sequences_body<'x, E: From<DecodeSequenceError> + From<ExecuteSequencesError>>(
    br: &mut BitReaderReversed<'x>,
    padded: &'x mut [u8; PADDING + PADDED_LEFT],
    [ll_table, ml_table, of_table]: [&[SeqEntry; SIZE]; 3],
    states: &mut [usize; 3],
    executor: &mut SequenceExecutor<'_>,
    seq_idx: &mut u32,
    num_sequences: u32,
) -> Result<bool, E> {
    if num_sequences <= *seq_idx {
        return Ok(false);
    }
    // The sequences the loop may take, all but the last, which is taken after them where the
    // source has room, without the next states' bits.
    let mut left = num_sequences - *seq_idx - 1;
    let first_left = left;
    let mut last = true;
    let [mut ll_state, mut ml_state, mut of_state] = *states;
    let mut padded = Some(padded);
    let mut on_padded = false;
    let mut run = executor.raw_run();
    loop {
        if br.can_refill_fast() {
            br.ensure_bits_fast(57);
            let mut window = br.sentinel_window();
            let (src, index) = br.raw_index();
            let src_addr = src as usize;
            // SAFETY: the index is within the source.
            let mut ip = unsafe { src.add(index) };
            // In chunks, each as many sequences as the source has room for: a sequence refills
            // at most twice, each moving `ip` down by at most 7, so a chunk of `k` from at least
            // `15 + 14 * (k - 1)` bytes past the start leaves `ip` at least 1 past, with the 8
            // bytes from it in the source, as `ip` started with. Within a chunk, the loop checks
            // only its count.
            loop {
                // `(d - 15) / 14 + 1` sequences from `d` bytes past the start, none below 15.
                let room = (ip as usize - src_addr).saturating_sub(1) / 14;
                let mut chunk = left.min(room as u32);
                if chunk == 0 {
                    break;
                }
                left -= chunk;
                while chunk > 0 {
                    // SAFETY: the chunk's count bounds the refills, as above.
                    window = unsafe { refill_raw(&mut ip, window) };
                    let ll = at(ll_table, ll_state);
                    let ml = at(ml_table, ml_state);
                    let of = at(of_table, of_state);
                    let of_add = take_bits(&mut window, of.nb_add_bits);
                    let ml_add = take_bits(&mut window, ml.nb_add_bits);
                    if of.nb_add_bits + ml.nb_add_bits + ll.nb_add_bits > 56 - 26 {
                        // SAFETY: the second of the sequence's two refills, as above.
                        window = unsafe { refill_raw(&mut ip, window) };
                    }
                    let ll_add = take_bits(&mut window, ll.nb_add_bits);
                    ll_state =
                        usize::from(ll.next_state) + take_bits(&mut window, ll.nb_bits) as usize;
                    ml_state =
                        usize::from(ml.next_state) + take_bits(&mut window, ml.nb_bits) as usize;
                    of_state =
                        usize::from(of.next_state) + take_bits(&mut window, of.nb_bits) as usize;
                    let slow = run.execute(
                        ll.base_value + ll_add as u32,
                        ml.base_value + ml_add as u32,
                        of.base_value + of_add as u32,
                    );
                    match slow {
                        Ok(None) => {}
                        Ok(Some(slow)) => {
                            executor.finish_raw(run);
                            executor.execute_slow(slow)?;
                            run = executor.raw_run();
                        }
                        Err(err) => return Err(err.into()),
                    }
                    chunk -= 1;
                }
            }
            if left == 0 && ip as usize - src_addr >= 15 {
                // SAFETY: as for a chunk of one.
                window = unsafe { refill_raw(&mut ip, window) };
                let ll = at(ll_table, ll_state);
                let ml = at(ml_table, ml_state);
                let of = at(of_table, of_state);
                let of_add = take_bits(&mut window, of.nb_add_bits);
                let ml_add = take_bits(&mut window, ml.nb_add_bits);
                if of.nb_add_bits + ml.nb_add_bits + ll.nb_add_bits > 56 - 26 {
                    // SAFETY: as for a chunk of one.
                    window = unsafe { refill_raw(&mut ip, window) };
                }
                let ll_add = take_bits(&mut window, ll.nb_add_bits);
                let slow = run.execute(
                    ll.base_value + ll_add as u32,
                    ml.base_value + ml_add as u32,
                    of.base_value + of_add as u32,
                );
                match slow {
                    Ok(None) => {}
                    Ok(Some(slow)) => {
                        executor.finish_raw(run);
                        executor.execute_slow(slow)?;
                        run = executor.raw_run();
                    }
                    Err(err) => return Err(err.into()),
                }
                last = false;
            }
            br.resume_index(ip as usize - src_addr, window);
        }
        // Near the stream's start, on a padded copy of what is left, once.
        if left == 0 && !last {
            break;
        }
        let Some(buf) = padded.take() else {
            break;
        };
        let Some(copy) = br.padded(buf) else {
            break;
        };
        *br = copy;
        on_padded = true;
    }
    executor.finish_raw(run);
    *seq_idx += first_left - left + u32::from(!last);
    *states = [ll_state, ml_state, of_state];
    Ok(on_padded)
}

/// The fast loop of [`decode_sequences`]: sequences while `br` can refill twice and one is
/// left after them, the states and the count moved on.
#[cfg(target_family = "wasm")]
#[inline(always)]
fn fast_sequences<E: From<DecodeSequenceError> + From<ExecuteSequencesError>>(
    br: &mut BitReaderReversed<'_>,
    [ll_table, ml_table, of_table]: [&[SeqEntry; SIZE]; 3],
    states: &mut [usize; 3],
    executor: &mut SequenceExecutor<'_>,
    seq_idx: &mut u32,
    num_sequences: u32,
) -> Result<(), E> {
    let [mut ll_state, mut ml_state, mut of_state] = *states;
    let mut b = br.clone();
    b.ensure_bits_fast(57);
    let mut window = b.sentinel_window();
    let mut run = executor.run();
    while *seq_idx + 1 < num_sequences && b.can_refill_fast_twice() {
        window = b.refill_window_fast(window);
        let ll = at(ll_table, ll_state);
        let ml = at(ml_table, ml_state);
        let of = at(of_table, of_state);
        let of_add = take_bits(&mut window, of.nb_add_bits);
        let ml_add = take_bits(&mut window, ml.nb_add_bits);
        if of.nb_add_bits + ml.nb_add_bits + ll.nb_add_bits > 56 - 26 {
            window = b.refill_window_fast(window);
        }
        let ll_add = take_bits(&mut window, ll.nb_add_bits);
        ll_state = usize::from(ll.next_state) + take_bits(&mut window, ll.nb_bits) as usize;
        ml_state = usize::from(ml.next_state) + take_bits(&mut window, ml.nb_bits) as usize;
        of_state = usize::from(of.next_state) + take_bits(&mut window, of.nb_bits) as usize;
        let slow = run.execute(Sequence {
            ll: ll.base_value + ll_add as u32,
            ml: ml.base_value + ml_add as u32,
            of: of.base_value + of_add as u32,
        })?;
        if let Some(slow) = slow {
            run.finish();
            executor.execute_slow(slow)?;
            run = executor.run();
        }
        *seq_idx += 1;
    }
    run.finish();
    b.consume_to_sentinel(window);
    *br = b;
    *states = [ll_state, ml_state, of_state];
    Ok(())
}

/// The next `n` bits of `window`, a reader's bits left aligned, taken off its top; `n` may be
/// 0, as the first shift keeps the second under 64.
#[inline(always)]
fn take_bits(window: &mut u64, n: u8) -> u64 {
    // `!n` is `63 - n` in the shift's six bits, which x86's and wasm's shifts mask the count
    // to: no masking of their own.
    let bits = (*window >> 1).wrapping_shr(!u32::from(n));
    *window = window.wrapping_shl(u32::from(n));
    bits
}

// This info is buried in the symbol compression mode table
/// "The maximum allowed accuracy log for literals length and match length tables is 9"
pub const LL_MAX_LOG: u8 = 9;
/// "The maximum allowed accuracy log for literals length and match length tables is 9"
pub const ML_MAX_LOG: u8 = 9;
/// "The maximum accuracy log for the offset table is 8."
pub const OF_MAX_LOG: u8 = 8;

fn maybe_update_fse_tables(
    section: &SequencesHeader,
    source: &[u8],
    scratch: &mut FSEScratch,
) -> Result<usize, DecodeSequenceError> {
    let modes = section
        .modes
        .ok_or(DecodeSequenceError::MissingCompressionMode)?;

    let mut bytes_read = 0;

    match modes.ll_mode() {
        ModeType::FSECompressed => {
            let bytes = scratch
                .literal_lengths
                .read_distribution(source, LL_MAX_LOG)?;
            bytes_read += bytes;

            vprintln!("Updating ll table");
            vprintln!("Used bytes: {}", bytes);
            scratch
                .ll
                .build_from_fse(&scratch.literal_lengths, SeqKind::LiteralLength)?;
        }
        ModeType::RLE => {
            vprintln!("Use RLE ll table");
            if source.is_empty() {
                return Err(DecodeSequenceError::MissingByteForRleLlTable);
            }
            bytes_read += 1;
            if source[0] > MAX_LITERAL_LENGTH_CODE {
                return Err(DecodeSequenceError::MissingByteForRleMlTable);
            }
            scratch.ll.build_rle(source[0], SeqKind::LiteralLength);
        }
        ModeType::Predefined => {
            vprintln!("Use predefined ll table");
            scratch.ll.build_predefined(SeqKind::LiteralLength)?;
        }
        ModeType::Repeat => {
            vprintln!("Repeat ll table");
            /* Nothing to do */
        }
    };

    let of_source = &source[bytes_read..];

    match modes.of_mode() {
        ModeType::FSECompressed => {
            let bytes = scratch.offsets.read_distribution(of_source, OF_MAX_LOG)?;
            vprintln!("Updating of table");
            vprintln!("Used bytes: {}", bytes);
            bytes_read += bytes;
            scratch
                .of
                .build_from_fse(&scratch.offsets, SeqKind::Offset)?;
        }
        ModeType::RLE => {
            vprintln!("Use RLE of table");
            if of_source.is_empty() {
                return Err(DecodeSequenceError::MissingByteForRleOfTable);
            }
            bytes_read += 1;
            if of_source[0] > MAX_OFFSET_CODE {
                return Err(DecodeSequenceError::MissingByteForRleMlTable);
            }
            scratch.of.build_rle(of_source[0], SeqKind::Offset);
        }
        ModeType::Predefined => {
            vprintln!("Use predefined of table");
            scratch.of.build_predefined(SeqKind::Offset)?;
        }
        ModeType::Repeat => {
            vprintln!("Repeat of table");
            /* Nothing to do */
        }
    };

    let ml_source = &source[bytes_read..];

    match modes.ml_mode() {
        ModeType::FSECompressed => {
            let bytes = scratch
                .match_lengths
                .read_distribution(ml_source, ML_MAX_LOG)?;
            bytes_read += bytes;
            vprintln!("Updating ml table");
            vprintln!("Used bytes: {}", bytes);
            scratch
                .ml
                .build_from_fse(&scratch.match_lengths, SeqKind::MatchLength)?;
        }
        ModeType::RLE => {
            vprintln!("Use RLE ml table");
            if ml_source.is_empty() {
                return Err(DecodeSequenceError::MissingByteForRleMlTable);
            }
            bytes_read += 1;
            if ml_source[0] > MAX_MATCH_LENGTH_CODE {
                return Err(DecodeSequenceError::MissingByteForRleMlTable);
            }
            scratch.ml.build_rle(ml_source[0], SeqKind::MatchLength);
        }
        ModeType::Predefined => {
            vprintln!("Use predefined ml table");
            scratch.ml.build_predefined(SeqKind::MatchLength)?;
        }
        ModeType::Repeat => {
            vprintln!("Repeat ml table");
            /* Nothing to do */
        }
    };

    Ok(bytes_read)
}

#[test]
fn test_ll_default() {
    // (symbol, bits, base) of each state
    let mut table = alloc::vec![(0, 0, 0); 1 << LL_DEFAULT_ACC_LOG];
    crate::fse::build_states(
        LL_DEFAULT_ACC_LOG,
        &LL_DEFAULT_DISTRIBUTION,
        |state, symbol, num_bits, base_line| table[state] = (symbol, num_bits, base_line),
    )
    .unwrap();

    assert!(table.len() == 64);

    //just test a few values. TODO test all values
    assert_eq!(table[0], (0, 4, 0));
    assert_eq!(table[19], (27, 6, 0));
    assert_eq!(table[39], (25, 4, 16));
    assert_eq!(table[60], (35, 6, 0));
    assert_eq!(table[59], (24, 5, 32));
}
