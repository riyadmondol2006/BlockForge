# gen_model.py

Generates `src/convert/model/generated.rs` from a clone of Chunker (`cli/src/main/java` only):

    python3 -I tools/convert/gen_model.py --chunker ../Chunker      # write
    python3 -I tools/convert/gen_model.py --chunker ../Chunker --check   # exit 1 when out of date

`--chunker` defaults to `$CHUNKER_DIR`, then `../Chunker`. Stdlib only. The header of the generated file records the
Chunker commit (`git rev-parse HEAD` of the clone). The output is deterministic.

Parsed sources (under `conversion/intermediate/column`): `VanillaBlockStates`, the value enums in `states/vanilla/types`,
`VanillaBlockStateGroups`, `ChunkerVanillaBlockType`, `ChunkerVanillaBlockGroups`, `ChunkerVanillaItemType`,
`ChunkerVanillaEntityType`, `PaintingEntity.Motive`, `ChunkerBiome.ChunkerVanillaBiome` and the item stack enums
(dye colour, banner pattern, enchantment, effect, potion, bottle, firework shape, horn, explorer map, trim).

Enum variants keep the Java constant names and ordinals. When a Java enum gains a constant, rerun the script; a new
block entity class needs a variant in `BlockEntityKind` (`src/convert/model/blockentity.rs`) and a `BlockEntityData`
arm. The script prints the block entity kinds the tables refer to.
