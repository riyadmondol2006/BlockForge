//! The fields every entity has: id, position, UUID and name, plus the id links (owner, UUIDs) shared by the types.

use md5::{Digest, Md5};
use uuid::Uuid;

use super::EntityContext;
use crate::convert::bedrock::resolvers::entity::{HOST_UNIQUE_ID, actor_link};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

/// The Java UUID of a Bedrock `UniqueID`: a name based (version 3) UUID of `bedrock:<id>`, as Java's
/// `UUID.nameUUIDFromBytes` makes it, so the same actor always gets the same UUID.
pub fn uuid_of_unique_id(unique_id: i64) -> Uuid {
    let digest = Md5::digest(format!("bedrock:{unique_id}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// A UUID as the four big-endian ints Java stores (1.16+).
pub fn uuid_ints(uuid: Uuid) -> Vec<i32> {
    let b = uuid.as_bytes();
    (0..4).map(|i| i32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])).collect()
}

/// Store a UUID under `key` as the version does: an int array from 1.16, `<key>Most` / `<key>Least` longs before.
pub fn put_uuid(ctx: &EntityContext<'_>, out: &mut Compound, key: &str, uuid: Uuid) {
    if ctx.since(1, 16, 0) {
        out.put(key, uuid_ints(uuid));
    } else {
        let (most, least) = uuid.as_u64_pair();
        out.put(format!("{key}Most"), most as i64);
        out.put(format!("{key}Least"), least as i64);
    }
}

/// The Java owner of a Bedrock owner id: the account of the settings for the host (or the host's own derived UUID),
/// the derived UUID of any other player.
pub fn owner_uuid(ctx: &EntityContext<'_>, owner: i64) -> Uuid {
    if owner == HOST_UNIQUE_ID {
        ctx.pet_owner.unwrap_or_else(|| uuid_of_unique_id(owner))
    } else {
        uuid_of_unique_id(owner)
    }
}

/// `Owner` (a UUID from 1.16, the `OwnerUUID` string before) from the Bedrock `OwnerNew`; whether there was one.
pub fn put_owner(ctx: &EntityContext<'_>, out: &mut Compound, source: &Compound) -> bool {
    let Some(owner) = actor_link(source, "OwnerNew") else { return false };
    let uuid = owner_uuid(ctx, owner);
    if ctx.since(1, 16, 0) {
        out.put("Owner", uuid_ints(uuid));
    } else {
        out.put("OwnerUUID", uuid.hyphenated().to_string());
    }
    true
}

/// `Pos`, `Motion` and `Rotation`.
pub fn put_position(out: &mut Compound, pos: [f64; 3], motion: [f64; 3], yaw: f32, pitch: f32) {
    out.put("Pos", Tag::List(pos.iter().map(|v| Tag::Double(*v)).collect()));
    out.put("Motion", Tag::List(motion.iter().map(|v| Tag::Double(*v)).collect()));
    out.put("Rotation", Tag::List(vec![Tag::Float(yaw), Tag::Float(pitch)]));
}

/// `UUID` from `UniqueID`, `CustomName` (Bedrock keeps plain text with `§` codes) and `CustomNameVisible`.
pub fn put_identity(ctx: &EntityContext<'_>, out: &mut Compound, source: &Compound) {
    if let Some(uid) = source.get_i64("UniqueID") {
        put_uuid(ctx, out, "UUID", uuid_of_unique_id(uid));
    }
    if let Some(name) = source.get_str("CustomName").filter(|n| !n.is_empty())
        && let Ok(text) = json_text::to_nbt_for(&json_text::from_text(name), ctx.version())
    {
        out.put("CustomName", text);
        if source.get_i8_or("CustomNameVisible", 0) != 0 {
            out.put("CustomNameVisible", true);
        }
    }
}

/// The x, y and z step of a 3D facing id.
pub fn step_3d(facing: i8) -> (i32, i32, i32) {
    match facing {
        0 => (0, -1, 0),
        1 => (0, 1, 0),
        2 => (0, 0, -1),
        3 => (0, 0, 1),
        4 => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

/// An `{X, Y, Z}` compound (`HivePos` before 1.20.5) or an int array (`hive_pos`, `home_pos`, `leash`).
pub fn block_pos_compound(x: i32, y: i32, z: i32) -> Compound {
    Compound::new().with("X", x).with("Y", y).with("Z", z)
}

/// A Bedrock float list position (`HomePos`) as a block position.
pub fn block_of(source: &Compound, key: &str) -> Option<[i32; 3]> {
    let list = source.get_list(key)?;
    let n = |i: usize| list.get(i).and_then(Tag::as_number).map(|v| v.floor() as i32);
    Some([n(0)?, n(1)?, n(2)?])
}

/// Whether an actor carries a component group (`+minecraft:cat_calico`).
pub fn group(source: &Compound, name: &str) -> bool {
    crate::convert::bedrock::resolvers::entity::has_definition(source, name)
}

/// The value of a Bedrock attribute (`Attributes[{Name, Current, Max, Base...}]`).
pub fn attribute(source: &Compound, name: &str, field: &str) -> Option<f64> {
    source
        .get_list("Attributes")?
        .iter()
        .filter_map(Tag::as_compound)
        .find(|a| a.get_str("Name") == Some(name))
        .and_then(|a| a.get(field).and_then(Tag::as_number))
}
