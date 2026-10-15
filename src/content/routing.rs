//! Works out which folder of an instance a file belongs in: mods, resource packs or shader packs. The file is
//! looked at by [`super::dropped`]; this only maps what it found to a folder.

use std::path::Path;

use super::dropped::{self, DroppedFile};
use super::kind::ContentKind;

/// At most this many names are read from a zip; enough to recognise any pack.
pub const MAX_ENTRIES: usize = dropped::MAX_ENTRIES;

/// Where a file goes.
///
/// * `file_name`: the file's name
/// * `zip_entries`: the names inside the file if it is a zip that could be read, otherwise `None`
/// * `current`: the tab the player dropped it on, used when a zip can't be told apart
///
/// Returns the kind, or `None` when the file doesn't belong in any of them (a data pack, a modpack, a text file).
pub fn route(
    file_name: Option<&str>,
    zip_entries: Option<&[String]>,
    current: Option<ContentKind>,
) -> Option<ContentKind> {
    let name = file_name?.to_lowercase();
    if name.ends_with(".jar") || name.ends_with(".litemod") {
        return Some(ContentKind::Mods);
    }
    if !name.ends_with(".zip") {
        return None;
    }
    if let Some(entries) = zip_entries {
        let found = dropped::classify(None, file_name?, entries, None);
        if found.kind != dropped::DroppedKind::Unknown {
            return found.kind.content_kind();
        }
    }
    // an unrecognised zip only makes sense in a pack folder, and only if that's where it was dropped
    if current == Some(ContentKind::Mods) { None } else { current }
}

/// The folder for what the detector found, or `None` if it doesn't go into one of these folders.
pub fn route_dropped(file: &DroppedFile) -> Option<ContentKind> {
    file.kind.content_kind()
}

/// The names inside a zip file, or `None` if it isn't a zip that can be read.
pub fn zip_entries(path: &Path) -> Option<Vec<String>> {
    dropped::zip_entries(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    use ContentKind::{Mods, ResourcePacks, Shaders};

    #[test]
    fn jars_are_always_mods() {
        assert_eq!(route(Some("sodium-0.6.jar"), None, Some(Shaders)), Some(Mods));
        assert_eq!(route(Some("Old.LITEMOD"), None, Some(ResourcePacks)), Some(Mods));
        assert_eq!(route(Some("Upper.JAR"), None, Some(Mods)), Some(Mods));
    }

    #[test]
    fn shader_packs_are_recognised_even_inside_their_own_folder() {
        let flat = names(&["shaders/", "shaders/composite.fsh", "shaders/lang/en_us.lang"]);
        let wrapped = names(&["Complementary/", "Complementary/shaders/final.fsh"]);
        assert_eq!(route(Some("BSL.zip"), Some(&flat), Some(Mods)), Some(Shaders));
        assert_eq!(route(Some("Comp.zip"), Some(&wrapped), Some(ResourcePacks)), Some(Shaders));
    }

    #[test]
    fn resource_packs_are_recognised_from_their_metadata() {
        let pack = names(&["pack.mcmeta", "pack.png", "assets/minecraft/textures/block/stone.png"]);
        assert_eq!(route(Some("Faithful.zip"), Some(&pack), Some(Mods)), Some(ResourcePacks));
        // a resource pack that also ships core shaders is still a resource pack
        let with_shaders = names(&["pack.mcmeta", "assets/minecraft/shaders/core/x.json"]);
        assert_eq!(route(Some("Fancy.zip"), Some(&with_shaders), Some(Shaders)), Some(ResourcePacks));
    }

    #[test]
    fn data_packs_and_other_files_are_rejected() {
        let data_pack = names(&["pack.mcmeta", "data/minecraft/tags/blocks/x.json"]);
        assert_eq!(route(Some("Terralith.zip"), Some(&data_pack), Some(ResourcePacks)), None);
        assert_eq!(route(Some("readme.txt"), None, Some(Mods)), None);
        assert_eq!(route(None, None, Some(Mods)), None);
    }

    #[test]
    fn unknown_zips_follow_the_pack_tab_they_were_dropped_on() {
        let unknown = names(&["something/else.txt"]);
        assert_eq!(route(Some("mystery.zip"), Some(&unknown), Some(Mods)), None);
        assert_eq!(route(Some("mystery.zip"), Some(&unknown), Some(Shaders)), Some(Shaders));
        assert_eq!(route(Some("broken.zip"), None, Some(ResourcePacks)), Some(ResourcePacks));
        assert_eq!(route(Some("broken.zip"), None, Some(Mods)), None);
    }

    #[test]
    fn zip_mods_are_mods() {
        assert_eq!(
            route(Some("old-forge-mod.zip"), Some(&names(&["mcmod.info", "a/B.class"])), Some(Shaders)),
            Some(Mods)
        );
    }

    #[test]
    fn reads_the_names_inside_a_real_zip() {
        let dir = std::env::temp_dir().join(format!("blockforge-routing-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("pack.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("pack.mcmeta", options).unwrap();
        z.write_all(b"{}").unwrap();
        z.start_file("assets/minecraft/lang/en_us.json", options).unwrap();
        z.finish().unwrap();
        let entries = zip_entries(&zip_path).unwrap();
        assert_eq!(entries, names(&["pack.mcmeta", "assets/minecraft/lang/en_us.json"]));
        assert_eq!(route(Some("pack.zip"), Some(&entries), Some(Mods)), Some(ResourcePacks));

        let not_zip = dir.join("fake.zip");
        std::fs::write(&not_zip, [1u8, 2, 3]).unwrap();
        assert!(zip_entries(&not_zip).is_none());
    }
}
