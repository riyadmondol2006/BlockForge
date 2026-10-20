#!/usr/bin/env python3
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
"""Generate differential test cases from the dumps: for every block row, a few state combinations drawn from the
type mappings of its state group (valid values, a missing state, an unknown value).

Usage: gen_cases.py <dumps-dir> <out.tsv> [seed]
Case lines are described in Check.java.
"""
import random
import sys
from pathlib import Path

PROBES = {
    ("java", "java"): ["1.13.2", "1.16.5", "1.18.2", "1.20.1", "1.20.4", "1.21.0", "1.21.4", "1.21.11", "26.3.0"],
    ("bedrock", "bedrock"): ["1.13.0", "1.16.0", "1.17.30", "1.18.30", "1.19.80", "1.20.0", "1.20.30", "1.20.40",
                             "1.20.80", "1.21.0", "1.21.50", "1.21.90", "1.21.120", "1.26.0", "1.26.60"],
    ("java", "java_legacy"): ["1.12.0"],
    ("bedrock", "bedrock_legacy"): ["1.13.0"],
}
VARIANTS = 4


def records(path, kind):
    recs, cur = {}, None
    for l in path.read_text().split("\n"):
        if not l:
            continue
        f = l.split("\t")
        if f[0] == kind:
            cur = recs[f[1]] = []
        else:
            cur.append(f)
    return recs


def main():
    dumps, out = Path(sys.argv[1]), Path(sys.argv[2])
    rnd = random.Random(int(sys.argv[3]) if len(sys.argv) > 3 else 1)
    lines = []
    for (edition, mode), versions in PROBES.items():
        groups = records(dumps / edition / "groups.tsv", "G")
        types = records(dumps / edition / "types.tsv", "T")
        # per type: input tuples and output tuples (from M and R rows)
        dom = {}
        for name, rows in types.items():
            ins = sorted({r[1] for r in rows if r[0] in ("M", "R") and r[1] != "~"})
            outs = sorted({r[2] for r in rows if r[0] in ("M", "R") and r[2] != "~"})
            dom[name] = (ins, outs)
        for v in versions:
            blocks = (dumps / edition / v / ("legacy_blocks.tsv" if mode.endswith("legacy") else "blocks.tsv")).read_text().split("\n")
            extra = [l.split("\t")[1] for l in blocks if l.startswith("X\t")]
            extra_g = groups.get(extra[0], []) if extra else []
            for l in blocks:
                if not l or l.startswith("X\t"):
                    continue
                f = l.split("\t")
                gname = f[5]
                glist = groups.get(gname, []) + extra_g
                if f[0] == "I":
                    base = [p for p in f[2].split(",") if p]
                    for _ in range(VARIANTS):
                        st = dict(p.split("=", 1) for p in base)
                        for s in glist:
                            if s[0] != "S":
                                continue
                            names = s[1].split(",")
                            ins = dom[s[3]][0]
                            r = rnd.random()
                            if r < 0.12 or not ins:
                                continue  # leave the state missing
                            if r < 0.18:
                                for n in names:
                                    st[n] = "s:bogus" if rnd.random() < 0.5 else "i:99"
                                continue
                            tup = rnd.choice(ins).split("|")
                            for n, val in zip(names, tup):
                                st.setdefault(n, val)
                        lines.append("\t".join([mode, v, "T", f[1], ",".join("%s=%s" % kv for kv in sorted(st.items()))]))
                else:
                    base = [p for p in f[2].split(",") if p]
                    for _ in range(VARIANTS):
                        st = dict(p.split("=", 1) for p in base)
                        for s in glist:
                            if s[0] != "S":
                                continue
                            names = s[2].split(",")
                            outs = dom[s[3]][1]
                            r = rnd.random()
                            if r < 0.12 or not outs:
                                continue
                            tup = rnd.choice(outs).split("|")
                            for n, val in zip(names, tup):
                                st.setdefault(n, val)
                        lines.append("\t".join([mode, v, "F", f[1], ",".join("%s=%s" % kv for kv in sorted(st.items()))]))
    lines = sorted(set(lines))
    out.write_text("\n".join(lines) + "\n")
    print(len(lines), "cases")


main()
