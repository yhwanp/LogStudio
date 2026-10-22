#!/bin/sh
# follow-idle.sh <follow binary> <base log> <small log> <seconds>
# Per-poll cost on the big log, then whole-process CPU of an idle followed
# small log against the same run without following (off/on/off/on).
bin=$1; big=$2; small=$3; secs=$4
"$bin" poll "$big" 100000
"$bin" poll "$big" 100000
for mode in off on off on; do
  /usr/bin/time -l "$bin" idle "$small" "$secs" "$mode" 2>/tmp/follow-bench-idle.txt
  grep -E " real | user | sys" /tmp/follow-bench-idle.txt | head -1
  grep -E "instructions retired|maximum resident set size" /tmp/follow-bench-idle.txt
done
