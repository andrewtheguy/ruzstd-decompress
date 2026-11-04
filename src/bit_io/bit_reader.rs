/// Wraps a slice and enables reading arbitrary amounts of bits
/// from that slice.
pub struct BitReader<'s> {
    idx: usize, //index counts bits already read
    source: &'s [u8],
}

impl<'s> BitReader<'s> {
    pub fn new(source: &'s [u8]) -> BitReader<'s> {
        BitReader { idx: 0, source }
    }

    pub fn bits_left(&self) -> usize {
        self.source.len() * 8 - self.idx
    }

    pub fn bits_read(&self) -> usize {
        self.idx
    }

    pub fn return_bits(&mut self, n: usize) {
        if n > self.idx {
            panic!("Cant return this many bits");
        }
        self.idx -= n;
    }

    /// Read `n` bits, at most 64, the first bit read the lowest.
    #[inline]
    pub fn get_bits(&mut self, n: usize) -> Result<u64, GetBitsError> {
        if n > 56 {
            return self.get_bits_wide(n);
        }
        if self.bits_left() < n {
            return Err(GetBitsError::NotEnoughRemainingBits {
                requested: n,
                remaining: self.bits_left(),
            });
        }
        // The 8 bytes from the current one, which hold all `n` bits, short of the source's end
        // padded with zeroes.
        let byte = self.idx / 8;
        let window = match self.source.get(byte..byte + 8) {
            Some(bytes) => u64::from_le_bytes(bytes.try_into().unwrap()),
            None => {
                let rest = &self.source[byte..];
                let mut bytes = [0; 8];
                bytes[..rest.len()].copy_from_slice(rest);
                u64::from_le_bytes(bytes)
            }
        };
        let value = (window >> (self.idx % 8)) & ((1 << n) - 1);
        self.idx += n;
        Ok(value)
    }

    /// [`Self::get_bits`] for more than 56 bits, which 8 bytes may not hold: in two reads.
    #[cold]
    fn get_bits_wide(&mut self, n: usize) -> Result<u64, GetBitsError> {
        if n > 64 {
            return Err(GetBitsError::TooManyBits {
                num_requested_bits: n,
                limit: 64,
            });
        }
        if self.bits_left() < n {
            return Err(GetBitsError::NotEnoughRemainingBits {
                requested: n,
                remaining: self.bits_left(),
            });
        }
        let low = self.get_bits(32)?;
        let high = self.get_bits(n - 32)?;
        Ok(low | (high << 32))
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GetBitsError {
    TooManyBits {
        num_requested_bits: usize,
        limit: u8,
    },
    NotEnoughRemainingBits {
        requested: usize,
        remaining: usize,
    },
}

#[cfg(feature = "std")]
impl std::error::Error for GetBitsError {}

impl core::fmt::Display for GetBitsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GetBitsError::TooManyBits {
                num_requested_bits,
                limit,
            } => {
                write!(
                    f,
                    "Cant serve this request. The reader is limited to {limit} bits, requested {num_requested_bits} bits"
                )
            }
            GetBitsError::NotEnoughRemainingBits {
                requested,
                remaining,
            } => {
                write!(
                    f,
                    "Can\'t read {requested} bits, only have {remaining} bits left"
                )
            }
        }
    }
}
