# Settings for the harness's scripts, sourced by each. Every one can be set in the environment;
# the defaults are the maintainer's layout (../devtools: ~/codes/staging-area), so an unset
# variable gives the runs recorded in README.md.
#
#   SCRATCH   corpora, variants, builds and results     ~/codes/staging-area/ruzstd-decompress-scratch
#   TARGETS   cargo's target directories, one a build   ~/codes/staging-area/ruzstd-decompress-target
#   CARGO     cargo (rustup's, so a variant's rust-toolchain.toml applies)   cargo on the PATH, else ~/.cargo/bin/cargo
#   WASM_OPT  binaryen's wasm-opt                       wasm-opt on the PATH
#   NODE      Node, for the wasm builds                 ~/codes/staging-area/node/bin/node if there, else node on the PATH
#   PIN       the command each timed process runs under: one CPU, so its timings do not move
#             between cores. Default `taskset -c 2` where taskset is and there are 3 CPUs or
#             more, else none. PIN= (empty) runs unpinned.
#   ROUNDS    rounds of run.sh                          3
SCRATCH=${SCRATCH:-$HOME/codes/staging-area/ruzstd-decompress-scratch}
TARGETS=${TARGETS:-$HOME/codes/staging-area/ruzstd-decompress-target}
if [ -z "${CARGO:-}" ]; then
  if command -v cargo >/dev/null; then CARGO=cargo; else CARGO=$HOME/.cargo/bin/cargo; fi
fi
WASM_OPT=${WASM_OPT:-wasm-opt}
if [ -z "${NODE:-}" ]; then
  if [ -x "$HOME/codes/staging-area/node/bin/node" ]; then NODE=$HOME/codes/staging-area/node/bin/node; else NODE=node; fi
fi
if [ -z "${PIN+set}" ]; then
  if command -v taskset >/dev/null && [ "$(nproc 2>/dev/null || echo 1)" -ge 3 ]; then PIN="taskset -c 2"; else PIN=; fi
fi
ROUNDS=${ROUNDS:-3}
