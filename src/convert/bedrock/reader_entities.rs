//! The entities of a dimension: every actor of the two stores Bedrock has used, read at once.
//!
//! - Before 1.18.30 a column kept its actors in the chunk `Entity` record (tag 50): little-endian NBT compounds back
//!   to back.
//! - Since then a `digp` record per column (`digp` + x + z [+ dimension]) lists 8 byte actor keys, and each actor is
//!   one compound under `actorprefix` + key. Actor records no `digp` lists are stale and are not read.
//!
//! A world converted by the game holds some actors in both; the same `UniqueID` is read once (the `digp` one wins).
//! The dimension comes from the key, Bedrock actors don't name it. Each entity goes to the column its `Pos` lies in,
//! because mobs wander out of the column they are stored with; when that column has no chunk data it stays with the
//! column it was stored with.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use super::{BedrockWorldReader, convert_error};
use crate::convert::bedrock::keys::{self, ChunkTag, DimensionId};
use crate::convert::bedrock::palette::decode_nbt_concat;
use crate::convert::bedrock::resolvers::entity::{chunk_of, entity_type, read_actor, unique_id, vec3};
use crate::convert::level::{ConvertError, Dimension, EntityFilter};
use crate::convert::model::{Entity, VanillaEntityType};
use crate::convert::nbt::{Compound, Tag};

/// The entities of a dimension by the column they are written to.
pub type ColumnEntities = HashMap<(i32, i32), Vec<Entity>>;

/// One actor as read, with the column it was stored with.
struct Stored {
    tag: Compound,
    column: (i32, i32),
}

fn compounds(bytes: &[u8]) -> Result<Vec<Compound>, String> {
    let tags = decode_nbt_concat(bytes).map_err(|e| e.to_string())?;
    Ok(tags.into_iter().filter_map(|(_, t)| if let Tag::Compound(c) = t { Some(c) } else { None }).collect())
}

impl BedrockWorldReader {
    /// Every entity of `dim` that the settings keep, by column (see the module documentation). Empty when the
    /// settings process no entities. Damaged records are skipped with a warning; only a database failure is an error.
    pub fn entities(&self, dim: &Dimension) -> Result<ColumnEntities, ConvertError> {
        let s = &self.settings;
        let mut out = ColumnEntities::new();
        if !s.process_entities || s.entity_filter == EntityFilter::None {
            return Ok(out);
        }
        let id = DimensionId(dim.bedrock_id());
        let Some(entry) = self.dimensions.iter().find(|d| d.id == id) else { return Ok(out) };
        let known: HashSet<(i32, i32)> = entry.columns.iter().copied().collect();

        // The digp store first, so its copy of an actor wins.
        let mut stored: Vec<Stored> = Vec::new();
        for (key, value) in self.db.prefix(keys::DIGP_PREFIX).map_err(convert_error)? {
            let Some((key_dim, x, z)) = keys::parse_digp_key(&key) else { continue };
            if key_dim != id {
                continue;
            }
            for storage in keys::parse_digp_value(&value) {
                let Some(bytes) = self.db.get(&keys::actor_key(storage)).map_err(convert_error)? else { continue };
                match compounds(&bytes) {
                    Ok(list) => stored.extend(list.into_iter().map(|tag| Stored { tag, column: (x, z) })),
                    Err(e) => self.warn(format!("actor in column {x},{z} of {}: {e}", dim.identifier())),
                }
            }
        }
        for &(x, z) in &entry.columns {
            let key = keys::chunk_key(id, x, z, ChunkTag::Entity);
            let Some(bytes) = self.db.get(&key).map_err(convert_error)? else { continue };
            match compounds(&bytes) {
                Ok(list) => stored.extend(list.into_iter().map(|tag| Stored { tag, column: (x, z) })),
                Err(e) => self.warn(format!("entities of column {x},{z} of {}: {e}", dim.identifier())),
            }
        }

        let mut seen = HashSet::new();
        stored.retain(|a| unique_id(&a.tag).is_none_or(|uid| seen.insert(uid)));
        self.entities_read.fetch_add(stored.len() as u64, Ordering::Relaxed);

        // Leash knots, by id: the mobs tied to one keep its block.
        let mut knots = HashMap::new();
        for a in &stored {
            if entity_type(&self.items, &a.tag) == Some(VanillaEntityType::LEASH_KNOT)
                && let (Some(uid), Some(pos)) = (unique_id(&a.tag), vec3(&a.tag, "Pos"))
            {
                knots.insert(uid, pos);
            }
        }

        for a in stored {
            if entity_type(&self.items, &a.tag).is_none() {
                let name = a.tag.get_str("identifier").unwrap_or("(no identifier)").to_string();
                if let Ok(mut unmapped) = self.unmapped_entities.lock() {
                    *unmapped.entry(name).or_insert(0) += 1;
                }
                continue;
            }
            let Some(entity) = read_actor(&self.items, s, &a.tag, &knots) else { continue };
            let at = chunk_of(entity.position);
            let column = if known.contains(&at) { at } else { a.column };
            out.entry(column).or_default().push(entity);
        }
        Ok(out)
    }
}
