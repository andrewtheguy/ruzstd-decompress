#!/bin/bash
# threads.sh OUT.jsonl: FrameDecoder::decode_all_parallel on 1 to 8 threads (THREADS), natively
# and in WebAssembly, on the general corpora in $SCRATCH/corpus and on Silesia cut into frames
# of 4 MiB ($SCRATCH/corpus-threads/silesia.tar.l3.4M-frames.zst, made here from
# $SCRATCH/corpus-src/silesia.tar).
#
# Natively: the crate's examples/decode_parallel.rs, with the profile of build.sh's builds.
# In wasm: examples/wasm-threads, the module on a pool of Node's workers, by its bench.mjs; it
# needs that example's nightly (rustup installs it) and wasm-bindgen-cli 0.2.130. Both print
# one JSON line a file and thread count, the median of decodes repeated for 2 s and 3 times at
# least; OUT gets each with its platform. Unpinned: the threads are the point. Run it from the
# tree to measure. Settings (SCRATCH, TARGETS, CARGO, WASM_OPT, NODE): env.sh.
set -euo pipefail
HARNESS=$(cd "$(dirname "$0")" && pwd)
. "$HARNESS/env.sh"
ROOT=$(cd "$HARNESS/../../.." && pwd)
OUT=$1
THREADS=${THREADS:-1,2,3,4,6,8}
S=$SCRATCH

# The independent frames: silesia.tar in 4 MiB pieces, each `zstd -3` (reading a file, as
# make-corpus.sh does), back to back. Kept out of corpus/, whose every file run.sh runs.
mkdir -p "$S/corpus-threads"
F=$S/corpus-threads/silesia.tar.l3.4M-frames.zst
if [ ! -e "$F" ]; then
  P=$(mktemp -d "$S/pieces.XXXXXX")
  split -b 4194304 -d -a 3 "$S/corpus-src/silesia.tar" "$P/part."
  for f in "$P"/part.*; do zstd -q -3 "$f" -c; done > "$F.part" && mv "$F.part" "$F"
  rm -r "$P"
fi

(cd "$ROOT" && CARGO_TARGET_DIR="$TARGETS/threads" CARGO_PROFILE_RELEASE_LTO=true \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort \
  "$CARGO" build -q --release --features rayon --example decode_parallel)
NATIVE=$TARGETS/threads/release/examples/decode_parallel
(cd "$ROOT/examples/wasm-threads" && CARGO="$CARGO" CARGO_TARGET_DIR="$TARGETS/wasm-threads" WASM_OPT="$WASM_OPT" ./build.sh > /dev/null)

run() { # run FILE [DICT]
  local dict=()
  [ $# -gt 1 ] && dict=(--dict "$2")
  "$NATIVE" "$1" --threads "$THREADS" "${dict[@]}" | sed 's/^{/{"platform":"native",/' >> "$OUT"
  "$NODE" --no-warnings "$ROOT/examples/wasm-threads/bench.mjs" "$1" --threads "$THREADS" "${dict[@]}" \
    | sed 's/^{/{"platform":"wasm",/' >> "$OUT"
  echo "done $(basename "$1") $(date)"
}
for f in silesia.tar.l3.zst silesia.tar.l19.zst silesia.tar.fast4.zst silesia-x-ray.l1.zst \
  silesia-nci.l3.zst; do
  run "$S/corpus/$f"
done
run "$F"
run "$S/corpus/github.frames"
run "$S/corpus/github-dict.frames" "$S/corpus/github-dict.dict"
echo ALLDONE
