#!/bin/bash
# make-corpus.sh [SCRATCH]: the general corpora of README.md's workloads, byte for byte, on any machine.
#
# Writes SCRATCH/corpus/ (what run.sh reads) from SCRATCH/corpus-src/ (the sources, downloaded
# into it if missing), then checks every output against corpus.sha256 beside this script.
# SCRATCH defaults to env.sh's.
#
# Needs: curl, unzip, GNU tar (TAR=gtar on macOS), zstd 1.5.7 (another version compresses
# differently), sha256sum (or shasum). And the video clip, which cannot be downloaded again: VIDEO,
# by default the backup drive's copy; without it the clip is left out and its check fails.
#
# What makes the bytes reproducible, since a tar's headers and zstd's choices are in them:
# - silesia.tar: the zip's 12 files in the zip's order, owner and group 0 by number, mode 0644,
#   mtime 0, GNU format, so no user name, time zone or locale reaches it.
# - zstd reads each input as a file, never a pipe: with no size known it picks other parameters.
#   Its multithreading (-T0) gives the same bytes on any number of cores.
# - The GitHub frames: the files in a fixed order (case-insensitive, lower case first, the order
#   en_US.UTF-8 gave on Debian 13) set without the locale. The dictionary's training depends on
#   its files' order, and it was trained from a directory's listing (zstd --train -r), so that
#   order is kept in github-dict-order.txt.
set -euo pipefail
H=$(cd "$(dirname "$0")" && pwd)
. "$H/env.sh"
SCRATCH=${1:-$SCRATCH}
SRC=$SCRATCH/corpus-src
OUT=$SCRATCH/corpus
TAR=${TAR:-tar}
VIDEO=${VIDEO:-/mnt/dasdata/backup/ruzstd-decompress-artifacts/corpus/explosions-4k-60s.webm}
export LC_ALL=C

SILESIA_URL=https://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip
SILESIA_SHA=0626e25f45c0ffb5dc801f13b7c82a3b75743ba07e3a71835a41e3d9f63c77af
GITHUB_URL=https://github.com/facebook/zstd/releases/download/v1.1.3/github_users_sample_set.tar.zst
GITHUB_SHA=e35074abe4db6a888f19bcc12ad56c760bec2ca86bb7dff18bb4967789fc0de1
VIDEO_SHA=ba8a6a586890f80bd78a3a5180644d867882f7c5370c63573b5aec582ed66e54

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
fetch() { # fetch URL SHA FILE: download unless already there, then check
  if [ ! -e "$3" ]; then curl -sSfL -o "$3.part" "$1" && mv "$3.part" "$3"; fi
  echo "$2  $3" | sha256 -c --quiet - || { echo "$3: not the expected file" >&2; exit 1; }
}

zstd --version | grep -q 'v1\.5\.7' || { echo "zstd 1.5.7 is needed, found: $(zstd --version)" >&2; exit 1; }
"$TAR" --version | grep -q 'GNU tar' || { echo "GNU tar is needed (TAR=gtar on macOS)" >&2; exit 1; }
mkdir -p "$SRC" "$OUT"
cd "$SRC"

# Silesia
fetch $SILESIA_URL $SILESIA_SHA silesia.zip
rm -rf silesia && mkdir silesia && unzip -q silesia.zip -d silesia
FILES="dickens mozilla mr nci ooffice osdb reymont samba sao webster xml x-ray"
"$TAR" -cf silesia.tar --format=gnu --numeric-owner --owner=0 --group=0 --mode=0644 --mtime=@0 -C silesia $FILES
zstd -q -f -T0 -3 silesia.tar -o "$OUT/silesia.tar.l3.zst"
zstd -q -f -T0 -19 silesia.tar -o "$OUT/silesia.tar.l19.zst"
zstd -q -f -T0 --fast=4 silesia.tar -o "$OUT/silesia.tar.fast4.zst"
zstd -q -f -1 silesia/x-ray -o "$OUT/silesia-x-ray.l1.zst"
zstd -q -f -3 silesia/nci -o "$OUT/silesia-nci.l3.zst"

# zstd's small-data and dictionary set
fetch $GITHUB_URL $GITHUB_SHA github_users_sample_set.tar.zst
rm -rf github && mkdir github && zstd -q -dc github_users_sample_set.tar.zst | "$TAR" -xf - -C github --warning=no-unknown-keyword
sed 's|.*|github/github/user.&.json|' "$H/github-dict-order.txt" > github-dict-order.list
zstd -q -f --train --filelist github-dict-order.list -o "$OUT/github-dict.dict"
# The frames' order: lower-cased, then with the case swapped so lower case comes first.
find github -type f | sort > github.paths
tr A-Z a-z < github.paths > github.k1
tr a-zA-Z A-Za-z < github.paths > github.k2
paste github.k1 github.k2 github.paths | sort -t "$(printf '\t')" -k1,1 -k2,2 | cut -f3 > github.list
rm -f "$OUT/github.frames" "$OUT/github-dict.frames"
while IFS= read -r f; do
  zstd -q -3 -c "$f" >> "$OUT/github.frames"
  zstd -q -3 -D "$OUT/github-dict.dict" -c "$f" >> "$OUT/github-dict.frames"
done < github.list

# The video clip, in raw blocks
if [ -e "$VIDEO" ]; then
  echo "$VIDEO_SHA  $VIDEO" | sha256 -c --quiet -
  zstd -q -f -T0 -3 "$VIDEO" -o "$OUT/explosions-4k-60s.webm.l3.zst"
else
  echo "no video clip at $VIDEO (VIDEO=...): left out" >&2
fi

cd "$OUT" && sha256 -c "$H/corpus.sha256"
