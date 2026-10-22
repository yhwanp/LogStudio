"""Summarizes zero-conditions.sh output: per input, indexing, analysis and the
field table, the range over runs of each method for every set and condition
count with the conditions found without rows, left open and the blocks
read, whether every check found the methods equal, the per-row and beside
measurements, and the peak RSS of each kind of process.
Usage: python3 zero-conditions-summary.py <dir>/out"""
import collections
import glob
import os
import re
import sys

out = sys.argv[1]
fmt = lambda v: "-" if not v else (f"{min(v):.4f}" if min(v) == max(v) else f"{min(v):.4f}-{max(v):.4f}")
for name in ["a-512.log", "d-512.dlt", "a-2048.log", "d-2048.dlt"]:
    files = sorted(glob.glob(os.path.join(out, f"{name}-*.txt")))
    if not files:
        continue
    print(f"== {name}")
    times = collections.defaultdict(list)
    found = collections.defaultdict(set)
    reads = collections.defaultdict(list)
    checks = collections.Counter()
    rss = collections.defaultdict(list)
    head = None
    rows, beside = [], collections.defaultdict(list)
    for path in files:
        kind = re.sub(r"-\d+$", "", path[len(os.path.join(out, name)) + 1 : -4])
        for line in open(path, encoding="utf-8"):
            f = line.rstrip("\n").split("\t")
            m = re.match(r"\s*(\d+)\s+maximum resident set size", line)
            if m:
                rss[kind].append(int(m.group(1)) / 1e6)
            elif len(f) > 2 and f[1].startswith("index"):
                head = head or line.strip()
                times[("index",)].append(float(f[1].split()[1]))
                times[("analysis",)].append(float(f[2].split()[1]))
            elif len(f) > 8 and f[3] in ("table", "first", "joint", "count"):
                key = (f[1], int(f[2]), f[3])
                times[key].append(float(f[5]))
                found[key].add(f"{f[6]} {f[7]} {f[8]}")
                reads[key].append(int(f[9].split()[1]))
                if len(f) > 10:
                    times[key + ("tables",)].append(float(f[10].split()[1]))
            elif len(f) > 5 and f[3] == "check":
                checks[f[6]] += 1
            elif f[0] == "rows":
                rows.append("\t".join(f[1:]))
            elif f[0] == "beside":
                beside[f[2]].append("\t".join(f[3:]))
    print(head)
    print(f"index {fmt(times[('index',)])}\tanalysis {fmt(times[('analysis',)])}\tchecks {dict(checks)}")
    print("set\tn\tmethod\tseconds\tanswers\tread\ttables")
    for key in sorted(k for k in times if len(k) == 3):
        tables = times.get(key + ("tables",), [])
        print(f"{key[0]}\t{key[1]}\t{key[2]}\t{fmt(times[key])}\t{' / '.join(sorted(found[key]))}\t{min(reads[key])}-{max(reads[key])}\t{fmt(tables)}")
    for line in dict.fromkeys(rows):
        print("rows\t" + line)
    for k, v in beside.items():
        print(f"beside\t{k}\t" + " | ".join(v))
    print("peak RSS MB\t" + "\t".join(f"{k} {fmt(v)}" for k, v in sorted(rss.items())))
    print()
