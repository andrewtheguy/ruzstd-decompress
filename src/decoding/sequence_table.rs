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
/// `1 << 8`): the size of each region of [`SeqTables`].
pub(crate) const SIZE: usize = 1 << 9;

/// [`SeqTables`]' entries: four regions of [`SIZE`]. A power of two, so that a state masked by
/// `ALL - 1` is in bounds, and indexing needs no check.
pub(crate) const ALL: usize = 4 * SIZE;

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

/// The three kinds' sequence decoding tables in one allocation, so that the loop over the
/// sequences finds all three from one base: a state is an index into it, and each entry's next
/// state is one too.
///
/// Each kind has tables of its own: one built from a description, a block's or a
/// dictionary's, or copied from a dictionary, in a region of [`SIZE`] states of its own; the
/// predefined one; and the RLE one, an RLE code as a table of one state. The predefined and RLE
/// tables of all three kinds share one more region (see [`Slot::start`]). A block using the
/// predefined table or an RLE code then leaves the dictionary's table where it was, for the
/// frames after it.
#[derive(Clone)]
pub struct SeqTables {
    /// Empty until a table is built, so that a new decoder allocates nothing it may not use,
    /// then always [`ALL`] long.
    entries: Vec<SeqEntry>,
    /// Each kind's table in use and what it keeps, by `SeqKind as usize`.
    kinds: [KindTables; 3],
}

/// What [`SeqTables`] keeps of one kind's tables.
#[derive(Copy, Clone)]
struct KindTables {
    /// The table in use.
    slot: Slot,
    /// How many bits the first state is read from; `None` until a table is built, since a block
    /// that repeats the previous table needs there to be one.
    accuracy_log: Option<u8>,
    /// The predefined table holds the predefined distribution's states, which a later block
    /// asking for them, in this frame or a later one, can keep: a reset leaves them.
    holds_predefined: bool,
    /// The dictionary whose table the built one is, as copied, kept through resets as
    /// `holds_predefined` is, so that a frame using it again need not copy it in (see
    /// [`SeqTables::reinit_from_dict`]); `None` once anything else may have been built there.
    dict: Option<u32>,
}

/// Which of a kind's tables is in use.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Slot {
    Built,
    Predefined,
    Rle,
}

impl Slot {
    /// Where `kind`'s table of this slot starts in [`SeqTables`]' entries: the built ones each in
    /// a region of [`SIZE`], then the predefined ones (64, 64 and 32 states) and the RLE ones
    /// (one each) in the last region.
    fn start(self, kind: SeqKind) -> usize {
        let k = kind as usize;
        match self {
            Slot::Built => k * SIZE,
            Slot::Predefined => 3 * SIZE + k * 64,
            Slot::Rle => 3 * SIZE + 160 + k,
        }
    }
}

const KINDS: [SeqKind; 3] = [
    SeqKind::LiteralLength,
    SeqKind::MatchLength,
    SeqKind::Offset,
];

impl SeqTables {
    pub fn new() -> SeqTables {
        let kind = KindTables {
            slot: Slot::Built,
            accuracy_log: None,
            holds_predefined: false,
            dict: None,
        };
        SeqTables {
            entries: Vec::new(),
            kinds: [kind; 3],
        }
    }

    pub fn reset(&mut self) {
        for kind in &mut self.kinds {
            kind.accuracy_log = None;
        }
    }

    /// Take `other`'s tables, dictionary `dict_id`'s: each copied only if this one no longer
    /// holds it from an earlier frame.
    pub fn reinit_from_dict(&mut self, other: &SeqTables, dict_id: u32) {
        for kind in KINDS {
            let k = kind as usize;
            self.kinds[k].slot = Slot::Built;
            self.kinds[k].accuracy_log = other.kinds[k].accuracy_log;
            if self.kinds[k].dict != Some(dict_id) {
                self.kinds[k].dict = Some(dict_id);
                if let Some(from) = other.entries.get(k * SIZE..(k + 1) * SIZE) {
                    self.entries_mut()[k * SIZE..(k + 1) * SIZE].copy_from_slice(from);
                }
            }
        }
    }

    /// Forget which dictionary's tables these hold, so that the next frame using one copies
    /// them in: for when a dictionary of the same id may have replaced it.
    pub fn forget_dict(&mut self) {
        for kind in &mut self.kinds {
            kind.dict = None;
        }
    }

    /// The entries, with room for every table, made on the first build.
    fn entries_mut(&mut self) -> &mut [SeqEntry; ALL] {
        if self.entries.len() != ALL {
            self.entries.resize(ALL, SeqEntry::default());
        }
        self.entries
            .as_mut_slice()
            .try_into()
            .expect("sized to ALL")
    }

    /// Build `kind`'s table of `fse`'s distribution (read with [`FSETable::read_distribution`]).
    pub fn build_from_fse(&mut self, fse: &FSETable, kind: SeqKind) -> Result<(), FSETableError> {
        self.kinds[kind as usize].dict = None;
        self.build(
            Slot::Built,
            fse.accuracy_log,
            &fse.symbol_probabilities,
            kind,
        )
    }

    /// Take `kind`'s predefined distribution's states, unless they are held already.
    pub fn build_predefined(&mut self, kind: SeqKind) -> Result<(), FSETableError> {
        let (acc_log, distribution): (u8, &[i32]) = match kind {
            SeqKind::LiteralLength => (LL_DEFAULT_ACC_LOG, &LL_DEFAULT_DISTRIBUTION),
            SeqKind::MatchLength => (ML_DEFAULT_ACC_LOG, &ML_DEFAULT_DISTRIBUTION),
            SeqKind::Offset => (OF_DEFAULT_ACC_LOG, &OF_DEFAULT_DISTRIBUTION),
        };
        let table = &mut self.kinds[kind as usize];
        if table.holds_predefined {
            table.slot = Slot::Predefined;
        } else {
            self.build(Slot::Predefined, acc_log, distribution, kind)?;
            self.kinds[kind as usize].holds_predefined = true;
        }
        self.kinds[kind as usize].accuracy_log = Some(acc_log);
        Ok(())
    }

    /// The states of a distribution of `kind`'s codes in its table of `slot`, each with its
    /// code's value, built straight from the probabilities as libzstd builds them
    /// (`ZSTD_buildFSETable`).
    fn build(
        &mut self,
        slot: Slot,
        acc_log: u8,
        probabilities: &[i32],
        kind: SeqKind,
    ) -> Result<(), FSETableError> {
        let table = &mut self.kinds[kind as usize];
        table.accuracy_log = None;
        table.slot = slot;
        if probabilities.len() > kind.codes() {
            return Err(FSETableError::TooManySymbols {
                got: probabilities.len(),
            });
        }
        let (base, bits) = kind.values();
        let start = slot.start(kind);
        let entries = self.entries_mut();
        build_states(
            acc_log,
            probabilities,
            |state, code, nb_bits, next_state| {
                let code = code as usize & (CODES - 1);
                entries[(start + state) & (ALL - 1)] = SeqEntry {
                    base_value: base[code],
                    next_state: (start + next_state as usize) as u16,
                    nb_add_bits: bits[code],
                    nb_bits,
                };
            },
        )?;
        self.kinds[kind as usize].accuracy_log = Some(acc_log);
        Ok(())
    }

    /// Every state of `kind`'s is `code`, and no bits are read for the next one.
    pub fn build_rle(&mut self, code: u8, kind: SeqKind) {
        let (base_value, nb_add_bits) = kind.code_value(code);
        let start = Slot::Rle.start(kind);
        self.entries_mut()[start] = SeqEntry {
            base_value,
            next_state: start as u16,
            nb_add_bits,
            nb_bits: 0,
        };
        let table = &mut self.kinds[kind as usize];
        table.slot = Slot::Rle;
        table.accuracy_log = Some(0);
    }

    /// How many bits `kind`'s first state is read from, and where its table in use starts: the
    /// first state is that start plus the bits read.
    pub fn first_state(&self, kind: SeqKind) -> Result<(u8, usize), DecodeSequenceError> {
        let table = &self.kinds[kind as usize];
        let accuracy_log = table
            .accuracy_log
            .ok_or(DecodeSequenceError::FSEDecoderError(
                FSEDecoderError::TableIsUninitialized,
            ))?;
        Ok((accuracy_log, table.slot.start(kind)))
    }

    /// The entries, as an array: a state masked by `ALL - 1` indexes it with no bounds check
    /// (see [`at`]).
    pub fn entries(&self) -> Result<&[SeqEntry; ALL], DecodeSequenceError> {
        self.entries.as_slice().try_into().map_err(|_| {
            DecodeSequenceError::FSEDecoderError(FSEDecoderError::TableIsUninitialized)
        })
    }
}

/// The entry of `state` in the tables' entries (see [`SeqTables::entries`]).
#[inline(always)]
pub fn at(entries: &[SeqEntry; ALL], state: usize) -> SeqEntry {
    entries[state & (ALL - 1)]
}

impl Default for SeqTables {
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
    /// code's value, its next states indices into the shared entries.
    #[test]
    fn built_from_fse_states() {
        const LL: SeqKind = SeqKind::LiteralLength;
        let mut fse = FSETable::new(crate::blocks::sequence_section::MAX_LITERAL_LENGTH_CODE);
        fse.accuracy_log = LL_DEFAULT_ACC_LOG;
        fse.symbol_probabilities = LL_DEFAULT_DISTRIBUTION.to_vec();
        // The states as the FSE table has them, from 0, which `states` gives back.
        let mut expected = alloc::vec![SeqEntry::default(); 1 << LL_DEFAULT_ACC_LOG];
        build_states(
            LL_DEFAULT_ACC_LOG,
            &LL_DEFAULT_DISTRIBUTION,
            |state, symbol, num_bits, base_line| {
                let (base_value, nb_add_bits) = LL.code_value(symbol);
                expected[state] = SeqEntry {
                    base_value,
                    next_state: base_line as u16,
                    nb_add_bits,
                    nb_bits: num_bits,
                };
            },
        )
        .unwrap();
        let states = |tables: &SeqTables| -> alloc::vec::Vec<SeqEntry> {
            let (_, start) = tables.first_state(LL).unwrap();
            let entries = tables.entries().unwrap();
            (0..64)
                .map(|state| {
                    let entry = at(entries, start + state);
                    SeqEntry {
                        next_state: entry.next_state - start as u16,
                        ..entry
                    }
                })
                .collect()
        };

        let mut tables = SeqTables::new();
        assert!(tables.first_state(LL).is_err());
        assert!(tables.entries().is_err());
        tables.build_from_fse(&fse, LL).unwrap();
        assert_eq!(tables.first_state(LL).unwrap(), (LL_DEFAULT_ACC_LOG, 0));
        assert_eq!(states(&tables), expected);
        // The other kinds' tables are their own.
        assert!(tables.first_state(SeqKind::Offset).is_err());
        tables.build_rle(3, SeqKind::Offset);
        tables.build_predefined(SeqKind::MatchLength).unwrap();
        assert_eq!(states(&tables), expected);

        // The predefined table is the same, kept across a reset, as a new frame does, and
        // taken again without a build.
        let mut tables = SeqTables::new();
        tables.build_predefined(LL).unwrap();
        let predefined = Slot::Predefined.start(LL);
        assert_eq!(
            tables.first_state(LL).unwrap(),
            (LL_DEFAULT_ACC_LOG, predefined)
        );
        assert_eq!(states(&tables), expected);
        tables.reset();
        assert!(tables.first_state(LL).is_err());
        tables.entries[predefined] = SeqEntry::default();
        tables.build_predefined(LL).unwrap();
        assert_eq!(tables.first_state(LL).unwrap().0, LL_DEFAULT_ACC_LOG);
        assert_eq!(
            tables.entries().unwrap()[predefined],
            SeqEntry::default(),
            "not rebuilt"
        );
        tables.entries[predefined] = SeqEntry {
            next_state: expected[0].next_state + predefined as u16,
            ..expected[0]
        };

        tables.build_rle(20, LL);
        let (log, start) = tables.first_state(LL).unwrap();
        assert_eq!(log, 0);
        assert_eq!(
            at(tables.entries().unwrap(), start),
            SeqEntry {
                base_value: 24,
                next_state: start as u16,
                nb_add_bits: 2,
                nb_bits: 0
            }
        );
        // An RLE table leaves the predefined one, and either leaves a dictionary's.
        tables.build_predefined(LL).unwrap();
        assert_eq!(states(&tables), expected);

        let mut dict = SeqTables::new();
        let mut other = fse.clone();
        other.symbol_probabilities[0] += 1;
        other.symbol_probabilities[1] -= 1;
        dict.build_from_fse(&other, LL).unwrap();
        let dict_states = states(&dict);
        assert_ne!(dict_states, expected);
        tables.reinit_from_dict(&dict, 7);
        assert_eq!(states(&tables), dict_states);
        tables.build_predefined(LL).unwrap();
        assert_eq!(states(&tables), expected);
        tables.build_rle(20, LL);
        tables.reset();
        // The next frame takes the dictionary's again with no copy, and then one built from a
        // block's description replaces it.
        tables.entries[Slot::Built.start(LL)] = SeqEntry::default();
        tables.reinit_from_dict(&dict, 7);
        assert_eq!(states(&tables)[0], SeqEntry::default(), "not copied");
        tables.build_from_fse(&fse, LL).unwrap();
        assert_eq!(states(&tables), expected);
        tables.reinit_from_dict(&dict, 7);
        assert_eq!(states(&tables), dict_states);
    }

    /// Each kind's tables lie in their own part of the entries.
    #[test]
    fn regions_do_not_overlap() {
        let mut taken = [false; ALL];
        for kind in KINDS {
            let sizes = [(Slot::Built, SIZE), (Slot::Rle, 1)];
            let predefined = match kind {
                SeqKind::Offset => 1 << OF_DEFAULT_ACC_LOG,
                SeqKind::LiteralLength => 1 << LL_DEFAULT_ACC_LOG,
                SeqKind::MatchLength => 1 << ML_DEFAULT_ACC_LOG,
            };
            for (slot, size) in sizes.into_iter().chain([(Slot::Predefined, predefined)]) {
                let start = slot.start(kind);
                for state in &mut taken[start..start + size] {
                    assert!(!*state, "{kind:?} {slot:?}");
                    *state = true;
                }
            }
        }
    }
}
