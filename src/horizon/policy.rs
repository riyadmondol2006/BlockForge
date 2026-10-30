//! Shared rules for content and operations on Horizon instances. No downloads or writes happen here.

use crate::accounts::Account;
use crate::content::kind::ContentKind;
use crate::instances::Instance;
use crate::launch::LaunchExtras;
use crate::modrinth::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Mods,
    ResourcePacks,
    Shaders,
    DataPacks,
    Plugins,
    Worlds,
    Modpacks,
}

impl From<ContentKind> for Content {
    fn from(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Mods => Self::Mods,
            ContentKind::ResourcePacks => Self::ResourcePacks,
            ContentKind::Shaders => Self::Shaders,
        }
    }
}

impl From<Kind> for Content {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Mod => Self::Mods,
            Kind::ResourcePack => Self::ResourcePacks,
            Kind::Shader => Self::Shaders,
            Kind::DataPack => Self::DataPacks,
            Kind::Modpack => Self::Modpacks,
        }
    }
}

impl From<crate::curseforge::api::Section> for Content {
    fn from(section: crate::curseforge::api::Section) -> Self {
        use crate::curseforge::api::Section;
        match section {
            Section::Mods => Self::Mods,
            Section::Modpacks => Self::Modpacks,
            Section::ResourcePacks => Self::ResourcePacks,
            Section::Worlds => Self::Worlds,
            Section::ShaderPacks => Self::Shaders,
            Section::DataPacks => Self::DataPacks,
            Section::Plugins => Self::Plugins,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add(Content),
    ExportModpack,
    RepairJava,
    ChangeLoader,
    ChangeMinecraftVersion,
    JavaSettings,
    UpdateModpack,
}

/// Java instances retain all their existing operations. Horizon accepts resource packs and saves.
pub fn allows(inst: &Instance, action: Action) -> Result<(), String> {
    if !super::is_horizon(inst) {
        return Ok(());
    }
    let reason = match action {
        Action::Add(Content::ResourcePacks | Content::Worlds) => return Ok(()),
        Action::Add(Content::Mods) => "Java mods don't run in it.",
        Action::Add(Content::Shaders) => "Java shader packs don't run in it.",
        Action::Add(Content::DataPacks) => "data packs can't be added as instance content.",
        Action::Add(Content::Plugins) => "Java plugins don't run in it.",
        Action::Add(Content::Modpacks) | Action::UpdateModpack => "Java modpacks can't be installed or updated in it.",
        Action::ExportModpack => "exporting it as a Java modpack isn't supported in any format.",
        Action::RepairJava => "Java game-file repair doesn't apply to it.",
        Action::ChangeLoader => "Java mod loaders don't run in it.",
        Action::ChangeMinecraftVersion => "its Minecraft version is chosen by the Horizon build.",
        Action::JavaSettings => "Java and memory settings don't apply to it.",
    };
    Err(format!("Horizon plays Minecraft rewritten in Rust; {reason}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Java,
    Horizon,
}

/// Choose the launch path before Java preflight, including for a dry run. `java_options` means explicit
/// Java/memory options (currently the CLI's `--yes` preflight override). Saved Java settings are ignored.
pub fn launch_engine(
    inst: &Instance,
    account: &Account,
    extras: &LaunchExtras,
    java_options: bool,
) -> Result<Engine, String> {
    if !super::is_horizon(inst) {
        return Ok(Engine::Java);
    }
    if extras.offline_name.is_some() {
        return Err(
            "Horizon doesn't support --offline-name; use a Microsoft account that owns Minecraft: Java Edition.".into(),
        );
    }
    if extras.realm.is_some() {
        return Err("Horizon doesn't support --realm (Realms).".into());
    }
    if java_options {
        return Err("Horizon doesn't support Java/memory options or --yes (Java preflight).".into());
    }
    if let Some(reason) = super::account_problem(account) {
        return Err(reason);
    }
    Ok(Engine::Horizon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_for_both_engines() {
        let mut inst =
            Instance { folder: "policy".into(), root: Default::default(), mod_count: 0, file: Default::default() };
        let cases = [
            (Action::Add(Content::ResourcePacks), true),
            (Action::Add(Content::Worlds), true),
            (Action::Add(Content::Mods), false),
            (Action::Add(Content::Shaders), false),
            (Action::Add(Content::DataPacks), false),
            (Action::Add(Content::Plugins), false),
            (Action::Add(Content::Modpacks), false),
            (Action::ExportModpack, false),
            (Action::RepairJava, false),
            (Action::ChangeLoader, false),
            (Action::ChangeMinecraftVersion, false),
            (Action::JavaSettings, false),
            (Action::UpdateModpack, false),
        ];
        for horizon in [false, true] {
            if horizon {
                super::super::mark(&mut inst);
            }
            for (action, allowed) in cases {
                let result = allows(&inst, action);
                assert_eq!(result.is_ok(), !horizon || allowed, "{horizon}: {action:?}");
                if let Err(reason) = result {
                    assert!(reason.starts_with("Horizon plays Minecraft rewritten in Rust;"));
                }
            }
        }
    }

    #[test]
    fn existing_content_kinds_map_to_policy() {
        for kind in ContentKind::ALL {
            let api = Kind::from_api(kind.project_type());
            assert_eq!(Content::from(kind), Content::from(api));
        }
        use crate::curseforge::api::Section;
        for (section, expected) in [
            (Section::Mods, Content::Mods),
            (Section::Modpacks, Content::Modpacks),
            (Section::ResourcePacks, Content::ResourcePacks),
            (Section::Worlds, Content::Worlds),
            (Section::ShaderPacks, Content::Shaders),
            (Section::DataPacks, Content::DataPacks),
            (Section::Plugins, Content::Plugins),
        ] {
            assert_eq!(Content::from(section), expected);
        }
        assert_eq!(Content::from(Kind::DataPack), Content::DataPacks);
        assert_eq!(Content::from(Kind::Modpack), Content::Modpacks);
    }

    #[test]
    fn launch_routes_and_validates_accounts_and_options() {
        let mut inst =
            Instance { folder: "route".into(), root: Default::default(), mod_count: 0, file: Default::default() };
        let offline = Account::offline("Player");
        let online = Account { access_token: Some("token".into()), ..Default::default() };
        let cases = [
            (&online, LaunchExtras::default(), false, true),
            (&offline, LaunchExtras::default(), false, false),
            (&Account::default(), LaunchExtras::default(), false, false),
            (&online, LaunchExtras { offline_name: Some("Player".into()), ..Default::default() }, false, false),
            (&online, LaunchExtras { realm: Some("123".into()), ..Default::default() }, false, false),
            (&online, LaunchExtras::default(), true, false),
        ];
        for (account, extras, java_options, _) in &cases {
            assert_eq!(launch_engine(&inst, account, extras, *java_options), Ok(Engine::Java));
        }
        super::super::mark(&mut inst);
        for (account, extras, java_options, allowed) in &cases {
            assert_eq!(launch_engine(&inst, account, extras, *java_options).is_ok(), *allowed);
        }
        assert_eq!(launch_engine(&inst, &online, &LaunchExtras::default(), false), Ok(Engine::Horizon));
    }
}
