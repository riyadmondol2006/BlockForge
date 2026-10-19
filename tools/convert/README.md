# Convert tables: how `data/convert/*.z` is generated

BlockForge's world converter is a Rust port of [Chunker](https://github.com/HiveGamesOSS/Chunker) by Hive Games
(MIT License, Copyright (c) 2024 Hive Games). Chunker has no data files: its block, item, biome and entity tables are
Java code (`register(BlockMapping.of(...))` statements, loops, `if (version >= ...)` gates, builders). The files in
`data/convert/` are those tables, executed and packed. Building BlockForge does not need any of this; it is only needed
to regenerate the data.

## Prerequisites

- A JDK, 17 or newer (`javac`, `java`; developed with JDK 25), `python3`, `curl`, `git`.
- A clone of Chunker (read-only), default `../Chunker`. The data in the repository was generated from commit
  `50aaf6d82e8b68929279161b790d95fdddfe0a40` (recorded in the header of every data file).

## Regenerate

```sh
tools/convert/regen.sh ../Chunker [scratch-dir]
```

This runs, in order:

1. `fetch_jars.sh` downloads guava, failureaccess, gson, fastutil, caffeine, jetbrains annotations, jspecify,
   leveldb-api, leveldb, picocli and lz4-java from Maven Central into `<scratch>/jars` (build-time only, all Apache-2.0;
   never committed).
2. `build_dump.sh` compiles `dump/Dump.java` and Chunker's sources with `javac`. Chunker uses a patched leveldb; the
   handful of symbols that differ (`CompressionType.ZLIB_RAW`, `BloomFilterPolicy`, `Options.filterPolicy`,
   `DBIterator.close`) are replaced by the stubs in `dump/stubs/`. They are only needed to compile and never run.
3. `versions.py` lists the version thresholds found in the resolver sources (`isGreaterThanOrEqual(a, b, c)`,
   `new Version(a, b, c)`), plus the probe versions below and a `0.0.0` base. A table at version `v` equals the table at
   the largest threshold `<= v`, so dumping at every threshold captures every distinct state.
4. `Dump.java <out> <edition> <versions...>` instantiates Chunker's own resolvers (`JavaBlockIdentifierResolver`,
   `BedrockBlockIdentifierResolver`, the item resolvers, the legacy resolvers and about 25 small id resolvers) for each
   version and writes their final lookup maps as text, one directory per version (`<out>/<edition>/<version>/*.tsv`),
   plus the state groups and type mappings once per edition, and the pivot (`pivot_states.tsv`, `pivot_types.tsv`).
5. `post.py` packs the dumps into `data/convert/*.z` and, with `--verify`, re-expands the packed files and compares them
   with every dump, so the packing is lossless.

## Plan A (executed Java) rather than Plan B (text parsing)

Chunker's tables are programs, not data: `BlockMapping.flatten/group/merge` expand multimaps, `ChunkerPotionType.values()`
loops generate item rows, `VersionedStateMappingGroup`s switch on the version, `TypeMapping.mapOutputsToInputs` derives
mappings and the `register*` flavours (override, duplicate input/output) change which direction a row lands in. Running
the code gives exactly what Chunker uses; a text tokenizer would have to re-implement Java overload resolution and the
builders and could drift silently. The harness reads the engine's own `inputToChunker` / `chunkerToInput` maps and the
`StateMappingGroup` / `TypeMapping` objects through reflection. Compilation takes about 30 s and a dump of all versions
about 2 s. As a cross-check, the distinct block ids per version match the survey counts (Java 598 / 765 / 901 / 1006 /
1064 / 1099 / 1171 / 1291 at 1.13.2 / 1.16.5 / 1.18.2 / 1.20.1 / 1.21.0 / 1.21.4 / 1.21.11 / 26.3).

## Dump format (per version)

- `blocks.tsv`, `legacy_blocks.tsv`: `I <id> <in-states> <pivot-type> <pivot-states> <group>` (edition -> pivot) and
  `C <pivot-type> <pivot-states> <id> <in-states> <group>` (pivot -> edition), plus `X <group>` for the extra group of
  every block. States are `name=value,...` in Chunker's sorted order; edition values are typed (`b:true`, `i:5`,
  `s:north`), pivot values are Chunker enum constants. Block types are `B:<CONSTANT>` of `ChunkerVanillaBlockType`,
  pivot states are the `VanillaBlockStates` constant (`FACING_HORIZONTAL`), because several constants share a Java name.
- `items.tsv`, `legacy_items.tsv`: the same with item types (`B:` block, `I:` item) and properties (`S:<STATE>`,
  `P:<property>`).
- `simple_<table>.tsv`: `F <constant> <value>` (forward), `I <value> <constant>` (inverse; complete, since several
  constants may share a value), `C <index> <r,g,b,a>` (map colours).
- `groups.tsv`, `types.tsv`: state groups `G`/`S`/`DI`/`DO` and type mappings `T`/`A`/`M`/`R`/`D`. Groups of a
  `VersionedStateMappingGroup` are named `NAME@default` and `NAME@a.b.c`; the version selection is therefore already
  part of each row. Legacy ones are prefixed `legacy:`.

## Packed format (`data/convert/`)

Zlib-compressed text, header lines start with `#` (licence, Chunker commit, description). Rows are
`kind since until ...` with `kind` `B` (both directions: the `I` row and its mirror `C` row), `I` or `C`, and
`since`/`until` versions (empty = open). `Version >= since && < until` selects the rows in force; the Rust loader is
`src/convert/mapping/data.rs`. Sizes: see the end of the output of `post.py`.

## Versions

Dumped at every threshold of the sources. Probes that are always included: Java 1.13.2, 1.16.5, 1.18.2, 1.20.1, 1.20.4,
1.21.0, 1.21.4, 1.21.11 (newest in the source: 26.3.0); Bedrock 1.13.0, 1.16.0, 1.17.30, 1.18.30, 1.19.80, 1.20.0, 1.20.80,
1.21.0, 1.21.50, 1.21.90, 1.21.120, 1.26.0 (newest: 1.26.60).

## Not covered

Block entity, entity (NBT) and item-stack handlers are code, not tables, and are ported by hand in their own tracks.
Chunker's pivot enums themselves (value lists, colours, flags) come from the model track; only what the engine needs
(types -> state sets, state defaults and value lists) is in `pivot.z`.
