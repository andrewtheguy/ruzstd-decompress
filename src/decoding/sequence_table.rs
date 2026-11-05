//! The decoding tables of the sequences section, laid out as libzstd lays them out: each
//! state holds what decoding it needs, the code's value and extra bits as well as the next
//! state's, so a sequence is three lookups and no translation of codes.

use crate::decoding::errors::{DecodeSequenceError, FSEDecoderError, FSETableError};
use crate::fse::{FSETable, build_states};
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

/// Room for every kind's codes (at most 53, the match lengths'), a power of two, so that a code
/// masked by `CODES - 1` indexes the values with no bounds check.
const CODES: usize = 64;

impl SeqKind {
    /// Each code's value before its extra bits are added, and how many extra bits it has; the
    /// codes past the kind's largest are 0.
    fn values(self) -> (&'static [u32; CODES], &'static [u8; CODES]) {
        match self {
            SeqKind::LiteralLength => (&LL_BASE, &LL_BITS),
            SeqKind::MatchLength => (&ML_BASE, &ML_BITS),
            SeqKind::Offset => (&OF_BASE, &OF_BITS),
        }
    }

    /// How many codes the kind has.
    fn codes(self) -> usize {
        match self {
            SeqKind::LiteralLength => LL_CODES,
            SeqKind::MatchLength => ML_CODES,
            SeqKind::Offset => OF_CODES,
        }
    }

    /// The value of `code` before its extra bits are added, and how many extra bits it has.
    /// `code` is at most the kind's largest.
    fn code_value(self, code: u8) -> (u32, u8) {
        let (base, bits) = self.values();
        let code = code as usize & (CODES - 1);
        (base[code], bits[code])
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
    /// The dictionary whose table the entries are, as copied, kept through resets as
    /// `holds_predefined` is, so that a frame using it again need not copy it in (see
    /// [`Self::reinit_from_dict`]); `None` once anything else may have been built in them.
    dict: Option<u32>,
}

impl SeqTable {
    pub fn new() -> SeqTable {
        SeqTable {
            entries: Vec::new(),
            accuracy_log: None,
            holds_predefined: false,
            dict: None,
        }
    }

    pub fn reset(&mut self) {
        self.accuracy_log = None;
    }

    /// Take `other`'s states, the table of dictionary `dict_id`: copied only if this table no
    /// longer holds them from an earlier frame.
    pub fn reinit_from_dict(&mut self, other: &SeqTable, dict_id: u32) {
        if self.dict != Some(dict_id) {
            self.entries.clear();
            self.entries.extend_from_slice(&other.entries);
            self.dict = Some(dict_id);
        }
        self.accuracy_log = other.accuracy_log;
        self.holds_predefined = other.holds_predefined;
    }

    /// Forget which dictionary's table this one holds, so that the next frame using one copies
    /// it in: for when a dictionary of the same id may have replaced it.
    pub fn forget_dict(&mut self) {
        self.dict = None;
    }

    /// Room for every state, made on the first build.
    fn sized(&mut self) {
        if self.entries.len() != SIZE {
            self.entries.resize(SIZE, SeqEntry::default());
        }
    }

    /// Build the table of `fse`'s distribution (read with [`FSETable::read_distribution`]), a
    /// table of `kind`'s codes.
    pub fn build_from_fse(&mut self, fse: &FSETable, kind: SeqKind) -> Result<(), FSETableError> {
        self.build(fse.accuracy_log, &fse.symbol_probabilities, kind)
    }

    /// Take the predefined distribution's states, unless the table already holds them.
    pub fn build_predefined(&mut self, kind: SeqKind) -> Result<(), FSETableError> {
        let (acc_log, distribution): (u8, &[i32]) = match kind {
            SeqKind::LiteralLength => (LL_DEFAULT_ACC_LOG, &LL_DEFAULT_DISTRIBUTION),
            SeqKind::MatchLength => (ML_DEFAULT_ACC_LOG, &ML_DEFAULT_DISTRIBUTION),
            SeqKind::Offset => (OF_DEFAULT_ACC_LOG, &OF_DEFAULT_DISTRIBUTION),
        };
        if !self.holds_predefined {
            self.build(acc_log, distribution, kind)?;
            self.holds_predefined = true;
        }
        self.accuracy_log = Some(acc_log);
        Ok(())
    }

    /// The states of a distribution of `kind`'s codes, each with its code's value, built
    /// straight from the probabilities as libzstd builds them (`ZSTD_buildFSETable`).
    fn build(
        &mut self,
        acc_log: u8,
        probabilities: &[i32],
        kind: SeqKind,
    ) -> Result<(), FSETableError> {
        self.accuracy_log = None;
        self.holds_predefined = false;
        self.dict = None;
        if probabilities.len() > kind.codes() {
            return Err(FSETableError::TooManySymbols {
                got: probabilities.len(),
            });
        }
        let (base, bits) = kind.values();
        self.sized();
        let entries: &mut [SeqEntry; SIZE] = self
            .entries
            .as_mut_slice()
            .try_into()
            .expect("sized to SIZE");
        build_states(
            acc_log,
            probabilities,
            |state, code, nb_bits, next_state| {
                let code = code as usize & (CODES - 1);
                entries[state & (SIZE - 1)] = SeqEntry {
                    base_value: base[code],
                    next_state: next_state as u16,
                    nb_add_bits: bits[code],
                    nb_bits,
                };
            },
        )?;
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
        self.dict = None;
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
const LL_CODES: usize = 36;
const LL_BASE: [u32; CODES] = padded(LL_BASE_LISTED);
const LL_BITS: [u8; CODES] = padded_bits(LL_BITS_LISTED);
const LL_BASE_LISTED: [u32; LL_CODES] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64,
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];
const LL_BITS_LISTED: [u8; LL_CODES] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];

/// Match length codes' values and extra bits.
///
/// <https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md#match-length-codes>
const ML_CODES: usize = 53;
const ML_BASE: [u32; CODES] = padded(ML_BASE_LISTED);
const ML_BITS: [u8; CODES] = padded_bits(ML_BITS_LISTED);
const ML_BASE_LISTED: [u32; ML_CODES] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027,
    2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS_LISTED: [u8; ML_CODES] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

/// Offset codes' values and extra bits: code `n` is `1 << n` with `n` extra bits.
const OF_CODES: usize = 32;
const OF_BASE: [u32; CODES] = {
    let mut base = [0; CODES];
    let mut code = 0;
    while code < OF_CODES {
        base[code] = 1 << code;
        code += 1;
    }
    base
};
const OF_BITS: [u8; CODES] = {
    let mut bits = [0; CODES];
    let mut code = 0;
    while code < OF_CODES {
        bits[code] = code as u8;
        code += 1;
    }
    bits
};

/// A kind's listed values, followed by zeros up to [`CODES`].
const fn padded<const N: usize>(listed: [u32; N]) -> [u32; CODES] {
    let mut out = [0; CODES];
    let mut code = 0;
    while code < N {
        out[code] = listed[code];
        code += 1;
    }
    out
}

/// [`padded`], for the extra bits.
const fn padded_bits<const N: usize>(listed: [u8; N]) -> [u8; CODES] {
    let mut out = [0; CODES];
    let mut code = 0;
    while code < N {
        out[code] = listed[code];
        code += 1;
    }
    out
}

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

    /// A table built from a distribution has the FSE table's states, in order, with each
    /// code's value.
    #[test]
    fn built_from_fse_states() {
        let mut fse = FSETable::new(crate::blocks::sequence_section::MAX_LITERAL_LENGTH_CODE);
        fse.build_from_probabilities(LL_DEFAULT_ACC_LOG, &LL_DEFAULT_DISTRIBUTION)
            .unwrap();
        let expected: alloc::vec::Vec<SeqEntry> = fse
            .decode
            .iter()
            .map(|entry| {
                let (base_value, nb_add_bits) = SeqKind::LiteralLength.code_value(entry.symbol);
                SeqEntry {
                    base_value,
                    next_state: entry.base_line as u16,
                    nb_add_bits,
                    nb_bits: entry.num_bits,
                }
            })
            .collect();
        let states = |table: &SeqTable| -> alloc::vec::Vec<SeqEntry> {
            (0..64)
                .map(|state| at(table.states().unwrap().1, state))
                .collect()
        };

        let mut table = SeqTable::new();
        assert!(table.states().is_err());
        table.build_from_fse(&fse, SeqKind::LiteralLength).unwrap();
        assert_eq!(table.states().unwrap().0, LL_DEFAULT_ACC_LOG);
        assert_eq!(states(&table), expected);

        // The predefined table is the same, kept across a reset, as a new frame does, and
        // taken again without a build.
        let mut table = SeqTable::new();
        table.build_predefined(SeqKind::LiteralLength).unwrap();
        assert_eq!(states(&table), expected);
        table.reset();
        assert!(table.states().is_err());
        table.entries[0] = SeqEntry::default();
        table.build_predefined(SeqKind::LiteralLength).unwrap();
        assert_eq!(table.states().unwrap().0, LL_DEFAULT_ACC_LOG);
        assert_eq!(states(&table)[0], SeqEntry::default(), "not rebuilt");

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
        table.build_predefined(SeqKind::LiteralLength).unwrap();
        assert_eq!(states(&table), expected);
    }
}
