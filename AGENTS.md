- Strict no backward-compatibility or legacy paths no matter what.
- No changelog: the commits and pull requests are the record.

- There is no CI. After Rust changes run `cargo fmt --all -- --check`, `cargo test`,
  `cargo test --no-default-features` and `cargo clippy --all-targets -- -D warnings`, on stable.
  Then the same for WebAssembly, whose build executes sequences its own way (see
  `SequenceExecutor`): `cargo test --target wasm32-wasip1`, with and without default features,
  which runs under Node (`.cargo/config.toml`), and
  `cargo clippy --target wasm32-wasip1 --lib --tests -- -D warnings`. Fuzzing never reaches
  that path.
- After changes to decoding, fuzz each target for three minutes, on nightly with
  AddressSanitizer (cargo-fuzz's default, whose `-Zsanitizer` needs nightly), every target even
  if one fails, naming the failed ones at the end and exiting nonzero:
  `failed=; for t in decode decode_dict interop; do cargo +nightly fuzz run $t -- -max_total_time=180 || failed="$failed $t"; done; [ -z "$failed" ] || { echo "fuzzing failed:$failed (crashes in fuzz/artifacts/)"; false; }`.
  With the crate's unsafe code forbidden, AddressSanitizer checks mostly `interop`'s libzstd,
  the C reference. Fuzz only on nightly, never without a sanitizer; install nightly where it
  is missing. A crash is saved under `fuzz/artifacts/`; commit it with the fix, and
  `cargo test artifacts` replays them all.
- The crate forbids unsafe code (`#![forbid(unsafe_code)]` in `src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes.
