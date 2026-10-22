#!/bin/sh
# follow-reopen.sh <baseline follow-reopen binary> <new follow-reopen binary> <log> <rounds>
# Opening the same log with the baseline and the new core, alternately.
base=$1; new=$2; log=$3; rounds=$4
i=0
while [ $i -lt "$rounds" ]; do
  i=$((i+1))
  printf 'base '; "$base" "$log"
  printf 'new  '; "$new" "$log"
done
