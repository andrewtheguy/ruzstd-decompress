#!/bin/bash
# Every variant, native and wasm, with zDesk's egfx release profile: LTO, one codegen unit, panic=abort.
# The harness is a crate of its own per variant, so ruzstd's dev-dependencies (libzstd's C) stay out.
set -e
S=~/codes/staging-area/zstd-rs-tuned-scratch
mkdir -p $S/bin
for v in "$@"; do
  C=$S/benchcrates/$v
  mkdir -p $C/src
  cp $S/harness/decode_bench.rs $C/src/main.rs
  cp $S/variants/$v/rust-toolchain.toml $C/
  cat > $C/Cargo.toml <<TOML
[package]
name = "decode_bench"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
ruzstd = { path = "../../variants/$v" }

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"

[workspace]
TOML
  cd $C
  export CARGO_TARGET_DIR=~/codes/staging-area/zstd-rs-tuned-target/$v
  ~/.cargo/bin/cargo build -q --release
  cp $CARGO_TARGET_DIR/release/decode_bench $S/bin/$v-native
  RUSTFLAGS="-C target-feature=+simd128,+bulk-memory" ~/.cargo/bin/cargo build -q --release --target wasm32-wasip1
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-simd --enable-sign-ext --enable-mutable-globals \
    $CARGO_TARGET_DIR/wasm32-wasip1/release/decode_bench.wasm -o $S/bin/$v.wasm
done
ls -la $S/bin
