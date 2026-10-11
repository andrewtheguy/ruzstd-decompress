/// Huffman coding is a method of encoding where symbols are assigned a code,
/// and more commonly used symbols get shorter codes, and less commonly
/// used symbols get longer codes. Codes are prefix free, meaning no two codes
/// will start with the same sequence of bits.
mod huff0_decoder;
pub use huff0_decoder::*;
pub mod huff0_encoder;

/// Only needed for testing.
///
/// Encodes the data with a table built from that data
/// Decodes the result again by first decoding the table and then the data
/// Asserts that the decoded data equals the input
#[cfg(any(test, feature = "fuzz_exports"))]
pub fn round_trip(data: &[u8]) {
    use crate::bit_io::{BitReaderReversed, BitWriter};
    use alloc::vec::Vec;

    if data.len() < 2 {
        return;
    }
    if data.iter().all(|x| *x == data[0]) {
        return;
    }
    let mut writer = BitWriter::new();
    let encoder_table = huff0_encoder::HuffmanTable::build_from_data(data);
    let mut encoder = huff0_encoder::HuffmanEncoder::new(&encoder_table, &mut writer);

    encoder.encode(data, true);
    let encoded = writer.dump();
    let mut decoder_table = HuffmanTable::new();
    let table_bytes = decoder_table.build_decoder(&encoded).unwrap();
    let mut decoder = HuffmanDecoder::new(&decoder_table);

    let mut br = BitReaderReversed::new(&encoded[table_bytes as usize..]);
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
        panic!("Corrupted end marker");
    }

    decoder.init_state(&mut br);
    let mut decoded = Vec::new();
    while br.bits_remaining() > -(decoder_table.max_num_bits as isize) {
        decoded.push(decoder.decode_symbol());
        decoder.next_state(&mut br);
    }
    assert_eq!(&decoded, data);

    // The table based decoder must agree, and use up the stream exactly
    let stream = &encoded[table_bytes as usize..];
    let mut br = crate::decoding::literals_section_decoder::init_stream(stream).unwrap();
    let mut decoded = alloc::vec![0; data.len()];
    decoder_table.decode_stream(&mut br, &mut decoded);
    assert_eq!(&decoded, data);
    crate::decoding::literals_section_decoder::check_stream_end(&br, decoder_table.max_num_bits)
        .unwrap();

    // Same for four interleaved streams, split like the literals section does
    let segment_size = data.len().div_ceil(4);
    if data.len() < 4 || 3 * segment_size > data.len() {
        return;
    }
    let mut writer = BitWriter::new();
    let mut encoder = huff0_encoder::HuffmanEncoder::new(&encoder_table, &mut writer);
    encoder.encode4x(data, false);
    let encoded = writer.dump();
    let jump1 = encoded[0] as usize + ((encoded[1] as usize) << 8);
    let jump2 = jump1 + encoded[2] as usize + ((encoded[3] as usize) << 8);
    let jump3 = jump2 + encoded[4] as usize + ((encoded[5] as usize) << 8);
    let streams = &encoded[6..];
    let init = crate::decoding::literals_section_decoder::init_stream;
    let mut brs = [
        init(&streams[..jump1]).unwrap(),
        init(&streams[jump1..jump2]).unwrap(),
        init(&streams[jump2..jump3]).unwrap(),
        init(&streams[jump3..]).unwrap(),
    ];
    let mut decoded = alloc::vec![0; data.len()];
    let (out0, rest) = decoded.split_at_mut(segment_size);
    let (out1, rest) = rest.split_at_mut(segment_size);
    let (out2, out3) = rest.split_at_mut(segment_size);
    decoder_table.decode_4_streams(&mut brs, [out0, out1, out2, out3]);
    assert_eq!(&decoded, data);
    for br in &brs {
        crate::decoding::literals_section_decoder::check_stream_end(br, decoder_table.max_num_bits)
            .unwrap();
    }
}

#[test]
fn roundtrip() {
    use alloc::vec::Vec;
    round_trip(&[1, 1, 1, 1, 2, 3]);
    round_trip(&[1, 1, 1, 1, 2, 3, 5, 45, 12, 90]);

    for size in 2..512 {
        use alloc::vec;
        let data = vec![123; size];
        round_trip(&data);
        let mut data = Vec::new();
        for x in 0..size {
            data.push(x as u8);
        }
        round_trip(&data);
    }

    #[cfg(feature = "std")]
    if std::fs::exists("fuzz/artifacts/huff0").unwrap_or(false) {
        for file in std::fs::read_dir("fuzz/artifacts/huff0").unwrap() {
            if file.as_ref().unwrap().file_type().unwrap().is_file() {
                let data = std::fs::read(file.unwrap().path()).unwrap();
                round_trip(&data);
            }
        }
    }
}
