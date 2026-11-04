#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate ruzstd;

fuzz_target!(|data: &[u8]| {
    ruzstd::decoding::Dictionary::decode_dict(data).ok(); // may return error on invalid data, may not panic.
});
