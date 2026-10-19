// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Mob spawners (`JavaSpawnerBlockEntityHandler`).

use super::ctx::Ctx;
use super::tables;
use crate::convert::model::blockentity::Spawner;
use crate::convert::nbt::Compound;

pub(super) fn write(ctx: &Ctx, out: &mut Compound, spawner: &Spawner) {
    out.put("Delay", spawner.delay);
    out.put("SpawnRange", spawner.spawn_range);
    out.put("MaxSpawnDelay", spawner.max_spawn_delay);
    out.put("MinSpawnDelay", spawner.min_spawn_delay);
    out.put("SpawnCount", spawner.spawn_count);
    out.put("RequiredPlayerRange", spawner.required_player_range);
    out.put("MaxNearbyEntities", spawner.max_nearby_entities);
    let Some(id) = spawner.entity_type.as_ref().and_then(|t| tables::entity_type_id(t, ctx.version)) else { return };
    // 1.18 nested the entity in `SpawnData.entity`.
    let spawn_data = if ctx.since(1, 18, 0) {
        Compound::new().with("entity", Compound::new().with("id", id))
    } else {
        Compound::new().with("id", id)
    };
    out.put("SpawnData", spawn_data);
}
