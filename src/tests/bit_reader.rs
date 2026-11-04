#[test]
fn test_bitreader_reversed() {
    use crate::bit_io::BitReaderReversed;

    let encoded: [u8; 16] = [
        0xC1, 0x41, 0x08, 0x00, 0x00, 0xEC, 0xC8, 0x96, 0x42, 0x79, 0xD4, 0xBC, 0xF7, 0x2C, 0xD5,
        0x48,
    ];
    //just the u128 in encoded
    let num_rev: u128 = 0x48_D5_2C_F7_BC_D4_79_42_96_C8_EC_00_00_08_41_C1;

    let mut br = BitReaderReversed::new(&encoded[..]);
    let mut accumulator = 0;
    let mut bits_read = 0;
    let mut x = 0;

    loop {
        x += 3;
        //semi random access pattern
        let mut num_bits = x % 16;
        if bits_read > 128 - num_bits {
            num_bits = 128 - bits_read;
        }

        let bits = br.get_bits(num_bits);
        bits_read += num_bits;
        accumulator |= u128::from(bits) << (128 - bits_read);
        if bits_read >= 128 {
            break;
        }
    }

    if accumulator != num_rev {
        panic!(
            "Bitreader failed somewhere. Accumulated bits: {:?}, Should be: {:?}",
            accumulator, num_rev
        );
    }
}

#[test]
fn test_bitreader_normal() {
    use crate::bit_io::BitReader;

    let encoded: [u8; 16] = [
        0xC1, 0x41, 0x08, 0x00, 0x00, 0xEC, 0xC8, 0x96, 0x42, 0x79, 0xD4, 0xBC, 0xF7, 0x2C, 0xD5,
        0x48,
    ];
    //just the u128 in encoded
    let num: u128 = 0x48_D5_2C_F7_BC_D4_79_42_96_C8_EC_00_00_08_41_C1;

    let mut br = BitReader::new(&encoded[..]);
    let mut accumulator = 0;
    let mut bits_read = 0;
    let mut x = 0;

    loop {
        x += 3;
        //semi random access pattern
        let mut num_bits = x % 16;
        if bits_read > 128 - num_bits {
            num_bits = 128 - bits_read;
        }

        let bits = br.get_bits(num_bits).unwrap();
        accumulator |= u128::from(bits) << bits_read;
        bits_read += num_bits;
        if bits_read >= 128 {
            break;
        }
    }

    if accumulator != num {
        panic!(
            "Bitreader failed somewhere. Accumulated bits: {:?}, Should be: {:?}",
            accumulator, num
        );
    }
}

/// The reader matches a bit-by-bit model, every width up to 64, up to the source's end.
#[test]
fn test_bitreader_normal_matches_model() {
    use crate::bit_io::BitReader;
    use alloc::vec::Vec;

    let source: Vec<u8> = (0..40u32).map(|x| (x * 151 + 7) as u8).collect();
    for len in 0..source.len() {
        let source = &source[..len];
        for first in 0..65 {
            let mut br = BitReader::new(source);
            let mut pos = 0;
            for (step, n) in core::iter::once(first).chain(1..65).enumerate() {
                let got = br.get_bits(n);
                if pos + n > len * 8 {
                    assert!(got.is_err(), "len {len} step {step}");
                    break;
                }
                let mut expected = 0u64;
                for bit in 0..n {
                    let at = pos + bit;
                    expected |= u64::from((source[at / 8] >> (at % 8)) & 1) << bit;
                }
                pos += n;
                assert_eq!(got.unwrap(), expected, "len {len} step {step}");
                assert_eq!(br.bits_read(), pos);
            }
        }
    }
    assert!(BitReader::new(&[0; 16]).get_bits(65).is_err());
}
