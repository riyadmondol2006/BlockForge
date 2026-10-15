//! The list of mods an instance remembers in `instance.json` (`launcher.mods`), in the Java edition's format.
//! Every way of getting a mod into an instance (Modrinth, CurseForge, a pack, a file the player added) records it
//! here, and every part that reads mods reads it from here, so both editions see the same list.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::instances::Instance;

/// One mod, resource pack, shader pack, ... that belongs to an instance.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModRecord {
    pub name: String,
    pub version: String,
    pub optional: bool,
    /// File name inside its folder, e.g. `sodium-fabric-0.5.jar`.
    pub file: String,
    /// Folder inside the instance when it is not the default for `kind` (`path` in the Java edition).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `mods`, `resourcepack`, `shaderpack`, `dependency`, `plugin`, `jar`, `forge`, ... (`type` in the Java edition).
    #[serde(rename = "type")]
    pub kind: String,
    pub description: String,
    pub disabled: bool,
    pub user_added: bool,
    pub was_selected: bool,
    pub skipped: bool,
    #[serde(alias = "curseModId", skip_serializing_if = "Option::is_none")]
    pub curse_forge_project_id: Option<i64>,
    #[serde(alias = "curseFileId", skip_serializing_if = "Option::is_none")]
    pub curse_forge_file_id: Option<i64>,
    /// Whole CurseForge / Modrinth objects as the Java edition saves them; kept as they are.
    #[serde(alias = "curseMod", skip_serializing_if = "Option::is_none")]
    pub curse_forge_project: Option<Value>,
    #[serde(alias = "curseFile", skip_serializing_if = "Option::is_none")]
    pub curse_forge_file: Option<Value>,
    #[serde(alias = "modrinthMod", skip_serializing_if = "Option::is_none")]
    pub modrinth_project: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth_version: Option<Value>,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

impl ModRecord {
    pub fn new(name: &str, file: &str, kind: &str) -> ModRecord {
        ModRecord {
            name: name.to_string(),
            file: file.to_string(),
            kind: kind.to_string(),
            was_selected: true,
            optional: true,
            ..Default::default()
        }
    }

    /// Modrinth project id, when this mod came from Modrinth.
    pub fn modrinth_project_id(&self) -> Option<&str> {
        self.modrinth_project.as_ref().and_then(|p| p.get("id")).and_then(Value::as_str)
    }

    /// Modrinth version id, when known.
    pub fn modrinth_version_id(&self) -> Option<&str> {
        self.modrinth_version.as_ref().and_then(|p| p.get("id")).and_then(Value::as_str)
    }

    /// The folder (relative to the instance) this record's file lives in while enabled.
    pub fn folder(&self) -> String {
        if let Some(p) = self.path.as_deref().filter(|p| !p.is_empty()) {
            return p.trim_matches('/').to_string();
        }
        match self.kind.as_str() {
            "resourcepack" | "texturepack" => "resourcepacks".into(),
            "shaderpack" => "shaderpacks".into(),
            "plugin" => "plugins".into(),
            "jar" | "forge" | "mcpc" | "millenaire" | "dependency" | "extract" | "decomp" | "coremods" => {
                // old-style kinds the Java edition places in other folders; the caller handles those specially
                "mods".into()
            }
            _ => "mods".into(),
        }
    }
}

impl Instance {
    /// The mods recorded for this instance (empty when none).
    pub fn mod_records(&self) -> Vec<ModRecord> {
        self.file.launcher.extras.get("mods").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
    }

    pub fn set_mod_records(&mut self, mods: &[ModRecord]) {
        self.file.launcher.extras.insert("mods".into(), serde_json::to_value(mods).unwrap_or(Value::Array(vec![])));
    }

    /// Add `record`, replacing an earlier record for the same file (or the same Modrinth/CurseForge project).
    pub fn upsert_mod_record(&mut self, record: ModRecord) {
        let mut mods = self.mod_records();
        mods.retain(|m| {
            m.file != record.file
                && !(record.modrinth_project_id().is_some() && m.modrinth_project_id() == record.modrinth_project_id())
                && !(record.curse_forge_project_id.is_some()
                    && m.curse_forge_project_id == record.curse_forge_project_id)
        });
        mods.push(record);
        self.set_mod_records(&mods);
    }

    pub fn remove_mod_record_file(&mut self, file: &str) {
        let mut mods = self.mod_records();
        mods.retain(|m| m.file != file);
        self.set_mod_records(&mods);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_records_round_trip_with_nested_objects() {
        let json = r#"{"name":"Gamma Utils","version":"3.2.0","optional":true,"file":"g.jar","type":"mods",
            "description":"d","disabled":false,"userAdded":true,"wasSelected":true,"skipped":false,
            "modrinthProject":{"id":"wdLuzzEP","slug":"gamma-utils"},"modrinthVersion":{"id":"abc"},"somethingNew":5}"#;
        let r: ModRecord = serde_json::from_str(json).unwrap();
        assert_eq!(r.modrinth_project_id(), Some("wdLuzzEP"));
        assert_eq!(r.modrinth_version_id(), Some("abc"));
        let back = serde_json::to_value(&r).unwrap();
        assert_eq!(back["somethingNew"], 5);
        assert_eq!(back["type"], "mods");
        assert_eq!(back["modrinthProject"]["slug"], "gamma-utils");
    }

    #[test]
    fn old_curse_field_names_are_read() {
        let r: ModRecord =
            serde_json::from_str(r#"{"name":"x","file":"x.jar","type":"mods","curseModId":5,"curseFileId":9}"#)
                .unwrap();
        assert_eq!((r.curse_forge_project_id, r.curse_forge_file_id), (Some(5), Some(9)));
    }
}
