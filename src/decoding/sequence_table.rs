//! The decoding tables of the sequences section, laid out as libzstd lays them out: each
//! state holds what decoding it needs, the code's value and extra bits as well as the next
//! state's, so a sequence is three lookups and no translation of codes.

use crate::decoding::errors::{DecodeSequenceError, FSEDecoderError};
use crate::fse::FSETable;
use alloc::vec::Vec;

/// One state of a sequence decoding table.
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct SeqEntry {
    /// The code's value, before its extra bits are added.
    pub base_value: u32,
    /// The next state, before the `nb_bits` bits read for it are added.
    pub next_state: u16,
    /// How many extra bits the code's value has.
    pub nb_add_bits: u8,
    /// How many bits are read for the next state.
    pub nb_bits: u8,
}

/// Room for the largest table, `1 << 9` states (literal and match lengths; offsets have at most
/// `1 << 8`). A power of two, so that a state masked by `SIZE - 1` is in bounds, and indexing
/// needs no check.
pub(crate) const SIZE: usize = 1 << 9;

/// What the codes of one of the three tables stand for.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SeqKind {
    LiteralLength,
    MatchLength,
    Offset,
}

impl SeqKind {
    /// The value of `code` before its extra bits are added, and how many extra bits it has.
    /// `code` is at most the kind's largest, which the tables are built to hold to.
    fn code_value(self, code: u8) -> (u32, u8) {
        let code = code as usize;
        match self {
            SeqKind::LiteralLength => (LL_BASE[code], LL_BITS[code]),
            SeqKind::MatchLength => (ML_BASE[code], ML_BITS[code]),
            SeqKind::Offset => (1 << code, code as u8),
        }
    }
}

/// A sequence decoding table: an FSE table's states, or an RLE code as a table of one state.
#[derive(Clone)]
pub struct SeqTable {
    /// Empty until a table is built, so that a new decoder allocates nothing it may not use,
    /// then always `SIZE` long.
    entries: Vec<SeqEntry>,
    /// How many bits the first state is read from; `None` until a table is built, since a block
    /// that repeats the previous table needs there to be one.
    accuracy_log: Option<u8>,
    /// The entries are the predefined distribution's states, which a later block asking for
    /// them, in this frame or a later one, can keep: a reset leaves them.
    holds_predefined: bool,
}

impl SeqTable {
    pub fn new() -> SeqTable {
        SeqTable {
            entries: Vec::new(),
            accuracy_log: None,
            holds_predefined: false,
        }
    }

    pub fn reset(&mut self) {
        self.accuracy_log = None;
    }

    pub fn reinit_from(&mut self, other: &SeqTable) {
        self.entries.clear();
        self.entries.extend_from_slice(&other.entries);
        self.accuracy_log = other.accuracy_log;
        self.holds_predefined = other.holds_predefined;
    }

    /// Room for every state, made on the first build.
    fn sized(&mut self) {
        if self.entries.len() != SIZE {
            self.entries.resize(SIZE, SeqEntry::default());
        }
    }

    /// Take the states of `fse`, a table of `kind`'s codes.
    pub fn build_from_fse(&mut self, fse: &FSETable, kind: SeqKind) {
        self.sized();
        for (entry, state) in self.entries.iter_mut().zip(&fse.decode) {
            let (base_value, nb_add_bits) = kind.code_value(state.symbol);
            *entry = SeqEntry {
                base_value,
                next_state: state.base_line as u16,
                nb_add_bits,
                nb_bits: state.num_bits,
            };
        }
        self.accuracy_log = Some(fse.accuracy_log);
        self.holds_predefined = false;
    }

    /// Take the predefined distribution's states, unless the table already holds them.
    pub fn build_predefined(
        &mut self,
        fse: &mut FSETable,
        kind: SeqKind,
    ) -> Result<(), crate::decoding::errors::FSETableError> {
        let (acc_log, distribution): (u8, &[i32]) = match kind {
            SeqKind::LiteralLength => (LL_DEFAULT_ACC_LOG, &LL_DEFAULT_DISTRIBUTION),
            SeqKind::MatchLength => (ML_DEFAULT_ACC_LOG, &ML_DEFAULT_DISTRIBUTION),
            SeqKind::Offset => (OF_DEFAULT_ACC_LOG, &OF_DEFAULT_DISTRIBUTION),
        };
        if !self.holds_predefined {
            fse.build_from_probabilities(acc_log, distribution)?;
            self.build_from_fse(fse, kind);
            self.holds_predefined = true;
        }
        self.accuracy_log = Some(acc_log);
        Ok(())
    }

    /// Every state is `code`, and no bits are read for the next one.
    pub fn build_rle(&mut self, code: u8, kind: SeqKind) {
        let (base_value, nb_add_bits) = kind.code_value(code);
        self.sized();
        self.entries[0] = SeqEntry {
            base_value,
            next_state: 0,
            nb_add_bits,
            nb_bits: 0,
        };
        self.accuracy_log = Some(0);
        self.holds_predefined = false;
    }

    /// How many bits the first state is read from, and the states, as an array: a state masked
    /// by `SIZE - 1` indexes it with no bounds check (see [`at`]).
    pub fn states(&self) -> Result<(u8, &[SeqEntry; SIZE]), DecodeSequenceError> {
        let uninitialized =
            DecodeSequenceError::FSEDecoderError(FSEDecoderError::TableIsUninitialized);
        let accuracy_log = self.accuracy_log.ok_or(uninitialized)?;
        let states = self.entries.as_slice().try_into().map_err(|_| {
            DecodeSequenceError::FSEDecoderError(FSEDecoderError::TableIsUninitialized)
        })?;
        Ok((accuracy_log, states))
    }
}

/// The entry of `state` in a table's states (see [`SeqTable::states`]).
#[inline(always)]
pub fn at(states: &[SeqEntry; SIZE], state: usize) -> SeqEntry {
    states[state & (SIZE - 1)]
}

impl Default for SeqTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Literal length codes' values and extra bits.
///
/// <https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md#literals-length-codes>
const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64,
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];
const LL_BITS: [u8; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];

/// Match length codes' values and extra bits.
///
/// <https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md#match-length-codes>
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027,
    2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

/// The predefined distributions, used when a block's mode is [ModeType::Predefined].
///
/// [ModeType::Predefined]: crate::blocks::sequence_section::ModeType::Predefined
/// <https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md#default-distributions>
pub(crate) const LL_DEFAULT_ACC_LOG: u8 = 6;
pub(crate) const LL_DEFAULT_DISTRIBUTION: [i32; 36] = [
    4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1,
    -1, -1, -1, -1,
];
const ML_DEFAULT_ACC_LOG: u8 = 6;
const ML_DEFAULT_DISTRIBUTION: [i32; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
const OF_DEFAULT_ACC_LOG: u8 = 5;
const OF_DEFAULT_DISTRIBUTION: [i32; 29] = [
    1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The value tables are the ones the specification lists, code by code.
    #[test]
    fn code_values_match_the_specification() {
        let ll = |code: u8| -> (u32, u8) {
            match code {
                0..=15 => (u32::from(code), 0),
                16 => (16, 1),
                17 => (18, 1),
                18 => (20, 1),
                19 => (22, 1),
                20 => (24, 2),
                21 => (28, 2),
                22 => (32, 3),
                23 => (40, 3),
                24 => (48, 4),
                25 => (64, 6),
                26..=35 => (1 << (code - 19), code - 19),
                _ => unreachable!(),
            }
        };
        for code in 0..36 {
            assert_eq!(
                SeqKind::LiteralLength.code_value(code),
                ll(code),
                "ll {code}"
            );
        }
        let ml = |code: u8| -> (u32, u8) {
            match code {
                0..=31 => (u32::from(code) + 3, 0),
                32 => (35, 1),
                33 => (37, 1),
                34 => (39, 1),
                35 => (41, 1),
                36 => (43, 2),
                37 => (47, 2),
                38 => (51, 3),
                39 => (59, 3),
                40 => (67, 4),
                41 => (83, 4),
                42 => (99, 5),
                43..=52 => ((1 << (code - 36)) + 3, code - 36),
                _ => unreachable!(),
            }
        };
        for code in 0..53 {
            assert_eq!(SeqKind::MatchLength.code_value(code), ml(code), "ml {code}");
        }
    }

    /// A table built from an FSE table has its states, in order, with each code's value.
    #[test]
    fn built_from_fse_states() {
        let mut fse = FSETable::new(crate::blocks::sequence_section::MAX_LITERAL_LENGTH_CODE);
        let mut table = SeqTable::new();
        assert!(table.states().is_err());
        table
            .build_predefined(&mut fse, SeqKind::LiteralLength)
            .unwrap();
        assert_eq!(table.states().unwrap().0, LL_DEFAULT_ACC_LOG);
        for (state, entry) in fse.decode.iter().enumerate() {
            let (base_value, nb_add_bits) = SeqKind::LiteralLength.code_value(entry.symbol);
            assert_eq!(
                at(table.states().unwrap().1, state),
                SeqEntry {
                    base_value,
                    next_state: entry.base_line as u16,
                    nb_add_bits,
                    nb_bits: entry.num_bits,
                }
            );
        }
        // Kept across a reset, as a new frame does, then taken again without a build.
        let states: alloc::vec::Vec<SeqEntry> = (0..64)
            .map(|state| at(table.states().unwrap().1, state))
            .collect();
        table.reset();
        assert!(table.states().is_err());
        fse.reset();
        table
            .build_predefined(&mut fse, SeqKind::LiteralLength)
            .unwrap();
        assert!(fse.decode.is_empty(), "rebuilt");
        assert_eq!(table.states().unwrap().0, LL_DEFAULT_ACC_LOG);
        assert!(
            (0..64)
                .map(|state| at(table.states().unwrap().1, state))
                .eq(states.iter().copied())
        );

        table.build_rle(20, SeqKind::LiteralLength);
        assert_eq!(table.states().unwrap().0, 0);
        assert_eq!(
            at(table.states().unwrap().1, 0),
            SeqEntry {
                base_value: 24,
                next_state: 0,
                nb_add_bits: 2,
                nb_bits: 0
            }
        );
        // An RLE table is not the predefined one any more.
        table
            .build_predefined(&mut fse, SeqKind::LiteralLength)
            .unwrap();
        assert!(
            (0..64)
                .map(|state| at(table.states().unwrap().1, state))
                .eq(states.iter().copied())
        );
    }
}
