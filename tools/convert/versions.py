#!/usr/bin/env python3
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
"""Print the version thresholds that gate Chunker's resolver tables, one per line (java or bedrock).

Every gate in the identifier/biome/entity/item resolvers is `version.isXxx(a, b, c)` or `new Version(a, b, c)`
(state groups). A table at version v equals the table at the largest threshold <= v, so dumping at the thresholds
(plus the next patch for strict comparisons, plus a 0.0.0 base) captures every distinct state.
"""
import re
import sys
from pathlib import Path

PROBES = {
    "java": ["1.13.2", "1.16.5", "1.18.2", "1.20.1", "1.20.4", "1.21.0", "1.21.4", "1.21.11"],
    "bedrock": ["1.13.0", "1.16.0", "1.17.30", "1.18.30", "1.19.80", "1.20.0", "1.20.80", "1.21.0", "1.21.50",
                "1.21.90", "1.21.120", "1.26.0"],
}
PAT = re.compile(r"(?:(isGreaterThanOrEqual|isLessThanOrEqual|isGreaterThan|isLessThan)|new Version)\(\s*(\d+),\s*(\d+),\s*(\d+)\s*\)")


def main():
    chunker, edition = Path(sys.argv[1]), sys.argv[2]
    root = chunker / "cli/src/main/java/com/hivemc/chunker/conversion/encoding" / edition / "base/resolver"
    found = set()
    for f in root.rglob("*.java"):
        if "blockentity" in f.parts or "handlers" in f.parts:
            continue
        for m in PAT.finditer(f.read_text()):
            t = tuple(int(x) for x in m.group(2, 3, 4))
            found.add(t)
            if m.group(1) in ("isGreaterThan", "isLessThanOrEqual"):
                found.add((t[0], t[1], t[2] + 1))
    for p in PROBES[edition]:
        found.add(tuple(int(x) for x in p.split(".")))
    found.add((0, 0, 0))
    for t in sorted(found):
        print("%d.%d.%d" % t)


main()
