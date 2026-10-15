//! What each mod loader can run, in Modrinth's terms, and what a shader pack needs to work.

use crate::loaders::Loader;

pub const FABRIC: &str = "fabric";
pub const QUILT: &str = "quilt";
pub const FORGE: &str = "forge";
pub const NEOFORGE: &str = "neoforge";
pub const LEGACY_FABRIC: &str = "legacy-fabric";

pub const IRIS: &str = "YL57xq9U";
pub const SODIUM: &str = "AANobbMI";
pub const OCULUS: &str = "GchcoXML";
pub const EMBEDDIUM: &str = "sk9rgfiA";

/// The Fabric API mod on Modrinth, and Quilt's stand-in for it (QSL).
pub const FABRIC_API: &str = "P7dR8mSH";
pub const QSL: &str = "qvIfYCYJ";
/// The Fabric API mod on CurseForge.
pub const CURSEFORGE_FABRIC_API: i64 = 306612;

/// Minecraft versions where NeoForge still runs Forge mods. The Java edition downloads this list with its remote
/// config (`loaders.neoforge.forgeCompatibleMinecraftVersions`); NeoForge started on 1.20.1, which is the one
/// version it has ever been true for.
pub const DEFAULT_NEOFORGE_RUNS_FORGE_ON: &[&str] = &["1.20.1"];

/// What a shader pack needs before it does anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShaderSetup {
    /// Iris, Oculus or OptiFine is already there
    Ready,
    /// Iris (with Sodium) can be added from Modrinth
    AddIris,
    /// Forge: Oculus can be added from Modrinth if it exists for this version, otherwise OptiFine is needed
    AddOculus,
    /// no mod loader, or one no shader loader is made for
    NeedsModLoader,
}

/// Modrinth's name for an instance's loader (`instance.json` calls it `Fabric`, `Quilt`, `Forge`, `NeoForge`,
/// `LegacyFabric`), or `None` for plain Minecraft and server-only loaders.
pub fn key(loader_type: Option<&str>) -> Option<&'static str> {
    let t = loader_type?;
    if t.eq_ignore_ascii_case("Fabric") {
        Some(FABRIC)
    } else if t.eq_ignore_ascii_case("Quilt") {
        Some(QUILT)
    } else if t.eq_ignore_ascii_case("NeoForge") {
        Some(NEOFORGE)
    } else if t.eq_ignore_ascii_case("Forge") {
        Some(FORGE)
    } else if t.eq_ignore_ascii_case("LegacyFabric") || t.eq_ignore_ascii_case("Legacy Fabric") {
        Some(LEGACY_FABRIC)
    } else {
        None
    }
}

/// Modrinth's name for a loader picked on the New instance page.
pub fn key_for_loader(loader: Loader) -> Option<&'static str> {
    match loader {
        Loader::Fabric => Some(FABRIC),
        Loader::Quilt => Some(QUILT),
        Loader::NeoForge => Some(NEOFORGE),
        Loader::Forge => Some(FORGE),
        Loader::Vanilla => None,
    }
}

/// The loaders whose mods run on this loader: Quilt runs Fabric mods, and NeoForge runs Forge mods on the
/// Minecraft versions where the two are still compatible.
pub fn accepted_loaders(
    key: Option<&str>,
    minecraft_version: &str,
    neoforge_runs_forge_mods_on: &[String],
) -> Vec<String> {
    let Some(key) = key else { return Vec::new() };
    let mut loaders = vec![key.to_string()];
    if key == QUILT {
        loaders.push(FABRIC.to_string());
    }
    if key == NEOFORGE && neoforge_runs_forge_mods_on.iter().any(|v| v == minecraft_version) {
        loaders.push(FORGE.to_string());
    }
    loaders
}

/// Whether a mod loader is there that shader packs work with.
pub fn has_shader_loader<P: AsRef<str>, F: AsRef<str>>(modrinth_project_ids: &[P], file_names: &[F]) -> bool {
    if modrinth_project_ids.iter().any(|id| id.as_ref() == IRIS || id.as_ref() == OCULUS) {
        return true;
    }
    file_names.iter().any(|f| {
        let name = f.as_ref().to_lowercase();
        name.starts_with("iris")
            || name.starts_with("oculus")
            || name.contains("optifine")
            || name.starts_with("optifabric")
    })
}

/// What has to happen before shaders work on an instance with this loader.
pub fn shader_setup(key: Option<&str>, has_shader_loader: bool) -> ShaderSetup {
    if has_shader_loader {
        return ShaderSetup::Ready;
    }
    match key {
        Some(FABRIC) | Some(QUILT) | Some(NEOFORGE) => ShaderSetup::AddIris,
        Some(FORGE) => ShaderSetup::AddOculus,
        _ => ShaderSetup::NeedsModLoader,
    }
}

/// A readable name for the loader, for messages.
pub fn display_name(key: Option<&str>) -> String {
    match key {
        None => "Minecraft".to_string(),
        Some(FABRIC) => "Fabric".to_string(),
        Some(QUILT) => "Quilt".to_string(),
        Some(FORGE) => "Forge".to_string(),
        Some(NEOFORGE) => "NeoForge".to_string(),
        Some(LEGACY_FABRIC) => "Legacy Fabric".to_string(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_loader_of_an_instance() {
        assert_eq!(key(Some("Fabric")), Some(FABRIC));
        assert_eq!(key(Some("Quilt")), Some(QUILT));
        assert_eq!(key(Some("NeoForge")), Some(NEOFORGE));
        assert_eq!(key(Some("Forge")), Some(FORGE));
        assert_eq!(key(Some("LegacyFabric")), Some(LEGACY_FABRIC));
        assert_eq!(key(None), None);
        assert_eq!(key(Some("Paper")), None);
        assert_eq!(key_for_loader(Loader::Vanilla), None);
        assert_eq!(key_for_loader(Loader::NeoForge), Some(NEOFORGE));
    }

    #[test]
    fn quilt_runs_fabric_mods_and_neoforge_sometimes_runs_forge_mods() {
        assert_eq!(accepted_loaders(Some("quilt"), "26.3", &[]), v(&["quilt", "fabric"]));
        assert_eq!(accepted_loaders(Some("fabric"), "26.3", &[]), v(&["fabric"]));
        assert_eq!(accepted_loaders(Some("neoforge"), "1.20.1", &v(&["1.20.1"])), v(&["neoforge", "forge"]));
        assert_eq!(accepted_loaders(Some("neoforge"), "1.21.1", &v(&["1.20.1"])), v(&["neoforge"]));
        assert!(accepted_loaders(None, "26.3", &[]).is_empty());
    }

    #[test]
    fn finds_shader_loaders_by_project_or_file_name() {
        let none: [&str; 0] = [];
        assert!(has_shader_loader(&[IRIS], &none));
        assert!(has_shader_loader(&none, &["iris-fabric-1.8.0.jar"]));
        assert!(has_shader_loader(&none, &["OptiFine_1.20.1_HD_U_I6.jar"]));
        assert!(has_shader_loader(&none, &["oculus-mc1.20.1-1.7.0.jar"]));
        assert!(!has_shader_loader(&[SODIUM], &["sodium-fabric-0.6.jar"]));
    }

    #[test]
    fn says_what_shaders_need_per_loader() {
        assert_eq!(shader_setup(Some(FABRIC), true), ShaderSetup::Ready);
        assert_eq!(shader_setup(Some(FABRIC), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(QUILT), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(NEOFORGE), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(FORGE), false), ShaderSetup::AddOculus);
        assert_eq!(shader_setup(None, false), ShaderSetup::NeedsModLoader);
        assert_eq!(shader_setup(Some(LEGACY_FABRIC), false), ShaderSetup::NeedsModLoader);
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name(None), "Minecraft");
        assert_eq!(display_name(Some(NEOFORGE)), "NeoForge");
        assert_eq!(display_name(Some(LEGACY_FABRIC)), "Legacy Fabric");
        assert_eq!(display_name(Some("x")), "x");
    }
}
