#!/bin/bash
# Lossless tile bodies of each sample, as the page receives them, for ruzstd-decompress's decode bench.
MB=~/codes/staging-area/wlshare-tiledump-target/release/examples/motion-bench
S=/mnt/dasdata/backup/wlshare-artifacts/frame-samples
cd "$(dirname "$0")"
jobs=()
for size in 1920x1080 2880x1800@2; do
  for sc in browser drag still terminal uncover video mixed; do
    f=$S/$sc-$size.wlcap.zst; [ -e "$f" ] || continue
    for pic in held lossless; do
      extra=; [ $pic = lossless ] && extra=--lossless
      echo "$sc $size $pic"
      TILE_DUMP=$sc-$size-$pic.tiles $MB --libx264 --tiles-only $extra "$f" > $sc-$size-$pic.json 2> $sc-$size-$pic.err &
      while [ $(jobs -r | wc -l) -ge 3 ]; do wait -n; done
    done
  done
done
wait
echo ALLDONE
