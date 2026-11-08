#!/bin/bash
# run.sh OUT.jsonl VARIANT...: every workload in $SCRATCH/corpus and $SCRATCH/tiles, $ROUNDS
# rounds (3), the variants shuffled each round, native and wasm, each process under $PIN.
# PLATFORMS="native" for native alone; a variant without a .wasm (Z, libzstd) runs native alone.
# A workload whose file is missing is left out, so a machine without the tiles runs the rest.
# Settings (SCRATCH, NODE, PIN, ROUNDS): env.sh.
HARNESS=$(cd "$(dirname "$0")" && pwd)
. "$HARNESS/env.sh"
S=$SCRATCH
OUT=$1; shift
shopt -s nullglob
# A workload is the harness's arguments, a mode and its files, joined by US (\x1f), so that a
# path may hold spaces.
US=$'\x1f'
WORK=()
for f in "$S"/tiles/*.tiles; do WORK+=("tiles$US$f"); done
for f in "$S"/corpus/*.zst; do WORK+=("frame$US$f"); done
# Small frames back to back, with NAME.dict beside NAME.frames if they were made with one.
for f in "$S"/corpus/*.frames; do d=${f%.frames}.dict; WORK+=("frames$US$f$([ -e "$d" ] && printf '%s' "$US$d")"); done
[ ${#WORK[@]} -gt 0 ] || { echo "no workloads in $S/corpus or $S/tiles (make-corpus.sh makes them)" >&2; exit 1; }
for v in "$@"; do [ -e "$S/bin/$v-native" ] || { echo "no build of $v in $S/bin (build.sh $v)" >&2; exit 1; }; done
echo "${#WORK[@]} workloads, pinned by: ${PIN:-nothing}"
# Not shuf, which macOS lacks; awk's srand() alone seeds by the second, so $RANDOM seeds it.
shuffle() { awk -v seed=$RANDOM 'BEGIN { srand(seed) } { print rand() "\t" $0 }' | sort | cut -f2-; }
for round in $(seq 1 "$ROUNDS"); do
  for w in "${WORK[@]}"; do
    IFS=$US read -r -a args <<< "$w"
    for v in $(printf '%s\n' "$@" | shuffle); do
      for p in ${PLATFORMS:-native wasm}; do
        [ $p = wasm ] && [ ! -e "$S/bin/$v.wasm" ] && continue
        # $PIN splits into words on purpose: a command and its arguments.
        if [ $p = native ]; then line=$($PIN "$S/bin/$v-native" "${args[@]}")
        else line=$($PIN "$NODE" --no-warnings "$HARNESS/run-wasi.mjs" "$S/bin/$v.wasm" "${args[@]}"); fi \
          || { echo "$v $p failed on: ${args[*]}" >&2; exit 1; }
        echo "{\"variant\":\"$v\",\"platform\":\"$p\",\"round\":$round,${line#\{}" >> "$OUT"
      done
    done
  done
  echo "round $round done $(date)"
done
echo ALLDONE
