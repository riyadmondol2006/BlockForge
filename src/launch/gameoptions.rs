//! Graphics settings for a brand new game, picked for this computer, so a budget laptop doesn't start at Minecraft's
//! 12 chunks and fancy graphics and a gaming PC isn't held back. Written only when the game has no `options.txt` yet:
//! the player's own settings, and a modpack's, are never touched. A port of the Java edition's `GameOptionsDefaults`
//! (the `autoGameOptions` setting turns it off).

use std::io::Read;
use std::path::Path;

use anyhow::Result;

use crate::launch::memory::{Tier, minor_version};

/// The settings for a computer of this tier, with the option names this Minecraft version uses, in the order they
/// are written. Empty for versions older than 1.7, whose options are too different to be worth it.
///
/// `graphics_presets` is true when the game has graphics presets (1.21.11 and newer): there the preset decides most
/// settings when the game starts, so the preset is what has to be chosen.
pub fn for_tier(tier: Tier, minecraft_version: &str, graphics_presets: bool) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| options.push((k.to_string(), v));
    let minor = minor_version(Some(minecraft_version));
    if minor < 7 {
        return options;
    }

    if graphics_presets {
        match tier {
            Tier::Low => {
                // Mojang's own "Fast" preset: 8 chunks, simulation 6, decreased particles, no entity shadows, no smooth
                // lighting; and 60 FPS to keep a laptop cool
                put("graphicsPreset", "\"fast\"".into());
                put("maxFps", "60".into());
            }
            Tier::High => {
                // "Fancy" (16 chunks, simulation 12) is what the game picks itself, and right for a gaming PC
                put("graphicsPreset", "\"fancy\"".into());
                put("maxFps", "260".into());
            }
            Tier::Medium => {
                // "Fancy" with a shorter view: Fancy's 16 chunks are too much for most laptops
                put("graphicsPreset", "\"custom\"".into());
                put("renderDistance", "10".into());
                put("simulationDistance", "8".into());
                put("particles", "0".into());
                put("maxFps", "120".into());
                put("entityShadows", "true".into());
            }
        }
        return options;
    }

    // particles: 0 all, 1 decreased, 2 minimal; 260 FPS means unlimited
    let (render, simulation, fancy, particles, max_fps, shadows) = match tier {
        // a budget laptop: half of the default view, fast graphics and fewer particles keep it smooth, and 60 FPS
        // keeps it cool and quiet
        Tier::Low => (6, 5, false, 1, 60, false),
        // a gaming PC: a little more view than the default (each step adds a ring of chunks, so 14 is about a third
        // more than 12 and fits in the automatic memory); fabulous graphics stay off, as they cost a lot and clash
        // with shader packs
        Tier::High => (if minor <= 12 { 12 } else { 14 }, 12, true, 0, 260, true),
        // a bit under Minecraft's own defaults (12 chunks), which are too much for most laptops
        Tier::Medium => (10, 8, true, 0, 120, true),
    };
    put("renderDistance", render.to_string());
    if minor >= 18 {
        put("simulationDistance", simulation.to_string());
    }
    if minor >= 16 {
        put("graphicsMode", if fancy { "1" } else { "0" }.into());
    } else {
        put("fancyGraphics", fancy.to_string());
    }
    put("particles", particles.to_string());
    if minor >= 8 {
        put("maxFps", max_fps.to_string());
        put("entityShadows", shadows.to_string());
    }
    options
}

/// The text of `options.txt`. From 1.14 on the file has to say which data version it was written for: without it,
/// Minecraft treats the file as ancient and "upgrades" it, which turns `graphicsMode:1` back into fast graphics.
pub fn to_text(options: &[(String, String)], data_version: Option<i64>) -> String {
    let mut out = String::new();
    if let Some(v) = data_version {
        out.push_str(&format!("version:{v}\n"));
    }
    for (k, v) in options {
        out.push_str(&format!("{k}:{v}\n"));
    }
    out
}

/// If the game already has settings, or a mod that hands out a modpack's default settings is set up.
pub fn has_own_options(game_directory: &Path) -> bool {
    game_directory.join("options.txt").exists()
        // "Your Options Shall Be Respected" and "Default Options" copy the pack's settings in on the first start, but
        // only when there is no options.txt yet
        || game_directory.join("config/yosbr/options.txt").exists()
        || game_directory.join("config/defaultoptions/options.txt").exists()
}

/// Writes `options.txt` for a game that has none yet. `client_jar` is the game's jar, to read its data version from.
/// Returns what was written, or None when nothing was.
pub fn write_if_missing(
    game_directory: &Path,
    tier: Tier,
    minecraft_version: &str,
    client_jar: Option<&Path>,
) -> Result<Option<Vec<(String, String)>>> {
    if has_own_options(game_directory) {
        return Ok(None);
    }
    let data_version = client_jar.and_then(read_data_version);
    let options = for_tier(tier, minecraft_version, client_jar.is_some_and(has_graphics_presets));
    if options.is_empty() {
        return Ok(None);
    }
    // 1.14 and newer need the data version (see to_text); if it can't be found, leave the defaults alone
    if data_version.is_none() && minor_version(Some(minecraft_version)) >= 14 {
        return Ok(None);
    }
    std::fs::create_dir_all(game_directory)?;
    // create_new: never overwrite, even if the game made the file in the meantime
    let path = game_directory.join("options.txt");
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(to_text(&options, data_version).as_bytes())?;
            Ok(Some(options))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn read_entry(jar: &Path, name: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

/// If the game has graphics presets (Fast, Fancy, Fabulous, Custom; 1.21.11 and newer). Told from its English texts,
/// as class names are obfuscated in some versions.
pub fn has_graphics_presets(client_jar: &Path) -> bool {
    read_entry(client_jar, "assets/minecraft/lang/en_us.json")
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get("options.graphics.preset").is_some())
}

/// The `world_version` from `version.json` in a client jar (1.14 and newer).
pub fn read_data_version(client_jar: &Path) -> Option<i64> {
    let text = read_entry(client_jar, "version.json")?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get("world_version")?.as_i64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn map(options: &[(String, String)]) -> std::collections::HashMap<&str, &str> {
        options.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }

    #[test]
    fn the_options_depend_on_the_tier_and_version() {
        let low = for_tier(Tier::Low, "1.21.1", false);
        let m = map(&low);
        assert_eq!(
            (m["renderDistance"], m["simulationDistance"], m["graphicsMode"], m["maxFps"], m["entityShadows"]),
            ("6", "5", "0", "60", "false")
        );
        let high = for_tier(Tier::High, "1.21.1", false);
        let m = map(&high);
        assert_eq!(m["renderDistance"], "14");
        assert_eq!(m["maxFps"], "260");
        // 1.12 and older: no simulation distance, old graphics option, and a smaller high-end view
        let old = for_tier(Tier::High, "1.12.2", false);
        let m = map(&old);
        assert_eq!(m["renderDistance"], "12");
        assert_eq!(m["fancyGraphics"], "true");
        assert!(!m.contains_key("simulationDistance") && !m.contains_key("graphicsMode"));
        // 1.7 has no max FPS or shadows options; before 1.7 nothing is written at all
        let m17 = for_tier(Tier::Medium, "1.7.10", false);
        assert!(!map(&m17).contains_key("maxFps"));
        assert!(for_tier(Tier::Medium, "1.6.4", false).is_empty());
        assert!(for_tier(Tier::Medium, "b1.7.3", false).is_empty());
        // the first option is always the view
        assert_eq!(low[0].0, "renderDistance");
    }

    #[test]
    fn presets_win_where_the_game_has_them() {
        let low = for_tier(Tier::Low, "26.1", true);
        assert_eq!(
            low,
            vec![("graphicsPreset".to_string(), "\"fast\"".to_string()), ("maxFps".to_string(), "60".to_string())]
        );
        let medium = map(&for_tier(Tier::Medium, "26.1", true)).len();
        assert_eq!(medium, 6);
    }

    #[test]
    fn options_text() {
        let text = to_text(&[("renderDistance".into(), "10".into())], Some(3955));
        assert_eq!(text, "version:3955\nrenderDistance:10\n");
        assert_eq!(to_text(&[("a".into(), "b".into())], None), "a:b\n");
    }

    fn client_jar(dir: &Path, world_version: Option<i64>) -> std::path::PathBuf {
        let path = dir.join("client.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        if let Some(v) = world_version {
            zip.start_file("version.json", opts).unwrap();
            zip.write_all(format!("{{\"id\":\"x\",\"world_version\":{v}}}").as_bytes()).unwrap();
        }
        zip.start_file("assets/minecraft/lang/en_us.json", opts).unwrap();
        zip.write_all(br#"{"options.graphics.preset":"Graphics Preset"}"#).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn only_a_game_without_options_gets_them() {
        let dir = std::env::temp_dir().join(format!("bf-opts-{}", std::process::id()));
        let game = dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let jar = client_jar(&dir, Some(3955));
        assert_eq!(read_data_version(&jar), Some(3955));
        assert!(has_graphics_presets(&jar));

        let written = write_if_missing(&game, Tier::Low, "1.21.1", Some(&jar)).unwrap().unwrap();
        assert_eq!(written[0].0, "graphicsPreset");
        let text = std::fs::read_to_string(game.join("options.txt")).unwrap();
        assert!(text.starts_with("version:3955\ngraphicsPreset:\"fast\"\n"));
        // the second time the file is the player's
        assert!(write_if_missing(&game, Tier::High, "1.21.1", Some(&jar)).unwrap().is_none());
        // a pack's default-options mods count as the player's too
        let other = dir.join("other");
        std::fs::create_dir_all(other.join("config/yosbr")).unwrap();
        std::fs::write(other.join("config/yosbr/options.txt"), "x:1").unwrap();
        assert!(write_if_missing(&other, Tier::Low, "1.21.1", Some(&jar)).unwrap().is_none());
        assert!(!other.join("options.txt").exists());
        // 1.14 and newer without a data version are left alone, older ones don't need it
        let nojar = client_jar(&dir.join(""), None);
        let third = dir.join("third");
        assert!(write_if_missing(&third, Tier::Low, "1.20.1", Some(&nojar)).unwrap().is_none());
        assert!(write_if_missing(&third, Tier::Low, "1.8.9", None).unwrap().is_some());
        assert!(!std::fs::read_to_string(third.join("options.txt")).unwrap().starts_with("version:"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
