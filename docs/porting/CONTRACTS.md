# Wave 2 contracts (exact public names the parallel agents code against)

Everything below lives under `src/convert/`. Crates/types referenced already exist on branch `converter`:
`nbt::{Tag, Compound, io}`, `util::Version`, `model::*` (BlockType, StateId, Column, Chunk, BlockEntity, BlockEntityData,
BlockEntityKind, ItemStack, ItemProps, ItemType, Palette, Biome(s), Dimension, Identifier ...), `mapping::{BlockResolver,
ItemResolver, IdTable, PivotBlock, PivotItem, Edition, Version (mapping::Version)}`, `bedrock::{db, keys, palette, data_version::BedrockVersion,
detect::{detect_bedrock, BedrockWorldInfo}, level_dat}`, `java::{format::JavaFormat, palette, region, dirs, version, level_dat}`,
`level::{Level, World, Dimension(level), LevelSettings, ConversionSettings, ConvertError ...}`.

## Item stacks
```rust
// bedrock/resolvers/itemstack/mod.rs   (owner: items-bedrock)
pub struct BedrockItems { .. }
impl BedrockItems {
    pub fn new(version: &'static BedrockVersion) -> BedrockItems;
    /// One item compound (Name, Count, Damage, tag{...}, Slot kept by the caller) -> model ItemStack.
    pub fn read(&self, tag: &Compound) -> Option<ItemStack>;
    /// Later phase (Java -> Bedrock); may return None until then.
    pub fn write(&self, stack: &ItemStack) -> Option<Compound>;
}
// java/resolvers/itemstack/mod.rs   (owner: items-java)
pub struct JavaItems { .. }
impl JavaItems {
    pub fn new(format: &JavaFormat, version: util::Version) -> JavaItems;
    /// model ItemStack -> item compound `{id, count/Count, components or tag}` for that version. None = cannot be represented (log + drop).
    pub fn write(&self, stack: &ItemStack) -> Option<Compound>;
    /// Later phase (Java -> Bedrock).
    pub fn read(&self, tag: &Compound) -> Option<ItemStack>;
}
```
## Block entities
```rust
// bedrock/resolvers/block_entity/mod.rs   (owner: items-bedrock)
/// A Bedrock block entity compound (has id, x, y, z) -> model BlockEntity (all 51 kinds Chunker handles; unknown id -> BlockEntityData::Unknown keeping original NBT when asked).
pub fn read_block_entity(items: &BedrockItems, tag: &Compound, keep_unknown: bool) -> Option<BlockEntity>;
// java/resolvers/block_entity/mod.rs   (owner: be-java)
/// model BlockEntity -> Java block entity compound (id, x, y, z, keepPacked, components/items ...) for that format.
pub fn write_block_entity(items: &JavaItems, format: &JavaFormat, be: &BlockEntity) -> Option<Compound>;
```
## Bedrock reader side
```rust
// bedrock/pretransform/mod.rs   (owner: pretransform)
/// The pre-transform for a column (fix fence/pane/wall/stairs/door/gate/redstone/fire/chorus/vine/chest/snowy/stem/note-block states
/// that Bedrock does not store but Java needs), to be handed to Column::set_pretransform or run by the pipeline.
pub fn pretransform(version: &'static BedrockVersion) -> model::PreTransform;     // use the exact closure type of model::Column
// bedrock/reader.rs, bedrock/level_reader.rs   (owner: reader)
pub struct BedrockWorldReader { .. }
impl BedrockWorldReader {
    pub fn open(info: &BedrockWorldInfo, settings: &ConversionSettings) -> Result<BedrockWorldReader, ConvertError>;
    pub fn level(&self) -> Result<Level, ConvertError>;                         // level.dat -> level model (+ dimensions present in the db)
    pub fn dimensions(&self) -> Vec<level::Dimension>;
    pub fn columns(&self, dim: &level::Dimension) -> Vec<(i32, i32)>;           // all column positions in a dimension
    pub fn read_column(&self, dim: &level::Dimension, x: i32, z: i32) -> Result<Option<Column>, ConvertError>;
}
```
## Java writer side
```rust
// java/writer.rs, java/level_writer.rs   (owner: writer)
pub struct JavaWorldWriter { .. }
impl JavaWorldWriter {
    pub fn create(out: &Path, target: util::Version, level: &Level, settings: &ConversionSettings) -> Result<JavaWorldWriter, ConvertError>;
    pub fn write_column(&mut self, dim: &level::Dimension, column: &Column) -> Result<(), ConvertError>;   // buffers per region, flushes regions when complete or when memory bound is reached
    pub fn finish_dimension(&mut self, dim: &level::Dimension) -> Result<(), ConvertError>;
    pub fn finish(self) -> Result<(), ConvertError>;                                                       // level.dat, session files, icon etc.
}
```
Until a contract owner's code is merged, code against a local stub with the same signature in a file named `stub_*.rs` inside your own folder and delete it when you merge the real thing (periodically `git log conv-<owner> --oneline | head -3`; merge it with `git merge conv-<owner>`).
Branch names: pretransform=conv-pretransform, items-bedrock=conv-items-bedrock, items-java=conv-items-java, be-java=conv-be-java, reader=conv-reader, writer=conv-writer.
