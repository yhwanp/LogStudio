#!/bin/sh
# follow-grow.sh <follow binary> <base log> <append MiB> <rounds> [poll ms]
# Alternates following and reopening (A/B/A/B) in separate processes so each
# reports its own peak RSS (bytes, /usr/bin/time -l).
bin=$1; log=$2; mib=$3; rounds=$4; poll=${5:-1}
i=0
while [ $i -lt "$rounds" ]; do
  i=$((i+1))
  for mode in follow reopen; do
    /usr/bin/time -l "$bin" grow "$log" "$mib" "$mode" "$poll" 2>/tmp/follow-bench-time.txt
    grep "maximum resident set size" /tmp/follow-bench-time.txt | awk -v m="$mode" '{print "  " m " peak_rss=" $1}'
  done
done
