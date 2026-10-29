# Porting Chunker to Rust

BlockForge's world converter is a Rust port of [Chunker](https://github.com/HiveGamesOSS/Chunker) by Hive Games
(MIT License, Copyright (c) 2024 Hive Games). The Java source is read from a clone at `../Chunker` (read-only
reference, only `cli/src/main` and `cli/src/test/java`; never port `app/`, never commit `cli/data`). This file is the
map: every Chunker package, where it goes, and its status. Update it with every phase so a new session can resume.

Status marks: `[ ]` not started, `[~]` in progress, `[x]` done, `[-]` out of scope.

Survey notes with the full detail (class inventories, algorithms, risks) are in `docs/porting/` (copies of the four
surveys: core, model, java, bedrock).

## Decisions

1. **Pivot = Chunker's own model.** The version-neutral block is `(BlockType, states)` where `BlockType` is Chunker's
   `ChunkerVanillaBlockType` (about 1,290 values) and the states are Chunker's `VanillaBlockStates` (about 133) with
   their value enums. This keeps every ported table valid as it is. Rust: generated `BlockType`/`StateId` enums
   (`src/convert/model/generated.rs`, produced by `tools/convert/gen_model.py` from the Java enums).
2. **Tables are generated, not hand-ported.** Chunker has no data files: its identifier tables are Java code
   (`register(BlockMapping.of(...))`, about 900 block rows per edition, state groups, type mappings, item, biome and
   entity tables, each gated by version). `tools/convert/dump/` (a small Java harness) compiles Chunker's sources with
   javac and dumps the resolver tables, per version, as plain text; `tools/convert/post.py` packs them into
   `data/convert/*.bin.z` (checked in, loaded lazily with `include_bytes!` + `LazyLock`). The mapping engine
   (`mapping/engine.rs`, port of `ChunkerBlockIdentifierResolver` and the state-group classes) interprets them.
   Regenerating needs a JDK and the Chunker clone; building BlockForge does not.
3. **Bedrock LevelDB = `rusty-leveldb` 4.0.2** with a 20-line raw-deflate `Compressor` (id 4; zlib id 2) built on
   `flate2`, behind our own `BedrockDb` trait. No C code. Always open a scratch copy of `db/` (it opens read-write).
4. **Inheritance chains become data.** Chunker's v1_11 ... v26_3 (Java) and v1_13 ... v1_26_60 (Bedrock) packages are
   linear `extends` chains that override a few switches; in Rust that is one `JavaFormat`/`BedrockFormat` struct
   derived from the data version (`format.rs`).
5. **Scheduler.** Chunker's task tree and 8-thread pool become a bounded `std::thread` worker pool over regions
   (Bedrock: batches of 4 regions) with an explicit progress counter and a cancel flag (`convert/task.rs`).
6. **Settings.** `ChunkerLevelSettings` (about 106 reflected fields) becomes a plain struct with the few fields we
   expose (target version, dimensions, keep-unknown-NBT flag); the rest default as in Chunker.
7. **Out of scope:** the Electron app, custom mapping files (`MappingsFile` JSON), map previews, the pruning UI,
   `cli/messenger`, picocli.
8. **Feature flag.** Everything lives behind the cargo feature `converter` (on by default);
   `cargo build --no-default-features` must compile.
9. **Tests that need Chunker's data** read `CHUNKER_DATA` (default `../Chunker/cli/data`) and skip cleanly when it is
   missing. Never read it into a context, never commit it. Fixture worlds in `tests/fixtures` stay under 1 MB each.
10. **Credits.** Every ported file starts with: `Derived from Chunker (https://github.com/HiveGamesOSS/Chunker),
    Copyright (c) 2024 Hive Games, MIT License.` NOTICE.md has the full licence text; README, the Convert screen and
    About credit Chunker (all through `src/i18n`), and say BlockForge is not affiliated with Hive Games.

## Milestones

- **M1 (first usable): Bedrock -> Java**, modern worlds (Bedrock 1.20 - 1.26.x to Java 1.20/1.21): blocks, biomes,
  heights, chests/signs/command blocks/pistons/furnaces/beds/banners/skulls, items in containers, level.dat basics.
  Test world: `~/Downloads/Mace, Spear, Sword PvP Map! V4.mcworld` (Bedrock 1.26.51, 93 columns, v9 sub-chunks).
- **Out of scope (decided by the owner): Java -> Bedrock.** BlockForge converts Bedrock worlds to Java Edition only.
- **M3:** entities (done, see Phase 5), players, maps, legacy (< 1.13) Java, older Bedrock, Java <-> Java version
  changes.

## Phases

### Phase 0 - skeleton  [x]
- [x] `PORTING.md`, cargo feature `converter`, `src/convert/{mod,nbt}`, NOTICE entry

### Phase 1 - foundations, in parallel (independent file sets)
| Track | Chunker source | Rust destination | Status |
|---|---|---|---|
| NBT | `nbt/**`, `util/**` (IO, tags, LZ4Block for MCA type 4, SNBT if used) | `src/convert/nbt/**`, `src/convert/util/**` | [x] |
| Model | `conversion/intermediate/column/**`, `mapping/identifier/**` | `src/convert/model/**` (+ generated enums via `tools/convert/gen_model.py`) | [x] |
| Table dumper | `conversion/encoding/{java,bedrock}/**/resolver/**`, `mapping/**` (engine semantics) | `tools/convert/dump/**`, `data/convert/*.bin.z`, loader `src/convert/mapping/data.rs` | [x] |
| Bedrock low level | `encoding/bedrock/util/**`, `base/reader` db/palette parts | `src/convert/bedrock/{db,keys,palette,level_dat,data_version,detect}.rs` (+ `tests.rs`: real-world tests) | [x] |
| Java low level | `encoding/java/util/**`, `base/reader|writer` region parts, `level` I/O | `src/convert/java/{region,palette,dirs,version,format}.rs` | [x] |

### Phase 2 - the Bedrock -> Java path (M1)
- [x] `mapping/engine.rs` (port of `ChunkerBlockIdentifierResolver`, `StateMappingGroup`, `TypeMapping`, versioned groups)
- [ ] `bedrock/world_reader.rs`, `bedrock/level_reader.rs` (columns, Data3D, sub-chunks v8/v9, level.dat)
- [ ] `bedrock/resolvers/{block,item,biome}.rs`, block entities (core set), item stacks
- [ ] `bedrock/pretransform/**` (fence, pane, wall, stairs, door, gate, redstone, fire, chorus, vine, chest, snowy, ...)
- [ ] `java/{chunk,level}/write.rs`, Java resolvers (block, item, biome, block entity, item stack)
- [ ] `convert/{pipeline,task,settings}.rs` (pipeline, worker pool, progress, cancel), detect(), `convert_world()` API
- [ ] CLI example `examples/convert_world.rs`, round-trip/golden tests, the test world

### Phase 3 - Java -> Bedrock: cancelled (Bedrock -> Java only)

### Phase 4 - GUI, credits, docs
- [~] `src/gui/convert.rs` page (page + Backend trait done; engine not wired) + entry on an instance's worlds list; progress, cancel, summary; i18n strings
- [x] Footer "Powered by Chunker by Hive Games (MIT)" (opens the repo with `open`), About credits
- [~] README (feature + Credits + non-affiliation), NOTICE.md, CHANGELOG.md, TESTING.md
- [ ] validation tests ported (block/item/biome identifier coverage per version) reading `CHUNKER_DATA`
- [ ] release-size check (growth < 5 MB), clippy `-D warnings`, fmt

### Phase 5 - later
- [x] entities: Bedrock actors from both stores (the legacy chunk `Entity` record and `digp`/`actorprefix`), read a
  dimension at a time and put in the column they stand in (`bedrock/reader_entities.rs`), typed and filtered by
  `bedrock/resolvers/entity.rs`, and item frames from their block entities; written per Java version by
  `java/resolvers/entity/` (`entities/` region files from 1.17, `Level.Entities` before) with items through the same
  writer and hooks as containers. The version switches were checked with the game's own DataFixer: an entity written
  for an older version and upgraded by the 26.3 game equals what is written for 26.3. Not converted: passengers,
  armor stand poses other than the default and the zero pose, villager professions for 1.13.
- [x] players: the host (`~local_player`) with inventory, armor, off hand, ender chest, experience, health, food,
  respawn point, effects and abilities (`bedrock/level_reader.rs` `parse_player`, `java/level_writer.rs`
  `fill_player`, one format per Java version); the other profiles (`player_server_<uuid>`, `player_<id>`) as their own
  player files; owner account, inventory source and what is kept are `ConversionSettings::player`
  (`src/convert/pipeline_players.rs` applies them)
- [x] maps: items keep their link (`minecraft:map_id` / `tag.map`). The reader gives each `map_uuid` the index of its
  map in id order (`with_map_indexes`), the writer's `WorldItemHooks` (`java/resolvers/hooks.rs`, from
  `JavaWorldWriter::item_hooks`) turns the index into the Java id, which is that same index; the hooks also write the
  block entity data of items (shulker box contents). Every item path must use them: chunk block entities
  (`EncodeContext::hooks`), the players (`LevelFiles::hooks`) and entities (`EntityContext::hooks`)
- [x] level setting overrides (`ConversionSettings::level_overrides`, applied with `changed_settings` by
  `apply_to_level_settings` in the pipeline; 26.1+ difficulty is `difficulty_settings.difficulty`)
- [ ] portals, lodestones
- [ ] legacy Java (< 1.13) numeric ids, older Bedrock, Java <-> Java up/downgrade

## Package map (Chunker -> Rust)

| Chunker package (`com.hivemc.chunker.`) | Lines | Rust | Status |
|---|---:|---|---|
| `nbt.tags.**`, `nbt.io.**`, `nbt` | ~4,000 | `convert/nbt/` | [x] |
| `util`, `scheduling.**` | ~2,300 | `convert/util/`, `convert/task.rs` | [~] util done (scheduling pending) |
| `conversion` (WorldConverter), `conversion.handlers.**` | ~1,200 | `convert/pipeline.rs` | [ ] |
| `conversion.intermediate.level/world` | ~1,900 | `convert/model/level.rs` | [ ] |
| `conversion.intermediate.column.**` | ~11,400 | `convert/model/**` (~9k Rust incl. 3.2k generated; `tools/convert/gen_model.py`) | [x] |
| `mapping.identifier.**` | ~460 | `convert/model/identifier.rs` | [x] |
| `mapping.resolver/mappings` (user JSON files) | ~1,100 | [-] later | [-] |
| `conversion.encoding.base.**` | ~3,900 | `convert/mapping/`, `convert/resolve/` | [ ] |
| `conversion.encoding.java.**` (v1_11 ... v26_3) | ~25,000 | `convert/java/**` + `data/convert/java*` | [ ] |
| `conversion.encoding.bedrock.**` (v1_13 ... v1_26_60) | ~30,000 | `convert/bedrock/**` + `data/convert/bedrock*` | [ ] |
| `resolver.**` | ~900 | `convert/mapping/engine.rs` | [ ] |
| `conversion.encoding.preview`, `pruning`, `cli.**` | ~3,000 | [-] out of scope | [-] |


## Status log

- Wave 1 (merged): NBT+util, level model + settings + task scheduler, block/column model (+generated enums), mapping
  tables + engine (data in `data/convert/`, generated by `tools/convert/`), Bedrock low level (LevelDB, keys, palettes,
  level.dat, detect), Java low level (regions, palettes, formats, dirs), GUI page + credits + docs (behind the
  `Backend` trait in `src/convert/backend.rs`, currently `NotReady`).
- Wave 2 (in progress): Bedrock pre-transforms, Bedrock items + block entities (read), Java items (write), Java block
  entities (write), Bedrock world/level reader, Java world/level writer. Contracts: `docs/porting/CONTRACTS.md`.
- Next: pipeline (`convert_world`), wire the GUI backend, test world end to end, then the remaining Bedrock -> Java gaps (entities are done). Java -> Bedrock is not planned.
- Players, map links and level overrides done (Phase 5): see the Phase 5 list. The converter page shows every option
  in its "What comes along" step (`ConvertJob::entities`, `ConvertJob::player`, `ConvertJob::level_overrides`;
  default = everything kept, the launcher's account as the owner), and so does `examples/convert_world.rs`.
