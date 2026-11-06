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
  AddressSanitizer checks the crate's unsafe code as well as `interop`'s libzstd, the C
  reference. Fuzz only on nightly, never without a sanitizer; install nightly where it is
  missing. A crash is saved under `fuzz/artifacts/`; commit it with the fix, and
  `cargo test artifacts` replays them all.
- After changes to unsafe code, or to what calls it, run the tests that reach it under Miri, on
  nightly (install it with `rustup +nightly component add miri` where it is missing), with
  isolation off so that they read the corpus files:
  `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --lib -- raw_writer small_files_decode short_writer`.
  Keep that set reaching every unsafe path, and small enough for Miri (about 12 minutes here).
- Unsafe code is allowed where it makes decoding measurably faster on the benchmarks' workloads,
  kept to the hot loops' loads and copies; the rest of the crate stays safe. Every `unsafe`
  block has a `// SAFETY:` comment saying why its pointers are in bounds
  (`clippy::undocumented_unsafe_blocks`), every `unsafe fn` a `# Safety` section, and
  `unsafe_op_in_unsafe_fn` is denied (`src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes.
