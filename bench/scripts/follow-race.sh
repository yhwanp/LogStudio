#!/bin/sh
# follow-race.sh <baseline follow-race binary> <new follow-race binary> <log> <writer ms> <attempts> <rounds>
# Alternates the baseline and new core (A/B/A/B) on the same growing log.
base=$1; new=$2; log=$3; every=$4; attempts=$5; rounds=$6
i=0
while [ $i -lt "$rounds" ]; do
  i=$((i+1))
  printf 'base '; "$base" "$log" "$every" "$attempts"
  printf 'new  '; "$new" "$log" "$every" "$attempts"
done
