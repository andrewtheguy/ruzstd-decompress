use super::super::blocks::sequence_section::ModeType;
use super::super::blocks::sequence_section::Sequence;
use super::super::blocks::sequence_section::SequencesHeader;
use super::scratch::FSEScratch;
#[cfg(test)]
use super::sequence_table::{LL_DEFAULT_ACC_LOG, LL_DEFAULT_DISTRIBUTION};
use super::sequence_table::{SIZE, SeqEntry, SeqKind, at};
use crate::bit_io::BitReaderReversed;
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

    //skip the 0 padding at the end of the last byte of the bit stream and throw away the first 1 found
    let mut skipped_bits = 0;
    loop {
        let val = br.get_bits(1);
        skipped_bits += 1;
        if val == 1 || skipped_bits > 8 {
            break;
        }
    }
    if skipped_bits > 8 {
        //if more than 7 bits are 0, this is not the correct end of the bitstream. Either a bug or corrupted data
        return Err(DecodeSequenceError::ExtraPadding { skipped_bits }.into());
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
        executor.execute(Sequence {
            ll: ll.base_value + ll_add as u32,
            ml: ml.base_value + ml_add as u32,
            of: of.base_value + of_add as u32,
        })?;
        *seq_idx += 1;
    }
    b.consume_to_sentinel(window);
    *br = b;
    *states = [ll_state, ml_state, of_state];
    Ok(())
}

/// The next `n` bits of `window`, a reader's bits left aligned, taken off its top; `n` may be
/// 0, as the first shift keeps the second under 64.
#[inline(always)]
fn take_bits(window: &mut u64, n: u8) -> u64 {
    let bits = (*window >> 1) >> (63 - n);
    *window <<= n;
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
    let mut table = crate::fse::FSETable::new(MAX_LITERAL_LENGTH_CODE);
    table
        .build_from_probabilities(LL_DEFAULT_ACC_LOG, &LL_DEFAULT_DISTRIBUTION)
        .unwrap();

    #[cfg(feature = "std")]
    for (idx, entry) in table.decode.iter().enumerate() {
        std::println!(
            "{:3}: {:3} {:3} {:3}",
            idx,
            entry.symbol,
            entry.num_bits,
            entry.base_line
        );
    }

    assert!(table.decode.len() == 64);

    //just test a few values. TODO test all values
    assert!(table.decode[0].symbol == 0);
    assert!(table.decode[0].num_bits == 4);
    assert!(table.decode[0].base_line == 0);

    assert!(table.decode[19].symbol == 27);
    assert!(table.decode[19].num_bits == 6);
    assert!(table.decode[19].base_line == 0);

    assert!(table.decode[39].symbol == 25);
    assert!(table.decode[39].num_bits == 4);
    assert!(table.decode[39].base_line == 16);

    assert!(table.decode[60].symbol == 35);
    assert!(table.decode[60].num_bits == 6);
    assert!(table.decode[60].base_line == 0);

    assert!(table.decode[59].symbol == 24);
    assert!(table.decode[59].num_bits == 5);
    assert!(table.decode[59].base_line == 32);
}
