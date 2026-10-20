# S2 - Chunker intermediate model survey (column/**, mapping/identifier/**)

Source root: `cli/src/main/java/com/hivemc/chunker`. Area = 187 files / 11,399 lines under
`conversion/intermediate/column` + 5 files / 463 lines in `mapping/identifier`. Of the 11.4k lines ~2.9k are
enum-constant tables (block 1,390, item 501, entity 200, biome 234, groups 350, states ~900 of tiny enums).
Real logic is ~3k lines (palette 584, biome layouts 557, ChunkerBlockIdentifier 295, ChunkerColumn 290, StructureBE 504...).

## 1. Data model in one page

```
ChunkerColumn (one 16x16 column, all Y)      <- unit that flows reader -> handlers -> writer
  position: ChunkCoordPair(chunkX,chunkZ)  record, helpers: to10BitIndex, toLong, regionX/Z, isRegionEdge
  chunks:  Byte2ObjectSortedMap<ChunkerChunk>   key = section Y index as i8 (AVL sorted; iterated top->bottom)
  biomes:  @Nullable ChunkerBiomes              (4 interchangeable layouts, see 3)
  heightMap: @Nullable HeightMap                (marker iface; JavaLegacy{short[16][16]} | Bedrock{short[16][16]}; opaque, same-format only)
  entities: List<Entity>        (absolute double coords)   -- NOT per chunk section
  blockEntities: List<BlockEntity> (absolute int x,y,z)    -- NOT per section; lookup is linear scan (x&15,z&15, full y)
  lightPopulated: bool          (Java isLightOn / LightPopulated)
  requiredPreTransformEdges: EnumSet<Edge>  + preTransformHandler: Consumer<Map<Edge,ChunkerColumn>> (chained via andThen, run once, then nulled)
ChunkerChunk (16^3 section)
  y: i8;  palette: Palette<ChunkerBlockIdentifier> (default SingleValuePalette(AIR))
  blockLight/skyLight: @Nullable byte[16][16][16]  ([x][y][z], one nibble per byte; ONLY legacy-Java reader populates; Bedrock ColumnUtil reads)
  isEmpty(): palette.isEmpty() || !palette.containsKey(!isAir)   (KEY check, not voxel scan -> stale unused keys count as non-empty)
```
Absent from the model (nothing to port): scheduled ticks, per-chunk status/InhabitedTime, structures, POI, carving masks,
per-chunk entity lists, Java-1.17+ heightmaps (writers recompute or require same-format), custom-block block entities
(`ChunkerCustomBlockType.getBlockEntityClass() = empty`). Dimension (`intermediate/world/Dimension`: javaId?, bedrockId,
identifier, fallbackBiome, biomeHeight 24/8/16) is outside the column but biome code uses it.

ChunkerColumn API worth keeping: getBlock(x,y,z) (AIR if section missing), setBlock (creates section filled with AIR,
`asWriteable`), getHighestBlock(x,z,predicate) (walks sections top->bottom, uses palette.containsKey as quick reject, then y 15..0),
getBlockEntity(x,y,z). setBlock is the main MUTATION path used by block-entity handlers (flower pot, bed, banner, skull,
note block, lectern HAS_BOOK, jukebox HAS_RECORD, chest CHEST_TYPE, piston EXTENDED, copper golem pose, item-frame entity->block)
and by `PreTransformManager` (neighbour-aware redstone/wall/fence/fire fix-up).
Other mutators: readers (set palette = `mergedPalette.map(resolvers::readBlock).compact(AIR)`, add BEs/entities, set biomes/heightmap/light),
writers (`chunk.setPalette(palette.compact(AIR))` before serialising), `ChunkerBiomes.remap(fn)` (writers substitute unsupported biomes via
`ChunkerVanillaBiome.getFallback()`), `BlockEntityResolver.updateBeforeProcess/Write` (handler chain may replace a BlockEntity).

## 2. Palette (chunk/palette, 584 lines) - the memory-critical piece
Interfaces: `Palette<T>` {getKeyCount, getKey(i,default), get(x,y,z,default), map(f)->Palette<U>, containsValue(pred) (SCANS voxels),
containsKey(pred) (keys only), isEmpty (no keys), asWriteable, compact(default), copy}; `WriteablePalette<T>` adds
getOrCreateKey(T)->short, set(x,y,z,T), setPaletteIndex(x,y,z,short).
Impls: `EmptyPalette` (no keys; get->default; singleton for dim 16), `SingleValuePalette(dim,value)` (get always value; asWriteable allocates),
`ShortBasedPalette(List<T> keys, short[dim][dim][dim] values)` = 8 KiB/section regardless of key count; keys searched by indexOf (O(n)).
Dimension 16 for blocks and per-block biomes, 4 for biome cells (4x4x4 per section).
Quirks to preserve: (a) out-of-range index (<0 or >=keys) is legal and reads as `default`; compact() rewrites those to `defaultValue`.
(b) compact(): remaps indices in first-seen voxel order, drops unused keys, dedups equal keys, returns Single if 1 key, Empty if 0 keys.
(c) `ShortBasedPalette.map()` SHARES the voxel array with the source (aliasing) - in Rust make map consume `self`.
(d) writers read the raw array: java v1_13/v1_18 ChunkWriter, bedrock PaletteUtil call `getValues()` -> expose packed-iteration + `to_u16_vec()` instead.
(e) iteration order of the Java array is [x][y][z]; on-disk orders differ per format (Java YZX, Bedrock XZY) - converters transpose.

## 3. Biomes (biome/, 841 lines)
`ChunkerBiome` iface -> `ChunkerVanillaBiome` enum (~90 constants; fields: javaId?, bedrockId?, fallback?; deprecated ones map to a fallback;
static InvertibleMap string->biome, Java name wins, bedrock alias added if different; `find(String)`) | `ChunkerCustomBiome(String id)` (equals by id, no fallback).
`ChunkerBiomes` iface (5 methods: asColumn(fallback)->256, as4X4(fallback)->1024, asPalette()->List<Palette 16^3>, as4X4Palette(chunkY)->Palette 4^3, remap(fn)),
four layouts which are lossy converters between each other (ported line-for-line; index math is the whole content):
- ColumnBased `ChunkerBiome[256]` idx z<<4|x (legacy Java). as4X4 samples (x*4+2,z*4+2) and replicates over 64 layers.
- ClusterBased `[1024]` idx chunkY<<6|y4<<4|z4<<2|x4 (Java 1.15-style / bedrock 2D+). asColumn uses only first 16 cells.
- PaletteBased `List<Palette16>` per section (Bedrock 3D). as4X4 samples cell centres (+2); chunkY beyond list clamps to last palette.
- ClusterPaletteBased `List<Palette4>` per section (Java 1.18+). asPalette upsamples 4->16 by cell replication; empty/single palettes shortcut.
Rust: `enum Biomes {Column(Box<[Biome;256]>), Cluster(Box<[Biome;1024]>), Palette(Vec<Palette<Biome,16>>), ClusterPalette(Vec<Palette<Biome,4>>)}`.

## 4. Block identifiers (chunk/identifier, 4,363 lines)
`ChunkerBlockIdentifier { type: ChunkerBlockType, states: Map<BlockState<?>,BlockStateValue>, preserved: @Nullable PreservedIdentifier }`
immutable; `AIR` constant; equals/hash over all three. API: getState(state[,returnDefault=true]) (default returned only if the TYPE's state set contains it),
containsState, getPresentStates, copyWith(state,val), copyWithout(state), isAir (type identity), getRGBColor, asFilledChunkPalette,
`custom(String id, Map<String,Object> states)` (any key named "waterlogged" with true/"true"/1 becomes the vanilla WATERLOGGED=Bool; other keys become one-off
`BlockState` with a single `CustomBlockStateValue<Object>`; type = new ChunkerCustomBlockType(id, those states)).
`ChunkerBlockType` iface {getRGBColor, isAllFacesSolid, getStates():Set<BlockState>, getBlockEntityClass()} with 2 impls:
- `ChunkerVanillaBlockType` enum, **1,292 constants** (~1,390 lines): (rgbMapColor, stateSet, [BlockEntity.class], allFacesSolid). State set = union(DEFAULT={WATERLOGGED}, given). 198 constants name a BE class
  (banner 49, sign 39, hanging sign 39, shulker 17, bed 16, skull 14, shelf 13, chest 9, ... one-offs). Contains modded/future entries (POPLAR_*, *_WOOL_STAIRS, ICICLE, DISC_PRESS, SULFUR_CUBE...) - it is Chunker's own superset list, not mojang's.
- `ChunkerCustomBlockType(id, states)` equals by id; colour 0; no BE; solid=false.
`ChunkerVanillaBlockGroups` (350 lines): 10 `Set<ChunkerBlockType>` (DOORS 22, FENCE_GATES 13, BARS_AND_GLASS_PANES 26, REDSTONE_CONNECTABLE ~70, SNOWY_BLOCKS, SNOWY_GRASS_BLOCKS, STAIRS ~100, WALLS ~35, WOODEN_FENCES, DOUBLE_CHESTS) used only by reader pre-transform handlers.
`PreservedIdentifier(boolean fromReader, mapping.Identifier identifier)`: carries a USER-supplied custom block/item mapping (mappings JSON) from reader to writer; unmapped blocks
become STONE placeholder + preserved id. The side that did NOT create it applies it on write (`reader == preserved.fromReader` => ignore).
Items: `ChunkerItemStackIdentifier` iface {getItemStackType(), isAir()} implemented by ChunkerBlockIdentifier (block-items keep states!) and by `ChunkerItemType`
(`ChunkerVanillaItemType`, ~470 constants, item-only things; 64-odd carry a BE class e.g. signs). No custom item type: unknown items ride in `ChunkerItemStack.preservedIdentifier`.
`ChunkerItemStackIdentifierType` = {Optional<Class<? extends BlockEntity>> getBlockEntityClass()} -> in Rust replace Class<> by a fieldless `BlockEntityKind`.

### 4a. Block states
`BlockState<T extends BlockStateValue>{name, defaultValue, T[] values}`; equals = name + values deepEquals; hash = name only; Comparable by name.
`BlockStateValue` is an empty marker iface; the 70 value types are Java enums (`.../states/vanilla/types`), plus `CustomBlockStateValue<T>`.
`VanillaBlockStates`: 133 `BlockState` constants (+ `DEFAULT_BLOCK_STATES={WATERLOGGED}`); ~68 of them are Bool. Names collide on purpose across types:
`facing`x3 (FacingDirection 6 / Horizontal 4 / HorizontalDown 5), `age`x8 (Age_1,2,3,4,5,7,15,25), `level`x4 (Cauldron, Composter, Light, Liquid), `shape`x3 (Rail, RailStraight, Stair),
`type`x3 (Chest, Piston, Slab), `mode`x3 (Comparator, Structure, Test), `axis`x2, `distance`x2 (Distance, Stability), `east/north/south/west`x3 (Bool, WallHeight, RedstoneConnection).
`VanillaBlockStateGroups`: 36 `Set<BlockState>` constants (BED, DOOR, STAIRS, WALL, CHEST, SKULL, SHELF, ...) = pure convenience unions.
Value enums (70 files): ordinal order == `values()` order and is relied on (StateMapping builds cartesian product of `getValues()`; DyeColor-like ordinal IDs).
Only 7 enums have methods: FacingDirection (from3DByte/from2DByte/to3DByte/to2DByte, order DOWN,UP,NORTH,SOUTH,WEST,EAST), FacingDirectionHorizontal
(NORTH,EAST,SOUTH,WEST with x/z offsets; rotateClockwise/AntiClockwise, isAdjacent, asDirection->BlockPreTransformHandler.Direction, from2DByte/to2DByte
with SOUTH=0,WEST=1,NORTH=2,EAST=3), FacingDirectionHorizontalDown (5 plain), Half, PistonType, WallHeight, Layers (trivial extras). The rest are pure constant lists.
Complete list of value types (70): Age_1/2/3/4/5/7/15/25, AttachmentType, Axis, AxisHorizontal, BambooLeafSize, BedPart, BellAttachmentType, Bites, Bool, Candles, CauldronLevel,
ChestType, ComparatorMode, ComposterLevel, CopperGolemPose, CoralFanDirection, Creaking, Delay, Distance, DripstoneThickness, Dusted, Eggs, FacingDirection,
FacingDirectionHorizontal, FacingDirectionHorizontalDown, Flowers, GrindstoneAttachmentType, Half, Hatch, HingeSide, HoneyLevel, Layers, LightLevel, LiquidLevel, Moisture, Note(0-24),
NoteBlockInstrument(27), Orientation(12), Pickles, PistonType, PotentSulfurState, Power(0-15), RailShape(10), RailShapeStraight, RedstoneConnection, RehydrationLevel, RespawnAnchorCharges,
Rotation(0-15), SculkSensorPhase, Segments, SideChain, SlabType, StabilityDistance, Stage, StairShape, StructureBlockMode, StructureVoidType, TestBlockMode, Tilt, TrialSpawnerState,
VaultState, VerticalDirection, WallHeight. Numeric ones are named `_N` (Java identifier quirk); only their count and ordinal matter for port.

## 5. mapping/identifier (463 lines): version-neutral string form
`Identifier{String identifier; Map<String,StateValue<?>> states}` (hash/equals over both; `isVanilla` = no ':' or "minecraft:" prefix; `fromData(id, OptionalInt)` stores legacy meta as state key `"data"` Int;
`getDataValue()`; `fromBoxed`). `StateValue<T>` sealed-ish iface: `StateValueBoolean`(TRUE/FALSE singletons) | `StateValueInt` | `StateValueString`;
`getBoxed()`, `toNBT()` (Byte 0/1, Int, String), `fromNBT` (ByteTag: ==1 -> true else false; Int; String; others throw), `fromBoxed` (Boolean|String|Number->intValue), Gson adapter
(primitive bool/string/number; a one-element list serialises as scalar). Used for: reader/writer raw names, mapping-file JSON, PreservedIdentifier. toString state order is hash-map order (non-deterministic in Java) - use BTreeMap in Rust and do not rely on string order.

## 6. Block entities (blockentity/, 54 files, 2,664 lines)
Base `BlockEntity`: x,y,z (i32 absolute), isMovable (Bedrock), customName: @Nullable JsonElement (text component), originalNBT: @Nullable CompoundTag (only set/used when `--keepOriginalNBT`
= converter.shouldAllowNBTCopying(); the Java handlers copy every unconsumed tag from it back out). equals ignores originalNBT. Hierarchy matters: resolvers dispatch by class and walk to super handlers.
`ContainerBlockEntity`: `Byte2ObjectAVLTreeMap<ChunkerItemStack> items` (slot->stack, sparse). `RandomizableContainerBlockEntity`: + @Nullable lootTable String.
52 concrete classes (+3 abstract). Fields beyond base (E = empty marker class, 7 lines):

| Class (parent) | Fields |
|---|---|
| Banner | patterns: List<(DyeColor, BannerPattern)>, base: Optional<DyeColor> (Java: shields only) |
| Beacon | primaryEffect, secondaryEffect: EffectType (default EMPTY) |
| Bed, Beehive, Bell, Conduit, CopperGolemStatue, CreakingHeart, DaylightDetector, EnchantmentTable, PotentSulfur, SculkCatalyst, SculkShrieker, SculkSensor, CalibratedSculkSensor(SculkSensor), EndPortal, Test, TestInstance, TrialSpawner, Vault, Hopper(Container), EnderChest(Container), BlastFurnace/Smoker(Furnace), Barrel/Chest/Dispenser/ShulkerBox(Randomizable), Dropper(Dispenser), TrappedChest(Chest) | E (no extra fields; state-bearing data lives in block states) |
| Brushable | brushDirection: i8, brushCount: i32, item: ItemStack, lootTable: String |
| Campfire | items: ItemStack[4], cookingTimes: i32[4] |
| Cauldron | potionType: PotionType(EMPTY), potionBottleType (UNKNOWN), customColor: Color |
| ChiseledBookshelf | books: ItemStack[6] |
| CommandBlock | command: String, auto: bool, trackOutput: bool (+ base customName) |
| Comparator | outputSignal: i32 |
| BrewingStand(Container) | brewTime: i16, fuel: i16, fuelTotal: i16 (=20) |
| Furnace(Container) | burnTime, cookTime, cookTimeTotal: i16 |
| Crafter(Randomizable) | disabledSlots: i16, craftingTicksRemaining: i32 |
| DecoratedPot | back,left,right,front: ItemStackIdentifier (default BRICK), item: ItemStack |
| EndGateway(EndPortal) | age: i64, exitX/Y/Z: i32 |
| Jigsaw | name,target,pool="minecraft:empty", finalState="minecraft:air", joint: Joint{ROLLABLE,ALIGNED}, selectionPriority, placementPriority: i32 |
| Jukebox | record: ItemStack |
| Lectern | book: ItemStack, page: i32 |
| Shelf | items: ItemStack[3] |
| Sign / HangingSign(Sign) | front,back: SignFace{lines: List<Json>, lit: bool, color: i32 = 0xff000000(ARGB)}, waxed: bool, allowOpFeatures: bool=true |
| Skull | ownerId, ownerName, texture, textureSignature: String (all optional) |
| Spawner | delay, spawnRange, maxSpawnDelay, minSpawnDelay, spawnCount, requiredPlayerRange, maxNearbyEntities: i16; entityType: @Nullable EntityType |
| Structure | name, author, metadata: String; pos/size XYZ: i32 x6; rotation{NONE,CW90,CW180,CCW90}, mirror{NONE,LEFT_RIGHT,FRONT_BACK,BOTH(bedrock)}, mode: StructureBlockMode(DATA); ignoreEntities, includePlayers, removeBlocks, strict, powered, showAir: bool; showBoundingBox=true; integrity: f32=1; seed, lastTouchedPlayerId: i64; animationMode: i8, animationSeconds: f32, redstoneSaveMode: i32 |
| Unknown | type: String (raw id; passthrough, relies on originalNBT) |

(Counting the table: 38 non-container + 13 container = 51 + Unknown.) The task brief's "33 handlers" is the Java/Bedrock handler count, not class count.
Dependencies of BEs on other model types: ItemStack, DyeColor, PotionType/BottleType, EffectType, EntityType, StructureBlockMode (a state enum), Color, JsonElement, CompoundTag.

## 7. Entities (entity/, 975 lines)
Base `Entity`: positionX/Y/Z f64, motionX/Y/Z f64, yaw f32, pitch f32, originalNBT: @Nullable CompoundTag, abstract getEntityType(). Only FIVE concrete classes:
`HangingEntity`(abstract: tileX/Y/Z i32) -> `ItemFrameEntity`{item: ItemStack, direction: FacingDirection, itemRotation: i8, map: bool, photo: bool} -> `GlowItemFrameEntity` (type only);
`PaintingEntity`{motive: Motive (50 constants with width,height; KEBAB..UNPACKED), direction: FacingDirectionHorizontal=SOUTH};
`CushionEntity`{color: DyeColor=WHITE}; `UnknownEntity(type)`. Everything else (mobs, minecarts, boats, armor stands...) is UnknownEntity + originalNBT (so NBT copy is the only passthrough) + position/motion/rotation.
`ChunkerEntityType` iface = `ChunkerVanillaEntityType` enum (166 constants, flag vanillaSpawnEgg) | `ChunkerCustomEntityType(String id)` (no equals override in Java - a bug; implement Eq by id).

## 8. Item stack model (chunk/itemstack, 1,404 lines)
`ChunkerItemStack extends PropertyHolder<ChunkerItemStack>`: identifier (ChunkerItemStackIdentifier) + @Nullable PreservedIdentifier + a typed heterogeneous bag
`Map<Property<..,V>, Object>` (resolver/property: Property registers itself in a static per-class name table; duplicate name throws; used as string<->property lookup).
NO components, NO raw NBT blob: Java components/NBT are decoded to the 25 typed properties below, anything else is dropped (except custom-id via preserved identifier,
and block-entity tag via BLOCK_ENTITY property). Properties (`ChunkerItemProperty`): amount i32, block_entity BlockEntity, book_author String, book_pages List<Json>,
book_title String, can_destroy / can_place_on List<ChunkerBlockIdentifier>, display ChunkerItemDisplay{displayName Json?, lore List<Json>?, color Color?}, durability i32,
enchantments Map<EnchantmentType,i32>, entity Entity, explorer_map ExplorerMap(16 enum), fireworks Fireworks{flightDuration i8, explosions List}, firework_explosion
Explosion{shape, colors List<Color>, fadeColors, trail, twinkle}, horn_instrument HornInstrument(8 enum), map_decoration {ExplorerMap structure, f64 x, f64 z, f32 rotation} (record),
map_index i32, ominous_bottle_amplifier i32, potion PotionType (~49 consts, string id each; Bedrock-only `LONG_MUNDANE`), repair_cost i32, spawn_egg_mob EntityType,
stew_effects StewEffect{Map<EffectType,i32>}, trim Trim{TrimMaterial(11 enum), TrimPattern(18 enum)}, bundle_contents List<ItemStack>, lodestone_data Lodestone{Dimension, x,y,z, tracked}.
Small enums: DyeColor (16, name + bedrockRGB + javaRGB; getID=ordinal, reversedID=15-ordinal; `findClosestDyeColor(Color)` = min squared RGB distance over BOTH palettes, first-min wins),
BannerPattern (43), EnchantmentType (43, internal names like ALL_DAMAGE_PROTECTION), EffectType (41 incl EMPTY), PotionBottleType {UNKNOWN,REGULAR,SPLASH,LINGERING; id=ordinal-1}, FireworkShape (5, id=ordinal).
Mutable after construction by resolvers via put/remove; equality is structural.

## 9. Passthrough / unknown handling (summary)
- Unknown block (no mapping) -> dropped to STONE+Preserved (only when user mapping exists) else reader-specific fallback; custom blocks via `ChunkerBlockIdentifier.custom` (custom states kept as boxed Object).
- Unknown biome -> `ChunkerCustomBiome(id)`; writer remap via fallback chain, then Dimension.fallbackBiome.
- Unknown entity/block entity -> `UnknownEntity/UnknownBlockEntity` + `originalNBT` when `--keepOriginalNBT`; otherwise only pos/motion/rot/type survive.
- Items: preservedIdentifier + the 25 properties; nothing else.

## 10. Lookup tables / cross-package dependencies for the port
1. `conversion/encoding/base/resolver/identifier/{ChunkerBlockIdentifierResolver, ChunkerItemIdentifierResolver, BlockMapping, ItemMapping, state/StateMapping}`: per-version tables of
   Identifier <-> (ChunkerBlockType + states). They enumerate `BlockState.getValues()` (cartesian) and need type->stateSet, so state-value enums need `ALL`, `ordinal`, and a stable order.
2. Mapping JSON files (`mapping/mappings/*`, `MappingsFile`) are NOT in this checkout (resources has only 2 files); user-supplied block/item/entity mapping files feed PreservedIdentifier.
3. `handlers/pretransform/Edge` (4 values, relative(ChunkCoordPair), opposite), `BlockPreTransformHandler.Direction` (referenced by FacingDirectionHorizontal.asDirection) - cyclic: break by moving Direction next to the state enum.
4. `resolver/property/{Property,PropertyHolder}`, `ComparableItemProperty`, `util InvertibleMap`, fastutil collections, Gson JsonElement (text components), java.awt.Color (use [u8;4] / u32 ARGB), NBT classes (CompoundTag, ByteTag/IntTag/StringTag) from the project's NBT module (area S1/other).
5. `converter.shouldAllowNBTCopying()` flag; `Dimension` list (3 statics + registry); per-version block-entity-id tables in encoding/{java,bedrock}/.../blockentity handlers (not in this area).

## 11. PROPOSED RUST DESIGN: `src/convert/model/`
Principles: enums+structs only; `dyn` appears in exactly one spot (pre-transform closure). Static tables generated once from the Java enums by a script (kept in repo), not hand-written.
Id types are `Copy` fieldless `#[repr(u16)]` enums; strings resolved through `const` tables.

```
src/convert/model/
  mod.rs               re-exports, `pub type Result`
  coords.rs            ChunkCoord{x,z}, RegionCoord, Edge{PosX,NegX,PosZ,NegZ}                                   ~110
  palette/mod.rs       Palette<T, const D: usize> enum + API                                                       ~330
  palette/packed.rs    PackedIndices{bits:u8, len, words:Box<[u64]>}, get/set/resize/iter/to_u16/from_u16        ~200
  column.rs            Column, Chunk (section), light, heightmap enums, pre_transform queue                       ~330
  biome/mod.rs         Biome enum {Vanilla(VanillaBiome),Custom(Arc<str>)} + find()/fallback()                    ~120
  biome/table.rs       GENERATED VanillaBiome enum + [(java,bedrock,fallback)] table via macro                    ~130
  biome/layout.rs      Biomes enum (4 layouts) + conversions                                                       ~420
  block/mod.rs         BlockType{Vanilla(VanillaBlock),Custom(Arc<CustomBlock>)}, BlockInfo accessors              ~140
  block/table.rs       GENERATED `blocks!{ Name, rgb, states=GROUP|[..], be=Kind, solid, flags }` 1,292 rows        ~1,350
  block/groups.rs      group flags (DOOR, STAIRS, ...) as u16 bitflags column in table + fn is_in(group)           ~60
  block/identifier.rs  BlockIdentifier{ty,states:StateSet,preserved}, copy_with/without, custom(), Eq/Hash        ~280
  block/state/mod.rs   StateId(u8 enum, 133), StateKey, StateVal, StateSet(SmallVec sorted), StateMask([u64;3])   ~260
  block/state/defs.rs  `states!{ FACING_HORIZONTAL = "facing", FacingHorizontal, default NORTH, ...}` x133        ~170
  block/state/types.rs `state_enum!{ Name {A,B,C} }` x70 + hand code for the 7 enums with methods                 ~520
  block/state/groups.rs state-set consts (36)                                                                      ~70
  item/mod.rs          ItemStack{id:ItemId, preserved, props:ItemProps}, ItemId{Block(BlockIdentifier),Item(VanillaItem)} ~180
  item/props.rs        ItemProps struct of Option<..> x25 (+Display, Lodestone, Trim ...)                         ~260
  item/table.rs        GENERATED VanillaItem enum (~470) with be-kind column                                      ~520
  item/enums.rs        DyeColor, BannerPattern, EnchantmentType, EffectType, PotionType, BottleType, FireworkShape,
                       HornInstrument, ExplorerMap, Trim{Material,Pattern}                                          ~520
  blockentity/mod.rs   BlockEntity{pos,movable,custom_name,original_nbt,data:BlockEntityData}, Kind, parent()     ~250
  blockentity/data.rs  one struct per non-empty kind (24) + `enum BlockEntityData` (52 variants)                  ~620
  blockentity/container.rs ContainerData{items:BTreeMap<u8,ItemStack>}, Randomizable                              ~60
  entity/mod.rs        Entity{pos,motion,yaw,pitch,original_nbt,data:EntityData}, EntityData{ItemFrame,Painting,Cushion,Unknown} ~220
  entity/types.rs      GENERATED EntityType enum (166) + Custom(Arc<str>), Motive(50)                              ~300
  identifier.rs        mapping::Identifier + StateValue{Bool,Int,Str} (serde, NBT conv)                            ~200
```
Estimated total: ~7,300 Rust lines, of which ~2,400 are generated tables (blocks 1,350, items 520, entity types/biomes ~430, misc) and ~1,000 are one-line macro invocations. Hand-written logic ~3,900.
Java 11.9k -> Rust ~7.3k (-38%).

### 11.1 Palette
```rust
pub enum Palette<T, const D: usize> {            // D=16 blocks/bio16, D=4 biome cells
    Empty,                                       // no keys: every read -> caller default
    Single(T),                                   // dense uniform
    Packed { keys: Vec<T>, idx: PackedIndices }  // idx.len == D*D*D, bits = max(1, ceil_log2(keys.len()))
}
pub struct PackedIndices { bits: u8, words: Box<[u64]> }  // entries do NOT span words: per = 64/bits; 4096 @4bit = 2 KiB (Java: 8 KiB)
impl<T: Clone + Eq + Hash, const D: usize> Palette<T, D> {
    pub const fn index(x: usize, y: usize, z: usize) -> usize { (y << S) | (z << (S/2... )) } // YZX: y*D*D + z*D + x (matches Java 1.13+ disk order -> cheap writer)
    pub fn get(&self, x,y,z) -> Option<&T>;  pub fn get_or<'a>(&'a self, ..., d: &'a T) -> &'a T;   // out-of-range key => None/default
    pub fn set(&mut self, x,y,z, v: T);      // promotes Empty/Single -> Packed, grows bits (repack) when keys > 1<<bits
    pub fn key_index_or_insert(&mut self, v: T) -> u16;   // linear for <=16 keys, side HashMap<T,u16> beyond (legacy worlds)
    pub fn set_index(&mut self, x,y,z, i: u16);
    pub fn map<U>(self, f: impl FnMut(T)->U) -> Palette<U, D>;   // consumes: no aliasing as in Java
    pub fn contains_key(&self, p: impl Fn(&T)->bool) -> bool;    // keys only (cheap)
    pub fn contains_value(&self, p: impl Fn(&T)->bool) -> bool;  // matched-index mask then scan words
    pub fn compact(self, default: &T) -> Self;                   // identical semantics to ShortBasedPalette.compact
    pub fn iter_indices(&self) -> impl Iterator<Item=u16>;  pub fn keys(&self) -> &[T];
    pub fn to_u16_vec(&self) -> Vec<u16>;  pub fn from_parts(keys: Vec<T>, idx: Vec<u16>) -> Self;  // reader/writer fast path
}
pub type BlockPalette = Palette<BlockRef, 16>;     // BlockRef = Arc<BlockIdentifier>
```
Key type: `Arc<BlockIdentifier>` (cheap clone, identity-short-circuit eq). Add an optional global interner (`DashMap<BlockIdentifier, Arc<..>>`) used by readers so equal blocks share
one allocation and `Arc::ptr_eq` fast-paths `==`. Whole-world cost target: a typical overworld section (<=16 keys) = 2 KiB packed + keys; an all-air section = one `Single`.

### 11.2 Column
```rust
pub struct Column { pub pos: ChunkCoord, pub chunks: BTreeMap<i8, Chunk>, pub biomes: Option<Biomes>, pub heightmap: Option<HeightMap>,
    pub entities: Vec<Entity>, pub block_entities: Vec<BlockEntity>, pub light_populated: bool, pub pre: PreTransform }
pub struct Chunk { pub y: i8, pub blocks: BlockPalette, pub block_light: Option<Box<[u8; 4096]>>, pub sky_light: Option<Box<[u8; 4096]>> } // light layout [x][y][z] -> YZX nibble-per-byte
pub enum HeightMap { JavaLegacy(Box<[[i16;16];16]>), Bedrock(Box<[[i16;16];16]>) }
pub struct PreTransform { edges: EdgeSet(u8 bitmask), handler: Option<Box<dyn FnOnce(&mut Column, &EdgeColumns) + Send>> } // the single dyn; compose on add
```
Methods: `block(x,y,z)->&BlockRef` (AIR static via `LazyLock`), `set_block`, `highest_block(x,z,pred)`, `block_entity_at(x,y,z)` (+ optional HashMap index built on demand since the Vec scan is O(n) per call in Java), `Chunk::is_empty()` (keys-only semantics, document it).

### 11.3 Block ids and states (cheap 133 state types / 70 value enums)
```rust
#[repr(u16)] #[derive(Copy,Clone,Eq,Hash,PartialEq)] pub enum VanillaBlock { Air, Stone, ... }          // generated, 1,292
pub struct BlockInfo { pub rgb: u32, pub solid: bool, pub states: &'static StateMask, pub be: Option<BlockEntityKind>, pub groups: u16 }
pub static BLOCKS: [BlockInfo; N];  impl VanillaBlock { pub fn info(self)->&'static BlockInfo }                   // table indexed by discriminant
pub enum BlockType { Vanilla(VanillaBlock), Custom(Arc<CustomBlock>) }  // CustomBlock{id:Arc<str>, states:StateMask-like Vec<StateKey>}, Eq by id

pub trait StateEnum: Copy + 'static { const ALL: &'static [Self]; fn ord(self)->u8; fn from_ord(u8)->Option<Self>; fn name(self)->&'static str; }
macro_rules! state_enum { ($n:ident { $($v:ident),+ }) => { #[repr(u8)] #[derive(Copy,Clone,Eq,PartialEq,Hash,Debug)] pub enum $n {$($v),+}  impl StateEnum for $n { ... } } }
//   state_enum!(Distance { _1,_2,_3,_4,_5,_6,_7 })  -- 1-3 lines each x70; the 7 enums with methods get `impl` blocks beside.
#[repr(u8)] pub enum StateId { Waterlogged, Age1, Age15, ..., WallWest }                                    // 133, generated from VanillaBlockStates
pub struct StateDef { pub id: StateId, pub name: &'static str, pub n_values: u8, pub default: u8 }               // static [StateDef;133]
pub struct State<T: StateEnum>(pub StateId, PhantomData<T>);                                                      // typed handle: pub const FACING_HORIZONTAL: State<FacingHorizontal>
pub enum StateKey { Vanilla(StateId), Custom(Arc<str>) }   pub enum StateVal { Vanilla(u8), Custom(StateValue) }  // Custom = mapping::StateValue (bool/int/string)
pub struct StateSet(SmallVec<[(StateKey, StateVal); 4]>);  // kept sorted by key => derived Eq/Hash == Java Map equality; no HashMap alloc per block
pub struct BlockIdentifier { pub ty: BlockType, states: StateSet, pub preserved: Option<Arc<PreservedIdentifier>> }
impl BlockIdentifier {
    pub fn get<T: StateEnum>(&self, s: State<T>) -> Option<T>;   // present value, else default iff ty.states.contains(s.0) (returnDefaultValue=true semantics)
    pub fn get_present<T: StateEnum>(&self, s: State<T>) -> Option<T>;
    pub fn with<T: StateEnum>(&self, s: State<T>, v: T) -> Self;  pub fn without<T>(&self, s: State<T>) -> Self;
    pub fn custom(id: &str, states: impl IntoIterator<Item=(String, StateValue)>) -> Self;  // "waterlogged" special case preserved
}
```
Why this is cheap: `state_enum!` + `states!` macro tables turn 70+133 Java files into ~1,000 declarative lines; no generics-over-Class, no `dyn BlockStateValue`; a vanilla state value is one `u8`,
a block identifier with <=4 states is allocation-free apart from the `Arc`; resolvers get `StateId`/`ALL` for the cartesian-product mapping builders.
Group sets (DOORS, STAIRS, ...) become a `groups: u16` bit column on `BlockInfo` (`ty.in_group(Group::Stairs)`) instead of 10 `HashSet`s; state-group unions become `const StateMask`s.

### 11.4 Block entities: enum (not trait objects)
```rust
#[repr(u8)] pub enum BlockEntityKind { Banner, Beacon, Bed, ..., Unknown }          // ~52, `parent() -> Option<Kind>` const fn: Dropper->Dispenser, TrappedChest->Chest, BlastFurnace|Smoker->Furnace,
                                                                                    // HangingSign->Sign, EndGateway->EndPortal, CalibratedSculkSensor->SculkSensor  (emulates class hierarchy dispatch)
pub struct BlockEntity { pub x: i32, pub y: i32, pub z: i32, pub movable: bool, pub custom_name: Option<serde_json::Value>, pub original_nbt: Option<Nbt>, pub data: BlockEntityData }
pub enum BlockEntityData {
    Banner(Banner), Beacon(Beacon), Bed, Beehive, Bell, Brushable(Brushable), Campfire(Campfire), Cauldron(Cauldron), ChiseledBookshelf(Box<[Option<ItemStack>;6]>),
    CommandBlock(CommandBlock), Comparator{output_signal:i32}, Conduit, CopperGolemStatue, CreakingHeart, DaylightDetector, DecoratedPot(Box<DecoratedPot>), EnchantmentTable,
    EndPortal, EndGateway(EndGateway), Jigsaw(Box<Jigsaw>), Jukebox{record:Option<ItemStack>}, Lectern{book:Option<ItemStack>, page:i32}, PotentSulfur, SculkCatalyst,
    SculkSensor, CalibratedSculkSensor, SculkShrieker, Shelf(Box<[Option<ItemStack>;3]>), Sign(Box<Sign>), HangingSign(Box<Sign>), Skull(Skull), Spawner(Box<Spawner>),
    Structure(Box<Structure>), Test, TestInstance, TrialSpawner, Vault,
    // containers share ContainerData; randomizable adds loot_table
    BlastFurnace(Furnace), Furnace(Furnace), Smoker(Furnace), BrewingStand(BrewingStand), EnderChest(Container), Hopper(Container),
    Barrel(Randomizable), Chest(Randomizable), TrappedChest(Randomizable), Dispenser(Randomizable), Dropper(Randomizable), ShulkerBox(Randomizable), Crafter(Crafter),
    Unknown(Arc<str>),
}
impl BlockEntityData { pub fn kind(&self)->BlockEntityKind; pub fn container(&self)->Option<&Container>; pub fn container_mut(&mut self)->Option<&mut Container>; }
```
Why enum: the Java list is closed and every handler is registered per class; in Rust a handler table is `[Option<fn(&mut Ctx, &Nbt)->BlockEntity>; N]` indexed by `BlockEntityKind`; `match` replaces
`getFromTypeHandler`/class walk, `parent()` replaces superclass lookup, `container()` replaces the `instanceof ContainerBlockEntity` interface handlers. Empty kinds are unit variants (macro generates
the 17 trivial ones: `empty_be!{Bed, Beehive, ...}`). Derive `Clone, PartialEq` (PartialEq must IGNORE `original_nbt` to match Java equals - implement manually on `BlockEntity`).
Box large payloads (Structure ~150 B, Jigsaw, Spawner, Sign) so `size_of::<BlockEntityData>()` stays ~64 B.

### 11.5 Entities, items
```rust
pub struct Entity { pub pos:[f64;3], pub motion:[f64;3], pub yaw:f32, pub pitch:f32, pub original_nbt: Option<Nbt>, pub data: EntityData }
pub enum EntityData { ItemFrame{ hanging: Hanging, glow: bool, item: Option<ItemStack>, direction: FacingDirection, item_rotation: i8, map: bool, photo: bool },
                      Painting{ hanging: Hanging, motive: Motive, direction: FacingDirectionHorizontal }, Cushion{ color: DyeColor }, Unknown(EntityType) }   // type() derived: glow => GLOW_ITEM_FRAME
pub enum EntityType { Vanilla(VanillaEntity), Custom(Arc<str>) }         // VanillaEntity generated (166) with fn has_spawn_egg()
pub struct ItemStack { pub id: ItemId, pub preserved: Option<Arc<PreservedIdentifier>>, pub props: ItemProps }
pub enum ItemId { Block(BlockIdentifier), Item(VanillaItem) }             // =ChunkerItemStackIdentifier
#[derive(Default, Clone, PartialEq)] pub struct ItemProps { pub amount: Option<i32>, pub block_entity: Option<Box<BlockEntity>>, pub book_author: Option<String>, pub book_pages: Option<Vec<Json>>, ... 25 fields ... pub bundle_contents: Option<Vec<ItemStack>> }
```
`ItemProps` as a struct of Options replaces the `Property<..>` registry (typed, no downcast, Eq derive, ~3x smaller than a hash map); if a name<->property lookup is needed by resolvers
generate `fn by_name(&str)` with a macro. `ItemStack` recursion (bundle, block_entity->container->items) needs `Box`/`Vec` indirection only.

### 11.6 mapping::Identifier
```rust
pub struct Identifier { pub name: Arc<str>, pub states: BTreeMap<Arc<str>, StateValue> }   // BTreeMap = deterministic order/hash
#[derive(Clone, PartialEq, Eq, Hash)] pub enum StateValue { Bool(bool), Int(i32), Str(Arc<str>) }
impl StateValue { fn from_json(&Value)->Result<Self>; fn to_json(&self)->Value; fn from_nbt(&Tag)->Result<Self> /* Byte==1 =>true else false; Int; String; else Err */; fn to_nbt(&self)->Tag }
impl Identifier { fn from_data(id, Option<i32>) /* states{"data":Int} */; fn data_value(&self)->Option<i32>; fn is_vanilla(&self)->bool }
```
Serde: StateValue untagged (bool|string|number->i32 truncating like intValue); lists: one element serialises as scalar - implement `deserialize_with` helper `one_or_many`.

## 12. Class inventory (lines) grouped by sub-package. `[B]` = pure boilerplate a macro/generator replaces
**column/** ChunkerColumn 290; heightmap/{HeightMap 7, JavaLegacyHeightMap 26 [B], BedrockHeightMap 26 [B]}; chunk/{ChunkerChunk 109, ChunkCoordPair 77, RegionCoordPair 33}.
**chunk/palette/** Palette 142 (iface), WriteablePalette 44 (iface), ShortBasedPalette 228, SingleValuePalette 74, EmptyPalette 96 -> one Rust enum (~530 incl. packing).
**biome/** ChunkerBiome 234 (enum table [B] + find), ChunkerCustomBiome 50, layout/{ChunkerBiomes 50 iface, ClusterBased 132, ClusterPaletteBased 143, ColumnBased 107, PaletteBased 125}.
**chunk/identifier/** ChunkerBlockIdentifier 295; ChunkerItemStackIdentifier 25, ChunkerItemStackIdentifierType 17, PreservedIdentifier 13; type/block/{ChunkerBlockType 32, ChunkerCustomBlockType 129,
 ChunkerVanillaBlockType 1390 [B table], ChunkerVanillaBlockGroups 350 [B]}; type/block/states/{BlockState 110, BlockStateValue 7}; states/vanilla/{VanillaBlockStates 160 [B], VanillaBlockStateGroups 48 [B]};
 states/vanilla/types/* 70 files, 1,150 lines total [B] (FacingDirection 78, FacingDirectionHorizontal 122, FacingDirectionHorizontalDown 14 have logic; Age_25 35, Note 34, NoteBlockInstrument 36 big lists);
 type/item/{ChunkerItemType 15, ChunkerVanillaItemType 501 [B]}.
**chunk/itemstack/** ChunkerItemStack 103, ChunkerItemProperty 245 [B], ChunkerItemDisplay 19, ChunkerLodestoneData 94 (record boilerplate), ChunkerDyeColor 182, banner/ChunkerBannerPattern 49 [B],
 enchantment/ChunkerEnchantmentType 50 [B], firework/{Explosion 146, Shape 77, Fireworks 78} (getter/setter boilerplate), horn/HornInstrument 15 [B], map/{ExplorerMap 23 [B], MapDecoration 12},
 potion/{EffectType 48 [B], PotionBottleType 34, PotionType 76 [B]}, stew/StewEffect 50, trim/{Trim 60, TrimMaterial 18 [B], TrimPattern 25 [B]}.
**blockentity/** BlockEntity 142 (base), UnknownBlockEntity 26, ContainerBlockEntity 36, RandomizableContainerBlockEntity 46; 17 empty-marker classes of 7 lines [B] (Bed, Beehive, Bell, Conduit, CopperGolemStatue,
 CreakingHeart, DaylightDetector, EnchantmentTable, PotentSulfur, SculkCatalyst, SculkShrieker, Test, TestInstance, TrialSpawner, Vault + containers Hopper/EnderChest/Barrel/Dispenser/Dropper/ShulkerBox/TrappedChest/
 BlastFurnace/Smoker, EndPortal 9, SculkSensor 9, CalibratedSculkSensor 7); with fields: Banner 66, Beacon 62, Brushable 105, Campfire 45, Cauldron 86, ChiseledBookshelf 35, CommandBlock 79, Comparator 26,
 BrewingStand 84, Furnace 79, Chest 19, Crafter 60, DecoratedPot 123, EndGateway 98, Jigsaw 148, Jukebox 46, Lectern 66, Shelf 35, Sign 161, Skull 108, Spawner 179, Structure 504 (getter/setter [B]).
 Nearly all of these 2,664 lines are getters/setters/equals/hashCode that disappear in Rust (pub fields + derive).
**entity/** Entity 224, HangingEntity 96, ItemFrameEntity 133, GlowItemFrameEntity 14, PaintingEntity 177 (Motive table [B]), CushionEntity 65, UnknownEntity 26; type/{ChunkerEntityType 7, ChunkerCustomEntityType 33, ChunkerVanillaEntityType 200 [B]}.
**mapping/identifier/** Identifier 150, states/{StateValue 136 (incl. Gson adapter), StateValueBoolean 60, StateValueInt 58, StateValueString 59} -> one Rust enum.

## 13. Porting order / risks
1. Generate tables first (block, item, entity, biome, state enums, state defs) with a script that parses the Java enums (regex is enough; constants are one per line) - keeps ordinal order, which downstream mappers depend on.
2. Palette + Column + coords (self-contained, unit-testable against the Java tests in `cli/src/test/java/.../conversion` if present).
3. BlockIdentifier/StateSet; check equality semantics: Java hashes states map-wise (order independent); `preserved` participates in equals/hash, so two otherwise equal blocks with different preserved ids are DISTINCT palette keys.
4. BlockEntity/Entity/ItemStack structs; fill handlers later (other areas). Keep `original_nbt` out of PartialEq.
5. Risks: (i) keys-only `contains_key` vs voxel `contains_value` semantic difference (isEmpty/getHighestBlock use keys; unused keys survive until compact()); (ii) `PreTransform` closure borrows
   neighbour columns by Edge - design borrow shape together with the pre-transform area; (iii) `StateSet` custom keys compare by name+value; custom block type equality ignores states (Java ChunkerCustomBlockType.equals = id only) while
   BlockIdentifier equality includes states; (iv) light arrays are legacy-only: keep `Option`; (v) Dimension/fallback biome and `biomeHeight` live outside this area but biome layout conversions need them as parameters.
