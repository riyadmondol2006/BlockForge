//! A handful of well known mods that make Minecraft run faster, especially on slower computers. They change nothing
//! about how the game plays.

use super::loader_support::{self, EMBEDDIUM, SODIUM};

pub const LITHIUM: &str = "gvQqBUqZ";
pub const FERRITE_CORE: &str = "uXXizFIs";
pub const MODERN_FIX: &str = "nmDcB62a";

/// One job in the pack, such as "faster rendering". The first project in the list that is available for the
/// instance is used; the others do the same job and must never be installed together with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub purpose: &'static str,
    pub project_ids: Vec<&'static str>,
}

impl Slot {
    fn new(purpose: &'static str, project_ids: &[&'static str]) -> Slot {
        Slot { purpose, project_ids: project_ids.to_vec() }
    }
}

/// The jobs the pack does on a loader, empty when there's no pack for it.
pub fn slots_for(loader_key: Option<&str>) -> Vec<Slot> {
    match loader_key {
        Some(loader_support::FABRIC) | Some(loader_support::QUILT) => vec![
            Slot::new("rendering", &[SODIUM]),
            Slot::new("game logic", &[LITHIUM]),
            Slot::new("memory", &[FERRITE_CORE]),
            Slot::new("start up", &[MODERN_FIX]),
        ],
        Some(loader_support::NEOFORGE) => vec![
            Slot::new("rendering", &[SODIUM, EMBEDDIUM]),
            Slot::new("game logic", &[LITHIUM]),
            Slot::new("memory", &[FERRITE_CORE]),
            Slot::new("start up", &[MODERN_FIX]),
        ],
        Some(loader_support::FORGE) => vec![
            Slot::new("rendering", &[EMBEDDIUM]),
            Slot::new("memory", &[FERRITE_CORE]),
            Slot::new("start up", &[MODERN_FIX]),
        ],
        _ => Vec::new(),
    }
}

/// Whether the pack can be offered for a loader at all. Each mod is still checked against the version.
pub fn offered_for(loader_key: Option<&str>) -> bool {
    !slots_for(loader_key).is_empty()
}

/// Picks what to install.
///
/// * `slots`: the jobs, from [`slots_for`]
/// * `available`: whether a project has a version for this instance's Minecraft version and loader
/// * `installed`: projects already in the instance
///
/// Returns project ids to install, in order; a job already done by something installed is skipped, as is one
/// nothing is available for.
pub fn choose<I: AsRef<str>>(slots: &[Slot], available: impl Fn(&str) -> bool, installed: &[I]) -> Vec<String> {
    let mut picked = Vec::new();
    for slot in slots {
        let done = slot.project_ids.iter().any(|id| installed.iter().any(|i| i.as_ref() == *id));
        if done {
            continue;
        }
        if let Some(id) = slot.project_ids.iter().find(|id| available(id)) {
            picked.push(id.to_string());
        }
    }
    picked
}

/// A readable name for one of the pack's projects (the project id itself for any other).
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
    use crate::content::loader_support::{FABRIC, FORGE, LEGACY_FABRIC, NEOFORGE, QUILT};

    const EVERYTHING: [&str; 5] = [SODIUM, EMBEDDIUM, LITHIUM, FERRITE_CORE, MODERN_FIX];
    const NONE: [&str; 0] = [];

    fn all(id: &str) -> bool {
        EVERYTHING.contains(&id)
    }

    #[test]
    fn fabric_and_quilt_get_the_full_pack() {
        let expected = vec![SODIUM, LITHIUM, FERRITE_CORE, MODERN_FIX];
        assert_eq!(choose(&slots_for(Some(FABRIC)), all, &NONE), expected);
        assert_eq!(choose(&slots_for(Some(QUILT)), all, &NONE), expected);
        assert_eq!(describe(Some(FABRIC)), "Sodium, Lithium, FerriteCore, ModernFix");
    }

    #[test]
    fn mods_without_a_version_for_the_game_are_skipped() {
        // ModernFix often takes a while to support a brand new Minecraft version
        let avail = |id: &str| all(id) && id != MODERN_FIX;
        assert_eq!(choose(&slots_for(Some(FABRIC)), avail, &NONE), vec![SODIUM, LITHIUM, FERRITE_CORE]);
    }

    #[test]
    fn neoforge_uses_embeddium_when_sodium_isnt_there() {
        assert_eq!(choose(&slots_for(Some(NEOFORGE)), all, &NONE)[0], SODIUM);
        let avail = |id: &str| all(id) && id != SODIUM && id != LITHIUM;
        assert_eq!(choose(&slots_for(Some(NEOFORGE)), avail, &NONE), vec![EMBEDDIUM, FERRITE_CORE, MODERN_FIX]);
    }

    #[test]
    fn forge_gets_embeddium_ferritecore_and_modernfix() {
        assert_eq!(choose(&slots_for(Some(FORGE)), all, &NONE), vec![EMBEDDIUM, FERRITE_CORE, MODERN_FIX]);
    }

    #[test]
    fn jobs_already_done_are_skipped_and_renderers_are_never_doubled() {
        // Embeddium already renders, so Sodium must not be added next to it
        let installed = [EMBEDDIUM, LITHIUM];
        assert_eq!(choose(&slots_for(Some(NEOFORGE)), all, &installed), vec![FERRITE_CORE, MODERN_FIX]);
    }

    #[test]
    fn only_offered_for_mod_loaders() {
        assert!(offered_for(Some(FABRIC)));
        assert!(offered_for(Some(QUILT)));
        assert!(offered_for(Some(NEOFORGE)));
        assert!(offered_for(Some(FORGE)));
        assert!(!offered_for(None));
        assert!(!offered_for(loader_support::key(Some("Paper"))));
        assert!(!offered_for(Some(LEGACY_FABRIC)));
        assert!(choose(&slots_for(None), all, &NONE).is_empty());
        assert_eq!(name_of("zzz"), "zzz");
    }
}
