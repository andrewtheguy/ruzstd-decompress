#!/bin/bash
# Every variant, native and wasm, with zDesk's egfx release profile: LTO, one codegen unit, panic=abort.
# The harness is a crate of its own per variant, so ruzstd-decompress's dev-dependencies (libzstd's C) stay out.
set -e
S=~/codes/staging-area/ruzstd-decompress-scratch
mkdir -p $S/bin
# Z is libzstd, from harness/libzstd (its Cargo.lock keeps zstd-sys at 1.5.7), native only.
HARNESS=$(cd "$(dirname "$0")" && pwd)
for v in "$@"; do
  if [ $v = Z ]; then
    C=$S/benchcrates/Z
    mkdir -p $C/src
    cp $HARNESS/libzstd/Cargo.toml $HARNESS/libzstd/Cargo.lock $C/
    cp $HARNESS/libzstd/decode_bench_libzstd.rs $C/src/main.rs
    (cd $C && CARGO_TARGET_DIR=~/codes/staging-area/ruzstd-decompress-target/Z ~/.cargo/bin/cargo build -q --release --locked)
    cp ~/codes/staging-area/ruzstd-decompress-target/Z/release/decode_bench_libzstd $S/bin/Z-native
    continue
  fi
  C=$S/benchcrates/$v
  mkdir -p $C/src
  # The harness of the variant's own commit if it has one, since the decoder's API changes
  # with it; else the scratch copy.
  H=$S/variants/$v/docs/benchmarks/harness/decode_bench.rs
  [ -e $H ] || H=$S/harness/decode_bench.rs
  cp $H $C/src/main.rs
  # The variant's own toolchain pin if it has one, else the default (stable)
  rm -f $C/rust-toolchain.toml
  if [ -e $S/variants/$v/rust-toolchain.toml ]; then cp $S/variants/$v/rust-toolchain.toml $C/; fi
  # The variant's package, under the name its harness uses: a commit from before the rename
  # calls the crate ruzstd.
  P=$(sed -n 's/^name = "\(.*\)"$/\1/p' $S/variants/$v/Cargo.toml | head -1)
  K=$(grep -o -m1 '\bruzstd[a-z_]*::' $C/src/main.rs | head -1 | sed 's/::$//')
  cat > $C/Cargo.toml <<TOML
[package]
name = "decode_bench"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
$K = { package = "$P", path = "../../variants/$v" }

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"

[workspace]
TOML
  cd $C
  export CARGO_TARGET_DIR=~/codes/staging-area/ruzstd-decompress-target/$v
  ~/.cargo/bin/cargo build -q --release
  cp $CARGO_TARGET_DIR/release/decode_bench $S/bin/$v-native
  RUSTFLAGS="-C target-feature=+simd128,+bulk-memory" ~/.cargo/bin/cargo build -q --release --target wasm32-wasip1
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-simd --enable-sign-ext --enable-mutable-globals \
    $CARGO_TARGET_DIR/wasm32-wasip1/release/decode_bench.wasm -o $S/bin/$v.wasm
done
ls -la $S/bin
