- Strict no backward-compatibility or legacy paths no matter what.
- No changelog: the commits and pull requests are the record.

- There is no CI. After Rust changes run `cargo fmt --all -- --check`, `cargo test`,
  `cargo test --no-default-features` and `cargo clippy --all-targets -- -D warnings`, on stable.
  Then the same for WebAssembly, whose build executes sequences its own way (see
  `SequenceExecutor`): `cargo test --target wasm32-wasip1`, with and without default features,
  which runs under Node (`.cargo/config.toml`), and
  `cargo clippy --target wasm32-wasip1 --lib --tests -- -D warnings`. Fuzzing never reaches
  that path.
- After changes to decoding, fuzz each target for three minutes, on stable, every target even
  if one fails, naming the failed ones at the end and exiting nonzero:
  `failed=; for t in decode decode_dict interop; do cargo fuzz run --sanitizer none $t -- -max_total_time=180 || failed="$failed $t"; done; [ -z "$failed" ] || { echo "fuzzing failed:$failed (crashes in fuzz/artifacts/)"; false; }`.
  Without a sanitizer cargo-fuzz needs no nightly (only `-Zsanitizer` does), and with the crate's
  unsafe code forbidden, AddressSanitizer would check only `interop`'s libzstd, the C reference.
  A crash is saved under `fuzz/artifacts/`; commit it with the fix, and `cargo test artifacts`
  replays them all.
- The crate forbids unsafe code (`#![forbid(unsafe_code)]` in `src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes.
