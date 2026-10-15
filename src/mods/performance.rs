//! A handful of well known mods that make Minecraft run faster, especially on slower computers ("Make Minecraft
//! run faster" in the content manager). They change nothing about how the game plays.

use super::versions::{self, EMBEDDIUM, FERRITE_CORE, LITHIUM, MODERN_FIX, SODIUM};

/// One job in the pack, such as "faster rendering". The first project in the list that is available for the
/// instance is used; the others do the same job and must never be installed together with it.
#[derive(Clone, Debug)]
pub struct Slot {
    pub purpose: &'static str,
    pub project_ids: Vec<&'static str>,
}

fn slot(purpose: &'static str, ids: &[&'static str]) -> Slot {
    Slot { purpose, project_ids: ids.to_vec() }
}

/// The jobs the pack does on a loader, empty when there's no pack for it.
pub fn slots_for(loader_key: Option<&str>) -> Vec<Slot> {
    match loader_key {
        Some(versions::FABRIC) | Some(versions::QUILT) => {
            vec![
                slot("rendering", &[SODIUM]),
                slot("game logic", &[LITHIUM]),
                slot("memory", &[FERRITE_CORE]),
                slot("start up", &[MODERN_FIX]),
            ]
        }
        Some(versions::NEOFORGE) => {
            vec![
                slot("rendering", &[SODIUM, EMBEDDIUM]),
                slot("game logic", &[LITHIUM]),
                slot("memory", &[FERRITE_CORE]),
                slot("start up", &[MODERN_FIX]),
            ]
        }
        Some(versions::FORGE) => {
            vec![slot("rendering", &[EMBEDDIUM]), slot("memory", &[FERRITE_CORE]), slot("start up", &[MODERN_FIX])]
        }
        _ => Vec::new(),
    }
}

/// Whether the pack can be offered for a loader at all. Each mod is still checked against the version.
pub fn offered_for(loader_key: Option<&str>) -> bool {
    !slots_for(loader_key).is_empty()
}

/// Picks what to install: project ids in order. A job already done by something installed is skipped, as is one
/// nothing is available for.
pub fn choose(slots: &[Slot], available: impl Fn(&str) -> bool, installed: &[String]) -> Vec<String> {
    let mut picked = Vec::new();
    for slot in slots {
        if slot.project_ids.iter().any(|id| installed.iter().any(|i| i == id)) {
            continue;
        }
        if let Some(id) = slot.project_ids.iter().find(|id| available(id)) {
            picked.push(id.to_string());
        }
    }
    picked
}

/// A readable name for one of the pack's projects.
pub fn name_of(project_id: &str) -> String {
    match project_id {
        SODIUM => "Sodium",
        EMBEDDIUM => "Embeddium",
        LITHIUM => "Lithium",
        FERRITE_CORE => "FerriteCore",
        MODERN_FIX => "ModernFix",
        other => other,
    }
    .to_string()
}

/// The mods offered on a loader, by name, for showing next to the option.
pub fn describe(loader_key: Option<&str>) -> String {
    slots_for(loader_key).iter().map(|s| name_of(s.project_ids[0])).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERYTHING: [&str; 5] = [SODIUM, EMBEDDIUM, LITHIUM, FERRITE_CORE, MODERN_FIX];

    fn everything(id: &str) -> bool {
        EVERYTHING.contains(&id)
    }

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn fabric_and_quilt_get_the_full_pack() {
        let expected = s(&[SODIUM, LITHIUM, FERRITE_CORE, MODERN_FIX]);
        assert_eq!(choose(&slots_for(Some(versions::FABRIC)), everything, &[]), expected);
        assert_eq!(choose(&slots_for(Some(versions::QUILT)), everything, &[]), expected);
        assert_eq!(describe(Some(versions::FABRIC)), "Sodium, Lithium, FerriteCore, ModernFix");
    }

    #[test]
    fn mods_without_a_version_for_the_game_are_skipped() {
        // ModernFix often takes a while to support a brand new Minecraft version
        let available = |id: &str| everything(id) && id != MODERN_FIX;
        assert_eq!(choose(&slots_for(Some(versions::FABRIC)), available, &[]), s(&[SODIUM, LITHIUM, FERRITE_CORE]));
    }

    #[test]
    fn neoforge_uses_embeddium_when_sodium_isnt_there() {
        assert_eq!(choose(&slots_for(Some(versions::NEOFORGE)), everything, &[])[0], SODIUM);
        let available = |id: &str| everything(id) && id != SODIUM && id != LITHIUM;
        assert_eq!(
            choose(&slots_for(Some(versions::NEOFORGE)), available, &[]),
            s(&[EMBEDDIUM, FERRITE_CORE, MODERN_FIX])
        );
    }

    #[test]
    fn forge_gets_embeddium_ferrite_core_and_modern_fix() {
        assert_eq!(
            choose(&slots_for(Some(versions::FORGE)), everything, &[]),
            s(&[EMBEDDIUM, FERRITE_CORE, MODERN_FIX])
        );
    }

    #[test]
    fn jobs_already_done_are_skipped_and_renderers_never_doubled() {
        let installed = s(&[EMBEDDIUM, LITHIUM]);
        // Embeddium already renders, so Sodium must not be added next to it
        assert_eq!(
            choose(&slots_for(Some(versions::NEOFORGE)), everything, &installed),
            s(&[FERRITE_CORE, MODERN_FIX])
        );
    }

    #[test]
    fn only_offered_for_mod_loaders() {
        for k in [versions::FABRIC, versions::QUILT, versions::NEOFORGE, versions::FORGE] {
            assert!(offered_for(Some(k)));
        }
        assert!(!offered_for(None));
        assert!(!offered_for(Some(versions::LEGACY_FABRIC)));
        assert!(choose(&slots_for(None), everything, &[]).is_empty());
    }
}
