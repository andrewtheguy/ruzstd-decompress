use crate::bit_io::GetBitsError;
use crate::decoding::errors::FSETableError;
use alloc::vec::Vec;

/// The description of an FSE table, the probabilities of all symbols from 0 to the highest
/// present one, as read from a stream: each user builds its own table from it (see
/// [`build_states`]).
///
/// <https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md#fse-table-description>
#[derive(Debug, Clone)]
pub struct FSETable {
    /// The maximum symbol in the table (inclusive). Limits the probabilities length to max_symbol + 1.
    max_symbol: u8,
    /// The size of the table is stored in logarithm base 2 format,
    /// with the **size of the table** being equal to `(1 << accuracy_log)`.
    /// This value is used so that the decoder knows how many bits to read from the bitstream.
    pub accuracy_log: u8,
    /// In this context, probability refers to the likelihood that a symbol occurs in the given data.
    /// Given this info, the encoder can assign shorter codes to symbols that appear more often,
    /// and longer codes that appear less often, then the decoder can use the probability
    /// to determine what code was assigned to what symbol.
    ///
    /// The probability of a single symbol is a value representing the proportion of times the symbol
    /// would fall within the data.
    ///
    /// If a symbol probability is set to `-1`, it means that the probability of a symbol
    /// occurring in the data is less than one.
    pub symbol_probabilities: Vec<i32>,
}

impl FSETable {
    /// Initialize a new empty Finite State Entropy decoding table.
    pub fn new(max_symbol: u8) -> FSETable {
        FSETable {
            max_symbol,
            symbol_probabilities: Vec::with_capacity(256), //will never be more than 256 symbols because u8
            accuracy_log: 0,
        }
    }

    /// Empty the description.
    pub fn reset(&mut self) {
        self.symbol_probabilities.clear();
        self.accuracy_log = 0;
    }

    /// Read a table's description, its accuracy log and probabilities, for a caller that builds
    /// its own table from them (see [`build_states`]). Returns how many bytes were read.
    pub fn read_distribution(
        &mut self,
        source: &[u8],
        max_log: u8,
    ) -> Result<usize, FSETableError> {
        self.accuracy_log = 0;
        self.read_probabilities(source, max_log)
    }

    /// Read the accuracy log and the probabilities from `source`, as libzstd reads them
    /// (`FSE_readNCount`), and return how many bytes they took. If the table is larger than
    /// `max_log` allows, return an error.
    fn read_probabilities(&mut self, source: &[u8], max_log: u8) -> Result<usize, FSETableError> {
        let probabilities = &mut self.symbol_probabilities;
        probabilities.clear();
        let most = self.max_symbol as usize + 1;
        let source_bits = source.len() * 8;
        if source_bits < 4 {
            return Err(FSETableError::GetBitsError(
                GetBitsError::NotEnoughRemainingBits {
                    requested: 4,
                    remaining: source_bits,
                },
            ));
        }

        let accuracy_log = ACC_LOG_OFFSET + (bits_at(source, 0) & 0xF) as u8;
        self.accuracy_log = accuracy_log;
        if accuracy_log > max_log {
            return Err(FSETableError::AccLogTooBig {
                got: accuracy_log,
                max: max_log,
            });
        }

        // What is left of the table's states, plus one: a value is at most this, and is read
        // from `num_bits` bits, or one fewer for the smallest ones. `threshold` is the largest
        // power of two not above `remaining`, `num_bits` one more than its log; both only fall,
        // a step at a time, so that no value waits on counting the bits it is read from.
        let sum = 1u32 << accuracy_log;
        let mut remaining = sum + 1;
        let mut threshold = sum;
        let mut num_bits = accuracy_log as usize + 1;
        let mut at = 4;
        while remaining > 1 {
            // The bits are read past the source's end as zeroes, which reads every
            // probability as "less than one" and ends the loop; the end is checked after it.
            let window = bits_at(source, at) as u32;
            let max = (2 * threshold - 1) - remaining;
            let value = if window & (threshold - 1) < max {
                at += num_bits - 1;
                window & (threshold - 1)
            } else {
                at += num_bits;
                let value = window & (2 * threshold - 1);
                if value >= threshold {
                    value - max
                } else {
                    value
                }
            };
            let prob = value as i32 - 1;
            probabilities.push(prob);
            if prob == 0 {
                // More zeros, 3 to each 2 bits read until they are not 3.
                loop {
                    let repeat = (bits_at(source, at) & 3) as usize;
                    at += 2;
                    let len = probabilities.len() + repeat;
                    if len > most {
                        return Err(FSETableError::TooManySymbols { got: len });
                    }
                    probabilities.resize(len, 0);
                    if repeat != 3 {
                        break;
                    }
                }
            } else {
                // a probability of -1 counts as 1
                remaining -= prob.unsigned_abs();
                while remaining < threshold {
                    num_bits -= 1;
                    threshold >>= 1;
                }
            }
            if probabilities.len() > most {
                return Err(FSETableError::TooManySymbols {
                    got: probabilities.len(),
                });
            }
        }
        if at > source_bits {
            return Err(FSETableError::GetBitsError(
                GetBitsError::NotEnoughRemainingBits {
                    requested: at,
                    remaining: source_bits,
                },
            ));
        }
        Ok(at.div_ceil(8))
    }
}

/// The bits of `source` from bit `at` on, the first the lowest: at least 56 of them, zeroes
/// past its end.
#[inline(always)]
fn bits_at(source: &[u8], at: usize) -> u64 {
    let byte = at / 8;
    let word = match source.get(byte..byte + 8) {
        Some(bytes) => u64::from_le_bytes(bytes.try_into().unwrap()),
        None => {
            let rest = source.get(byte..).unwrap_or(&[]);
            let mut bytes = [0; 8];
            bytes[..rest.len()].copy_from_slice(rest);
            u64::from_le_bytes(bytes)
        }
    };
    word >> (at % 8)
}

/// Work out the states of the table of `probabilities`, of `1 << acc_log` states, and hand
/// each to `state` in order: its index, symbol, the bits read for the next state and the
/// base they are added to.
///
/// As libzstd builds it (`FSE_buildDTable`): a symbol of probability `p` has the states
/// `p..2 * p`, given out in table order, and state `x` reads `nb = log - floor(log2(x))`
/// bits onto a base of `(x << nb) - table_size`. A "less than one" probability is one
/// state, at the top of the table. The probabilities must sum to the table's size, a "less
/// than one" counting one, as [`FSETable`] checks when it reads them.
#[inline(always)]
pub(crate) fn build_states(
    acc_log: u8,
    probabilities: &[i32],
    mut state: impl FnMut(usize, u8, u8, u32),
) -> Result<(), FSETableError> {
    if acc_log > MAX_ACC_LOG {
        return Err(FSETableError::AccLogTooBig {
            got: acc_log,
            max: MAX_ACC_LOG,
        });
    }
    if probabilities.len() > 256 {
        return Err(FSETableError::TooManySymbols {
            got: probabilities.len(),
        });
    }
    let table_size = 1 << acc_log;

    // Each symbol's next state; a "less than one" symbol has one state, at the top of the
    // table, so those take the states from `high` up.
    let mut next = [0u16; 256];
    let mut high = table_size;
    for (symbol, &prob) in probabilities.iter().enumerate() {
        if prob == -1 {
            high -= 1;
            next[symbol] = 1;
        } else {
            next[symbol] = prob.max(0) as u16;
        }
    }

    // The other symbols, spread over the rest in a semi-random order: laid out in a row, a
    // symbol's run written 8 bytes at a time, so that no loop is as long as a symbol's
    // probability, and the row's `k`th symbol given the `k`th position of a walk round the
    // table by `step`, past the states from `high` up.
    let mut row = [0u8; MAX_TABLE_SIZE + 8];
    let mut end = 0;
    for (symbol, &prob) in probabilities.iter().enumerate() {
        let run = prob.max(0) as usize;
        let mut at = end;
        loop {
            row[at..at + 8].fill(symbol as u8);
            at += 8;
            if at >= end + run {
                break;
            }
        }
        end += run;
    }
    let mask = table_size - 1;
    let step = (table_size >> 1) + (table_size >> 3) + 3;

    // Each state's bits and base, from the symbol's states in table order.
    let mut emit = |index: usize, symbol: u8| {
        let next = &mut next[symbol as usize];
        let x = u32::from(*next);
        *next += 1;
        let num_bits = acc_log - (u32::BITS - 1 - x.leading_zeros()) as u8;
        state(index, symbol, num_bits, (x << num_bits) - table_size as u32);
    };
    if high == table_size {
        // The walk skips no state, so its `k`th position is `k * step` round the table, and
        // state `u` is given the row's symbol at `u` times the inverse of `step` (odd, so
        // there is one) round the table: no walk at all.
        let inverse = inverse_mod_power_of_two(step) & mask;
        let mut k = 0;
        for index in 0..table_size {
            emit(index, row[k & (MAX_TABLE_SIZE - 1)]);
            k = (k + inverse) & mask;
        }
    } else {
        let mut symbols = [0u8; MAX_TABLE_SIZE];
        let symbols = &mut symbols[..table_size];
        let mut top = table_size;
        for (symbol, &prob) in probabilities.iter().enumerate() {
            if prob == -1 {
                top -= 1;
                symbols[top] = symbol as u8;
            }
        }
        let mut position = 0;
        for &symbol in &row[..high] {
            symbols[position] = symbol;
            position = (position + step) & mask;
            while position >= high {
                // everything from `high` up is taken
                position = (position + step) & mask;
            }
        }
        for (index, &symbol) in symbols.iter().enumerate() {
            emit(index, symbol);
        }
    }
    Ok(())
}

/// The largest accuracy log of any table zstd has (literal and match lengths).
const MAX_ACC_LOG: u8 = 9;
const MAX_TABLE_SIZE: usize = 1 << MAX_ACC_LOG;

/// This value is added to the first 4 bits of the stream to determine the
/// `Accuracy_Log`
const ACC_LOG_OFFSET: u8 = 5;

/// The `y` with `x * y == 1` round any power of two up to `1 << 64`, for an odd `x`: each step
/// of Newton's doubles the low bits that are right, from the three that `x` itself gets right.
fn inverse_mod_power_of_two(x: usize) -> usize {
    debug_assert!(x % 2 == 1);
    let mut y = x;
    for _ in 0..5 {
        y = y.wrapping_mul(2usize.wrapping_sub(x.wrapping_mul(y)));
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bit_io::BitReader;

    fn highest_bit_set(x: u32) -> u32 {
        assert!(x > 0);
        u32::BITS - x.leading_zeros()
    }

    // The table as built before, a state at a time, as a model.

    /// Calculate the position of the next entry of the table given the current
    /// position and size of the table.
    fn next_position(mut p: usize, table_size: usize) -> usize {
        p += (table_size >> 1) + (table_size >> 3) + 3;
        p &= table_size - 1;
        p
    }

    fn calc_baseline_and_numbits(
        num_states_total: u32,
        num_states_symbol: u32,
        state_number: u32,
    ) -> (u32, u8) {
        if num_states_symbol == 0 {
            return (0, 0);
        }
        let num_state_slices = if 1 << (highest_bit_set(num_states_symbol) - 1) == num_states_symbol
        {
            num_states_symbol
        } else {
            1 << (highest_bit_set(num_states_symbol))
        }; //always power of two

        let num_double_width_state_slices = num_state_slices - num_states_symbol; //leftovers to the power of two need to be distributed
        let num_single_width_state_slices = num_states_symbol - num_double_width_state_slices; //these will not receive a double width slice of states
        let slice_width = num_states_total / num_state_slices; //size of a single width slice of states
        let num_bits = highest_bit_set(slice_width) - 1; //number of bits needed to read for one slice

        if state_number < num_double_width_state_slices {
            let baseline =
                num_single_width_state_slices * slice_width + state_number * slice_width * 2;
            (baseline, num_bits as u8 + 1)
        } else {
            let index_shifted = state_number - num_double_width_state_slices;
            ((index_shifted * slice_width), num_bits as u8)
        }
    }

    fn model(acc_log: u8, probs: &[i32]) -> Vec<(u8, u8, u32)> {
        let table_size = 1usize << acc_log;
        let mut decode = alloc::vec![(0u8, 0u8, 0u32); table_size];
        let mut negative_idx = table_size;
        for (symbol, &prob) in probs.iter().enumerate() {
            if prob == -1 {
                negative_idx -= 1;
                decode[negative_idx] = (symbol as u8, acc_log, 0);
            }
        }
        let mut position = 0;
        for (symbol, &prob) in probs.iter().enumerate() {
            for _ in 0..prob.max(0) {
                decode[position].0 = symbol as u8;
                position = next_position(position, table_size);
                while position >= negative_idx {
                    position = next_position(position, table_size);
                }
            }
        }
        let mut counter = alloc::vec![0u32; probs.len()];
        for entry in &mut decode[..negative_idx] {
            let prob = probs[entry.0 as usize] as u32;
            let (bl, nb) =
                calc_baseline_and_numbits(table_size as u32, prob, counter[entry.0 as usize]);
            counter[entry.0 as usize] += 1;
            entry.1 = nb;
            entry.2 = bl;
        }
        decode
    }

    /// The table matches the model's for random distributions of every size, with and
    /// without "less than one" probabilities.
    #[test]
    fn build_matches_model() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % u64::from(n)) as u32
        };
        let mut without_less_than_one = 0;
        for round in 0..3000 {
            let acc_log = 5 + next(5) as u8;
            // Probabilities that sum to the table's size, a "less than one" counting one; every
            // other table has none, which spreads its symbols another way.
            let less_than_one = round % 2 == 0;
            let mut left = 1i32 << acc_log;
            let mut probs = Vec::new();
            while left > 0 {
                let prob = match next(6) {
                    0 => 0,
                    1 if less_than_one => -1,
                    2 => left,
                    _ => 1 + next(left as u32) as i32,
                };
                left -= prob.max(0) + i32::from(prob == -1);
                probs.push(prob);
            }
            if probs.len() > 256 {
                continue;
            }
            let mut got = alloc::vec![(0, 0, 0); 1 << acc_log];
            build_states(acc_log, &probs, |state, symbol, num_bits, base_line| {
                got[state] = (symbol, num_bits, base_line)
            })
            .unwrap();
            assert_eq!(got, model(acc_log, &probs), "{acc_log} {probs:?}");
            without_less_than_one += usize::from(!probs.contains(&-1));
        }
        assert!(without_less_than_one > 1500, "{without_less_than_one}");
    }

    /// The reader as it was, a bit reader's `get_bits` for each value; but reading on past
    /// the source's end into zeroes, and checking the end once done, since the last value may
    /// take fewer bits than are read for it.
    fn read_model(source: &[u8], max_log: u8, most: usize) -> Option<(u8, Vec<i32>, usize)> {
        let mut padded = source.to_vec();
        padded.resize(source.len() + 128, 0);
        let mut br = BitReader::new(&padded);
        let accuracy_log = ACC_LOG_OFFSET + (br.get_bits(4).ok()? as u8);
        if accuracy_log > max_log {
            return None;
        }
        let probability_sum = 1 << accuracy_log;
        let mut probability_counter = 0;
        let mut probs = Vec::new();
        while probability_counter < probability_sum {
            let max_remaining_value = probability_sum - probability_counter + 1;
            let bits_to_read = highest_bit_set(max_remaining_value);
            let unchecked_value = br.get_bits(bits_to_read as usize).ok()? as u32;
            let low_threshold = ((1 << bits_to_read) - 1) - (max_remaining_value);
            let mask = (1 << (bits_to_read - 1)) - 1;
            let small_value = unchecked_value & mask;
            let value = if small_value < low_threshold {
                br.return_bits(1);
                small_value
            } else if unchecked_value > mask {
                unchecked_value - low_threshold
            } else {
                unchecked_value
            };
            let prob = (value as i32) - 1;
            probs.push(prob);
            if prob != 0 {
                probability_counter += prob.unsigned_abs();
            } else {
                loop {
                    let skip_amount = br.get_bits(2).ok()? as usize;
                    probs.resize(probs.len() + skip_amount, 0);
                    if skip_amount != 3 {
                        break;
                    }
                }
            }
            if probs.len() > most {
                return None;
            }
        }
        if source.is_empty() || br.bits_read() > source.len() * 8 {
            return None;
        }
        Some((accuracy_log, probs, br.bits_read().div_ceil(8)))
    }

    /// Random descriptions, and real ones cut short, read as the old reader read them.
    #[test]
    fn read_matches_model() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut read = 0;
        for round in 0..200_000 {
            let len = (next() % 80) as usize;
            let mut source: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            // Small accuracy logs, as the sequences' tables have, and runs of zero bytes,
            // which are runs of zero probabilities.
            if let Some(first) = source.first_mut() {
                *first &= if round % 2 == 0 { 0xF3 } else { 0xFF };
            }
            if round % 3 == 0 {
                for byte in source.iter_mut().skip(1).step_by(3) {
                    *byte = 0;
                }
            }
            let (max_log, most) = [(9, 36), (9, 53), (8, 32), (6, 256)][round % 4];
            let mut table = FSETable::new((most - 1) as u8);
            let got = table
                .read_probabilities(&source, max_log)
                .ok()
                .map(|bytes| {
                    (
                        table.accuracy_log,
                        table.symbol_probabilities.clone(),
                        bytes,
                    )
                });
            assert_eq!(got, read_model(&source, max_log, most), "{source:?}");
            read += usize::from(got.is_some());
        }
        assert!(read > 20_000, "{read}");
    }

    /// The inverse of any odd number, round every table size and more.
    #[test]
    fn inverse_of_odd_numbers() {
        for x in (1..20_000usize).step_by(2) {
            let y = inverse_mod_power_of_two(x);
            assert_eq!(x.wrapping_mul(y), 1, "{x}");
        }
        for acc_log in 5..=MAX_ACC_LOG {
            let size = 1usize << acc_log;
            let step = (size >> 1) + (size >> 3) + 3;
            assert_eq!(
                (step * (inverse_mod_power_of_two(step) & (size - 1))) % size,
                1
            );
        }
    }
}
