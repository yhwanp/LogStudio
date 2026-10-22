#!/bin/sh
# Generates the inputs in <dir> (kept for reruns; delete them afterwards) and
# runs `field-filters` for each given build, rotating the order per round,
# each in its own process under `/usr/bin/time -l` (macOS) for peak RSS. A
# build is <label>=<release dir>; the current checkout's is target/release
# after `cargo build --release -p logstudio-bench`, an earlier commit's comes
# from ../baseline.sh. Raw output goes to <dir>/out; summarize it with
# field-filters-summary.py. Labels and paths must not contain spaces; set
# PERF_FILTERS=wide to time the filters chosen for the wide values.
# Usage: field-filters.sh <dir> <rounds> <reps for 2 GiB> <label>=<dir>...
set -e
DATA=${1:?data dir}
ROUNDS=${2:?rounds}
BIG_REPS=${3:?reps for 2 GiB}
shift 3
[ $# -gt 0 ] || { echo "no <label>=<release dir> given" >&2; exit 2; }
GEN=${1#*=}/field-filters-gen
mkdir -p "$DATA/out"
[ -f "$DATA/a-512.log" ] || "$GEN" 512 "$DATA/a-512.log"
[ -f "$DATA/w-512.log" ] || "$GEN" 512 "$DATA/w-512.log" wide
[ -f "$DATA/a-2048.log" ] || "$GEN" 2048 "$DATA/a-2048.log"
[ -f "$DATA/w-2048.log" ] || "$GEN" 2048 "$DATA/w-2048.log" wide
for f in a-512 w-512 a-2048 w-2048; do
  case $f in *-2048) reps=$BIG_REPS ;; *) reps=2 ;; esac
  "${1#*=}/field-collect" "$DATA/$f.log" 6 > "$DATA/out/$f-collect.txt"
  r=1
  while [ "$r" -le "$ROUNDS" ]; do
    # Round r starts with build r (cycling), so each build leads in turn.
    i=0
    while [ $i -lt $# ]; do
      k=$(( (r - 1 + i) % $# + 1 ))
      eval "build=\${$k}"
      label=${build%%=*}
      /usr/bin/time -l "${build#*=}/field-filters" "$DATA/$f.log" "$label" "$reps" \
        > "$DATA/out/$f-$label-$r.txt" 2>&1
      i=$((i + 1))
    done
    r=$((r + 1))
  done
done
