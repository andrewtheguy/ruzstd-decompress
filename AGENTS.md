- Strict no backward-compatibility or legacy paths no matter what.
- No changelog: the commits and pull requests are the record.

- There is no CI. After Rust changes run `cargo fmt --all -- --check`, `cargo test`,
  `cargo test --no-default-features` and `cargo clippy --all-targets -- -D warnings`, on stable.
- After changes to decoding, fuzz each target for three minutes, on stable:
  `for t in decode decode_dict interop; do cargo fuzz run --sanitizer none $t -- -max_total_time=180; done`.
  Without a sanitizer cargo-fuzz needs no nightly (only `-Zsanitizer` does), and with the crate's
  unsafe code forbidden, AddressSanitizer would check only `interop`'s libzstd, the C reference.
  A crash is saved under `fuzz/artifacts/`; commit it with the fix, and `cargo test artifacts`
  replays them all.
- The crate forbids unsafe code (`#![forbid(unsafe_code)]` in `src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes.
