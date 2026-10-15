//! The kinds of things a player adds to an instance, and where each one lives inside it.

/// Mods, resource packs and shader packs: the three tabs of the content manager.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum ContentKind {
    Mods,
    ResourcePacks,
    Shaders,
}

impl ContentKind {
    /// Every kind, in the order the content manager shows its tabs.
    pub const ALL: [ContentKind; 3] = [ContentKind::Mods, ContentKind::ResourcePacks, ContentKind::Shaders];

    /// The project type used when searching Modrinth.
    pub fn project_type(self) -> &'static str {
        match self {
            ContentKind::Mods => "mod",
            ContentKind::ResourcePacks => "resourcepack",
            ContentKind::Shaders => "shader",
        }
    }

    /// How the launcher records a file of this kind (`type` in `launcher.mods`).
    pub fn record_type(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::ResourcePacks => "resourcepack",
            ContentKind::Shaders => "shaderpack",
        }
    }

    /// The folder inside the instance.
    pub fn folder(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::ResourcePacks => "resourcepacks",
            ContentKind::Shaders => "shaderpacks",
        }
    }

    /// The tab title.
    pub fn title(self) -> &'static str {
        match self {
            ContentKind::Mods => "Mods",
            ContentKind::ResourcePacks => "Resource packs",
            ContentKind::Shaders => "Shaders",
        }
    }

    /// The kind that a launcher file type (`mods`, `resourcepack`, `shaderpack`) belongs to; `None` for anything
    /// else, such as data packs.
    pub fn from_record_type(record_type: &str) -> Option<ContentKind> {
        ContentKind::ALL.into_iter().find(|k| k.record_type() == record_type)
    }

    /// The kind for a Modrinth project type; `None` for modpacks, plugins and data packs.
    pub fn from_project_type(project_type: &str) -> Option<ContentKind> {
        ContentKind::ALL.into_iter().find(|k| k.project_type() == project_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_folder_and_types() {
        assert_eq!(ContentKind::Mods.folder(), "mods");
        assert_eq!(ContentKind::ResourcePacks.folder(), "resourcepacks");
        assert_eq!(ContentKind::Shaders.folder(), "shaderpacks");
        assert_eq!(ContentKind::from_record_type("shaderpack"), Some(ContentKind::Shaders));
        assert_eq!(ContentKind::from_project_type("resourcepack"), Some(ContentKind::ResourcePacks));
        assert_eq!(ContentKind::from_record_type("datapack"), None);
        assert_eq!(ContentKind::from_project_type("modpack"), None);
    }
}
