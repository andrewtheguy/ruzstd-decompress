- Strict no backward-compatibility or legacy paths no matter what.
- No changelog: the commits and pull requests are the record.

- There is no CI. After Rust changes run `cargo fmt --all -- --check`, `cargo test`,
  `cargo test --no-default-features`, `cargo test --features rayon` and
  `cargo clippy --all-targets -- -D warnings` (and again with `--features rayon`), on stable.
  Then the same for WebAssembly, whose addresses are 32 bits:
  `cargo test --target wasm32-wasip1`, with and without default features and with
  `--features rayon` (one thread there), which runs under Node (`.cargo/config.toml`), and
  `cargo clippy --target wasm32-wasip1 --lib --tests -- -D warnings`.
- After changes to the parallel decoder (`src/decoding/parallel.rs`, the `rayon` feature) or to
  `examples/wasm-threads`, the module on several threads, which builds on the nightly of its
  own `rust-toolchain.toml`: in `examples/wasm-threads`, `./build.sh && node test.mjs`, and
  `node browser-test.mjs ../../decodecorpus_files z000033.zst z000021.zst` (headless Chrome).
- After changes to decoding, fuzz each target for three minutes, on nightly with
  AddressSanitizer (cargo-fuzz's default, whose `-Zsanitizer` needs nightly), every target even
  if one fails, naming the failed ones at the end and exiting nonzero:
  `failed=; for t in decode decode_dict interop decode_parallel; do cargo +nightly fuzz run $t -- -max_total_time=180 || failed="$failed $t"; done; [ -z "$failed" ] || { echo "fuzzing failed:$failed (crashes in fuzz/artifacts/)"; false; }`.
  AddressSanitizer checks the crate's unsafe code as well as `interop`'s libzstd, the C
  reference. Fuzz only on nightly, never without a sanitizer; install nightly where it is
  missing. A crash is saved under `fuzz/artifacts/`; commit it with the fix, and
  `cargo test artifacts` replays them all.
- After changes to unsafe code, or to what calls it, run the tests that reach it under Miri, on
  nightly (install it with `rustup +nightly component add miri` where it is missing), with
  isolation off so that they read the corpus files:
  `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --lib -- raw_writer small_files_decode short_writer`.
  Keep that set reaching every unsafe path, and small enough for Miri (about 12 minutes here).
  The parallel decoder's split blocks, whose stored sequences the executor's raw loop runs,
  apart, under Tree Borrows (rayon's crossbeam fails Stacked Borrows) and with its pool's
  threads left running at the end (about 5 minutes):
  `MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test --features rayon --lib -- split_small_frame`.
- Unsafe code is allowed where it makes decoding measurably faster on the benchmarks' workloads,
  kept to the hot loops' loads and copies; the rest of the crate stays safe. Every `unsafe`
  block has a `// SAFETY:` comment saying why its pointers are in bounds
  (`clippy::undocumented_unsafe_blocks`), every `unsafe fn` a `# Safety` section, and
  `unsafe_op_in_unsafe_fn` is denied (`src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes ("Running it again":
  its scripts run on any machine, their settings in `docs/benchmarks/harness/env.sh`).
