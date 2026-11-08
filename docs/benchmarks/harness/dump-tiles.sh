#!/bin/bash
# dump-tiles.sh [OUT]: lossless tile bodies of each sample, as the page receives them, for
# ruzstd-decompress's decode bench, into OUT (default $SCRATCH/tiles-all).
# Needs wlshare's motion-bench with wlshare-tile-dump.patch (MOTION_BENCH) and its frame samples
# (SAMPLES), which are not public: the defaults are the maintainer's.
HARNESS=$(cd "$(dirname "$0")" && pwd)
. "$HARNESS/env.sh"
MB=${MOTION_BENCH:-$HOME/codes/staging-area/wlshare-tiledump-target/release/examples/motion-bench}
S=${SAMPLES:-/mnt/dasdata/backup/wlshare-artifacts/frame-samples}
OUT=${1:-$SCRATCH/tiles-all}
[ -x "$MB" ] || { echo "no motion-bench at $MB (MOTION_BENCH=...)" >&2; exit 1; }
[ -d "$S" ] || { echo "no frame samples at $S (SAMPLES=...)" >&2; exit 1; }
mkdir -p "$OUT" && cd "$OUT"
jobs=()
for size in 1920x1080 2880x1800@2; do
  for sc in browser drag still terminal uncover video mixed; do
    f=$S/$sc-$size.wlcap.zst; [ -e "$f" ] || continue
    for pic in held lossless; do
      extra=; [ $pic = lossless ] && extra=--lossless
      echo "$sc $size $pic"
      TILE_DUMP=$sc-$size-$pic.tiles "$MB" --libx264 --tiles-only $extra "$f" > $sc-$size-$pic.json 2> $sc-$size-$pic.err &
      while [ $(jobs -r | wc -l) -ge 3 ]; do wait -n; done
    done
  done
done
wait
echo ALLDONE
