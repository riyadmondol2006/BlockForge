//! Saved plans, the player's stock, and writing a plan out as text or JSON.

use std::collections::{BTreeMap, BTreeSet};

use super::types::*;
use crate::paths::Dirs;

/// One saved plan: the request, and which gather/craft items were ticked off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPlan {
    pub id: u64,
    pub name: String,
    pub version: String,
    pub request: Request,
    /// `gather:<item>` and `step:<recipe id>` keys that were ticked.
    pub done: BTreeSet<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    pub plans: Vec<SavedPlan>,
    /// Per game version, the items the player says they own.
    pub stock: BTreeMap<String, BTreeMap<ItemId, u64>>,
}

pub fn load(_dirs: &Dirs) -> Store {
    Store::default()
}

pub fn save(_dirs: &Dirs, _store: &Store) -> Result<(), String> {
    Ok(())
}

/// The shopping list as plain text, to paste into a chat or a note.
pub fn to_text(_ds: &Dataset, _plan: &Plan) -> String {
    String::new()
}

/// The whole plan as JSON, to keep or share.
pub fn to_json(_ds: &Dataset, _plan: &Plan) -> String {
    String::new()
}

/// A plan file written by [`export_plan`].
pub fn export_plan(_plan: &SavedPlan) -> String {
    String::new()
}

pub fn import_plan(_text: &str) -> Result<SavedPlan, String> {
    Err("not written yet".into())
}
