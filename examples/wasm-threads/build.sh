#!/bin/bash
# build.sh: the module and its JavaScript glue, into pkg/: cargo (the nightly and the flags of
# rust-toolchain.toml and .cargo/config.toml), then wasm-bindgen (`--target web`, whose glue a
# browser and Node both load, given the module), then wasm-opt.
#
# Needs: rustup, wasm-bindgen-cli at the wasm-bindgen of Cargo.toml (0.2.130:
# `cargo install wasm-bindgen-cli --version 0.2.130 --locked`), binaryen's wasm-opt.
# CARGO, CARGO_TARGET_DIR, WASM_BINDGEN and WASM_OPT are taken from the environment.
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-target}
"${CARGO:-cargo}" build --release
"${WASM_BINDGEN:-wasm-bindgen}" --target web --out-dir pkg --out-name ruzstd_mt \
  "$T/wasm32-unknown-unknown/release/ruzstd_mt.wasm"
"${WASM_OPT:-wasm-opt}" -O3 --enable-threads --enable-bulk-memory --enable-simd \
  --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
  pkg/ruzstd_mt_bg.wasm -o pkg/ruzstd_mt_bg.wasm
ls -l pkg
