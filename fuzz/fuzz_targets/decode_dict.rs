#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate ruzstd_decompress;

fuzz_target!(|data: &[u8]| {
    ruzstd_decompress::decoding::Dictionary::decode_dict(data).ok(); // may return error on invalid data, may not panic.
});
