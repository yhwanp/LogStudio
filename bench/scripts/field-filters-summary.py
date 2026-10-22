"""Summarizes field-filters.sh output: per input, the range over runs of
indexing, analysis, peak RSS and each filter for every build label (with and
without the field table), and whether every run of every build counted the
same rows. Runs of the same input in several folders are merged.
Usage: python3 field-filters-summary.py <dir>/out [more folders]"""
import collections
import glob
import os
import re
import sys

dirs = sys.argv[1:]
for name in ["a-512", "w-512", "a-2048", "w-2048"]:
    files = sorted(f for d in dirs for f in glob.glob(os.path.join(d, f"{name}-*-[0-9]*.txt")))
    files = [f for f in files if not f.endswith("-collect.txt")]
    if not files:
        continue
    times = collections.defaultdict(list)
    counts = collections.defaultdict(set)
    labels = []
    order = []
    extra = []
    for path in files:
        label = None
        for line in open(path, encoding="utf-8"):
            f = line.rstrip("\n").split("\t")
            if label is None and len(f) > 1:
                label = f[0]
                if label not in labels:
                    labels.append(label)
            m = re.match(r"\s*(\d+)\s+(maximum resident set size|peak memory footprint)", line)
            if m:
                key = "peak RSS MB" if m.group(2).startswith("max") else "peak footprint MB"
                times[(label, key)].append(int(m.group(1)) / 1e6)
            elif len(f) > 4 and f[1] in ("index EOF", "analysis"):
                times[(label, f[1])].append(float(f[4]))
                h = re.search(r"heap in use (\d+) KiB", line)
                if h and f[1] == "analysis":
                    times[(label, "heap after analysis MB")].append(int(h.group(1)) / 1024)
            elif len(f) > 2 and f[1] == "field table":
                extra.append(line.strip())
            elif len(f) > 5 and f[2] in ("meta", "nometa"):
                key = f[1]
                if key not in order:
                    order.append(key)
                times[(f"{label} {f[2]}", key)].append(float(f[4]))
                counts[key].add(f[5])
    fmt = lambda v: "-" if not v else (f"{min(v):.3f}" if min(v) == max(v) else f"{min(v):.3f}-{max(v):.3f}")
    print(f"== {name} ({len(files)} runs)")
    for key in ["index EOF", "analysis", "peak RSS MB", "peak footprint MB", "heap after analysis MB"]:
        print(key + "".join(f"\t{label} {fmt(times[(label, key)])}" for label in labels))
    for line in extra[:1]:
        print(line)
    modes = [f"{label} {mode}" for label in labels for mode in ("meta", "nometa")]
    print("filter\t" + "\t".join(modes) + "\tcount")
    for key in order:
        same = "same" if len(counts[key]) == 1 else f"DIFFERENT {sorted(counts[key])}"
        cells = "\t".join(fmt(times[(mode, key)]) for mode in modes)
        print(f"{key}\t{cells}\t{next(iter(counts[key]))} {same}")
    for path in (f for d in dirs for f in glob.glob(os.path.join(d, f"{name}-collect.txt"))):
        print(open(path, encoding="utf-8").read().strip())
    print()
