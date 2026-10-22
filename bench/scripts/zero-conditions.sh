#!/bin/sh
# Generates text and DLT inputs of 512 MiB and 2 GiB in <dir> (kept for
# reruns; delete them afterwards) and runs `zero-conditions` on each in
# separate processes under `/usr/bin/time -l` (macOS) for peak RSS: the
# search methods in <processes> processes of 2 rounds, then the per-row and
# beside-a-query measurements, and the tables alone and the joint scan alone
# for their peak RSS. Raw output goes to <dir>/out.
# Usage: zero-conditions.sh <dir> <processes> [release dir]
set -e
DATA=${1:?data dir}
PROCS=${2:?processes}
BIN=${3:-target/release}
mkdir -p "$DATA/out"
[ -f "$DATA/a-512.log" ] || "$BIN/field-filters-gen" 512 "$DATA/a-512.log"
[ -f "$DATA/a-2048.log" ] || "$BIN/field-filters-gen" 2048 "$DATA/a-2048.log"
[ -f "$DATA/d-512.dlt" ] || "$BIN/dlt-gen" 512 "$DATA/d-512.dlt" android
[ -f "$DATA/d-2048.dlt" ] || "$BIN/dlt-gen" 2048 "$DATA/d-2048.dlt" android
for f in a-512.log d-512.dlt a-2048.log d-2048.dlt; do
  p=1
  while [ "$p" -le "$PROCS" ]; do
    /usr/bin/time -l "$BIN/zero-conditions" "$DATA/$f" table,first,joint,count 2 \
      > "$DATA/out/$f-search-$p.txt" 2>&1
    p=$((p + 1))
  done
  /usr/bin/time -l "$BIN/zero-conditions" "$DATA/$f" rows,beside 3 \
    > "$DATA/out/$f-beside.txt" 2>&1
  for m in table joint; do
    /usr/bin/time -l "$BIN/zero-conditions" "$DATA/$f" $m 1 \
      > "$DATA/out/$f-$m.txt" 2>&1
  done
done
