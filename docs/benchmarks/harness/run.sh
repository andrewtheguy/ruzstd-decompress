#!/bin/bash
# run.sh OUT.jsonl VARIANT...: every workload, 3 rounds, the variants shuffled each round, native and wasm, on one vCPU.
# PLATFORMS="native" for native alone; a variant without a .wasm (Z, libzstd) runs native alone.
S=~/codes/staging-area/zstd-rs-tuned-scratch
N=~/codes/staging-area/node/bin/node
OUT=$1; shift
WORK=()
for f in $S/tiles/*.tiles; do WORK+=("tiles $f"); done
for f in $S/corpus/*.zst; do WORK+=("frame $f"); done
for round in 1 2 3; do
  for w in "${WORK[@]}"; do
    for v in $(printf '%s\n' "$@" | shuf); do
      for p in ${PLATFORMS:-native wasm}; do
        [ $p = wasm ] && [ ! -e $S/bin/$v.wasm ] && continue
        if [ $p = native ]; then line=$(taskset -c 2 $S/bin/$v-native $w)
        else line=$(taskset -c 2 $N --no-warnings $S/harness/run-wasi.mjs $S/bin/$v.wasm $w); fi
        echo "{\"variant\":\"$v\",\"platform\":\"$p\",\"round\":$round,${line#\{}" >> $OUT
      done
    done
  done
  echo "round $round done $(date)"
done
echo ALLDONE
