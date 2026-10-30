//! The two games an instance can play: Minecraft: Java Edition, and Horizon, Minecraft rewritten in Rust by the
//! Horizon project (see `crate::horizon`). What differs between them in the window is here: how an instance's game is
//! named where its loader would be, what of BlockForge applies to a Horizon instance, the credit shown wherever Horizon
//! is offered, and the one start both games go through, so the log, Stop, the play time and the end are the same.

use egui::{RichText, Ui};

use super::instance_menu::Act;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab};
use crate::horizon;
use crate::instances::Instance;
use crate::launch;

/// A game ready to start: Minecraft: Java Edition, got ready by `launch`, or Horizon, got ready by `horizon`.
pub(super) enum Start {
    Java(launch::Prepared),
    Horizon(horizon::Start),
}

impl Start {
    /// What was decided on the way, for the log.
    pub(super) fn notes(&self) -> &[String] {
        match self {
            Start::Java(p) => &p.notes,
            Start::Horizon(s) => &s.notes,
        }
    }

    /// The command line with the account's token left out, safe to show.
    pub(super) fn display(&self) -> String {
        match self {
            Start::Java(p) => p.display(),
            Start::Horizon(s) => s.display(),
        }
    }

    pub(super) fn spawn(&self) -> anyhow::Result<launch::Running> {
        match self {
            Start::Java(p) => launch::spawn(p),
            Start::Horizon(s) => s.spawn(),
        }
    }

    /// The game closed, or never started: what the start left behind is let go.
    pub(super) fn cleanup(&self) {
        match self {
            Start::Java(p) => p.cleanup(),
            Start::Horizon(s) => s.cleanup(),
        }
    }

    /// Whether this is Minecraft: Java Edition, whose helpers and crash advice are about Java.
    pub(super) fn is_java(&self) -> bool {
        matches!(self, Start::Java(_))
    }
}

/// The game a new instance plays, as New instance offers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Edition {
    #[default]
    Java,
    Horizon,
}

/// What Explore (or a dropped file, or Add from GitHub) can put into an instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Content {
    Mods,
    Modpacks,
    ResourcePacks,
    Shaders,
    DataPacks,
    Worlds,
    Plugins,
}

impl From<crate::modrinth::Kind> for Content {
    fn from(kind: crate::modrinth::Kind) -> Content {
        use crate::modrinth::Kind;
        match kind {
            Kind::Mod => Content::Mods,
            Kind::Modpack => Content::Modpacks,
            Kind::ResourcePack => Content::ResourcePacks,
            Kind::Shader => Content::Shaders,
            Kind::DataPack => Content::DataPacks,
        }
    }
}

impl From<crate::curseforge::api::Section> for Content {
    fn from(section: crate::curseforge::api::Section) -> Content {
        use crate::curseforge::api::Section;
        match section {
            Section::Mods => Content::Mods,
            Section::Modpacks => Content::Modpacks,
            Section::ResourcePacks => Content::ResourcePacks,
            Section::Worlds => Content::Worlds,
            Section::ShaderPacks => Content::Shaders,
            Section::DataPacks => Content::DataPacks,
            Section::Plugins => Content::Plugins,
        }
    }
}

/// Whether `content` can go into `inst`. Anything can go into a Java Edition instance (Explore says what fits);
/// Horizon runs no Java mods and no loader, so only what it reads as the game does goes into one of its instances:
/// resource packs and worlds.
pub(super) fn takes(inst: &Instance, content: Content) -> bool {
    !horizon::is_horizon(inst) || matches!(content, Content::ResourcePacks | Content::Worlds)
}

/// Whether the instance menu's `act` means something for a Horizon instance. Its mods, its loader, the helpers that
/// make Java start faster, a modpack export and a repair of Minecraft's files don't; nor does playing offline, which
/// Horizon doesn't start for.
pub(super) fn horizon_allows(act: &Act) -> bool {
    !matches!(
        act,
        Act::PlayOffline(_)
            | Act::FastStart(..)
            | Act::Helper(..)
            | Act::Mods(_)
            | Act::AddMods(_)
            | Act::ChangeLoader(_)
            | Act::Export(_)
            | Act::Repair(_)
    )
}

/// The instance an act of the instance menu is about, if it is about one.
pub(super) fn act_folder(act: &Act) -> Option<&str> {
    match act {
        Act::AddAccount => None,
        Act::Open(f)
        | Act::Play(f)
        | Act::PlayOffline(f)
        | Act::PlayAs(f, _)
        | Act::FastStart(f, _)
        | Act::Helper(f, _)
        | Act::Stop(f)
        | Act::StopSession(f)
        | Act::Folder(f)
        | Act::Mods(f)
        | Act::AddMods(f)
        | Act::Settings(f)
        | Act::Rename(f)
        | Act::ChangeImage(f)
        | Act::ChangeDescription(f)
        | Act::Duplicate(f)
        | Act::Export(f)
        | Act::ChangeLoader(f)
        | Act::Repair(f)
        | Act::Delete(f) => Some(f),
    }
}

/// Whether a tab of the instance page is there for `inst`: a Horizon instance has no Mods tab.
pub(super) fn has_tab(inst: &Instance, tab: InstTab) -> bool {
    !(horizon::is_horizon(inst) && tab == InstTab::Mods)
}

/// What an instance plays besides Minecraft, as its cards, the Library's filters and its cover name it: its mod loader
/// (`Fabric`), `Horizon`, or nothing for plain Minecraft.
pub(super) fn loader_kind(inst: &Instance) -> Option<&str> {
    if horizon::is_horizon(inst) { Some(horizon::NAME) } else { inst.loader().map(|(kind, _)| kind) }
}

/// The same with its version: `("Fabric", "0.16.5")`, or `("Horizon", "v0.2.97")` with the build installed
/// (`build`; empty when there is none yet).
pub(super) fn loader_parts<'a>(inst: &'a Instance, build: Option<&'a str>) -> Option<(&'a str, &'a str)> {
    if horizon::is_horizon(inst) { Some((horizon::NAME, build.unwrap_or(""))) } else { inst.loader() }
}

/// "Fabric 0.16.5", or the name alone when there is no version to say.
pub(super) fn joined(kind: &str, version: &str) -> String {
    if version.is_empty() { kind.to_string() } else { format!("{kind} {version}") }
}

/// Horizon's small labels beside its name: what it is, and that it is early days.
pub(super) fn horizon_tags(ui: &mut Ui) {
    tag(ui, "Rust rewrite", loader_color(horizon::NAME));
    tag(ui, "Experimental", c_secondary());
}

/// Who makes Horizon, in a paragraph, and the links to its website and its source. Returns the address pressed.
pub(super) fn credit(ui: &mut Ui) -> Option<String> {
    ui.add(egui::Label::new(RichText::new(horizon::CREDIT).size(FS_META).color(c_muted())).wrap());
    super::about::links(
        ui,
        &[("playhorizon.dev", horizon::WEBSITE.to_string()), ("Source on GitHub", horizon::REPOSITORY.to_string())],
    )
}

impl App {
    /// The tag of the newest Horizon build installed (`v0.2.97`), as it was when the instances were last read.
    pub(super) fn horizon_tag(&self) -> Option<&str> {
        self.horizon_build.as_ref().map(|b| b.tag.as_str())
    }

    /// The Minecraft version Horizon plays: the installed build's, or the one its builds pin now.
    pub(super) fn horizon_minecraft(&self) -> String {
        self.horizon_build.as_ref().map(|b| b.minecraft.clone()).unwrap_or_else(|| horizon::MINECRAFT_VERSION.into())
    }

    /// Look again at which Horizon build is installed (no network): after the instances are read, which is after
    /// every job, so a build that a job installed shows at once.
    pub(super) fn refresh_horizon_build(&mut self) {
        // the screenshot tour shows a build of its own (see `shot_mark_horizon`)
        if self.shot.is_none() {
            self.horizon_build = horizon::installed_build(&self.dirs);
        }
    }

    /// The card on a Horizon instance's Overview: what it is, the build it plays, and who makes it.
    pub(super) fn horizon_about_card(&mut self, ui: &mut Ui) {
        let mut link = None;
        let tag = self.horizon_tag().map(str::to_string);
        let mc = self.horizon_minecraft();
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(format!("About {}", horizon::NAME)).size(FS_SECTION).semibold());
                horizon_tags(ui);
            });
            let line = format!(
                "{}, by the Horizon project. Java mods, loaders and Java settings are not for it.",
                horizon::TAGLINE
            );
            ui.add(egui::Label::new(RichText::new(line).color(c_muted())).wrap());
            ui.add_space(2.0);
            facts(
                ui,
                &[
                    ("Build", tag.unwrap_or_else(|| "None yet".into())),
                    ("Minecraft", mc),
                    ("Account", "Microsoft".into()),
                ],
            );
            ui.add_space(4.0);
            link = credit(ui);
        });
        if let Some(url) = link {
            self.open_link(&url);
        }
    }

    /// For the screenshot tour only: the sample instance of Minecraft 26.3 shown as a Horizon instance, in memory (its
    /// file is never written), with a build to show.
    pub(super) fn shot_mark_horizon(&mut self) {
        if let Some(inst) = self.instances.iter_mut().find(|i| i.folder == "Minecraft263") {
            horizon::mark(inst);
            inst.file.launcher.loader_version = None;
            inst.file.launcher.name = "Horizon".into();
            inst.mod_count = 0;
        }
        self.horizon_build = Some(horizon::Build {
            tag: "v0.2.97".into(),
            minecraft: horizon::MINECRAFT_VERSION.into(),
            prepared: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(horizon: bool) -> Instance {
        let mut file = crate::instances::InstanceFile::default();
        file.version.id = "26.3".into();
        file.launcher.loader_version = (!horizon).then(|| crate::instances::LoaderVersion {
            kind: "Fabric".into(),
            version: "0.16.5".into(),
            ..Default::default()
        });
        let mut inst = Instance { folder: "A".into(), mod_count: 0, root: "/nonexistent".into(), file };
        if horizon {
            horizon::mark(&mut inst);
        }
        inst
    }

    #[test]
    fn a_horizon_instance_takes_only_what_horizon_reads() {
        let (java, hz) = (instance(false), instance(true));
        for c in [
            Content::Mods,
            Content::Modpacks,
            Content::ResourcePacks,
            Content::Shaders,
            Content::DataPacks,
            Content::Worlds,
            Content::Plugins,
        ] {
            assert!(takes(&java, c), "{c:?} into Java Edition");
        }
        assert!(takes(&hz, Content::ResourcePacks) && takes(&hz, Content::Worlds));
        assert!(!takes(&hz, Content::Mods) && !takes(&hz, Content::Shaders) && !takes(&hz, Content::DataPacks));
        assert!(!takes(&hz, crate::modrinth::Kind::Mod.into()));
        assert!(takes(&hz, crate::modrinth::Kind::ResourcePack.into()));
        assert!(takes(&hz, crate::curseforge::api::Section::Worlds.into()));
        assert!(!takes(&hz, crate::curseforge::api::Section::ShaderPacks.into()));
    }

    #[test]
    fn a_horizon_instance_is_named_horizon_where_the_loader_would_be() {
        let (java, hz) = (instance(false), instance(true));
        assert_eq!(loader_kind(&java), Some("Fabric"));
        assert_eq!(loader_kind(&hz), Some("Horizon"));
        assert_eq!(loader_parts(&java, Some("v0.2.97")), Some(("Fabric", "0.16.5")));
        assert_eq!(loader_parts(&hz, Some("v0.2.97")), Some(("Horizon", "v0.2.97")));
        assert_eq!(loader_parts(&hz, None), Some(("Horizon", "")));
        assert_eq!(joined("Horizon", ""), "Horizon");
        assert_eq!(joined("Fabric", "0.16.5"), "Fabric 0.16.5");
        assert!(has_tab(&java, InstTab::Mods) && !has_tab(&hz, InstTab::Mods));
        assert!(has_tab(&hz, InstTab::ResourcePacks) && has_tab(&hz, InstTab::Logs));
    }

    #[test]
    fn the_menu_keeps_what_applies_to_horizon() {
        let f = || "A".to_string();
        for act in [Act::Play(f()), Act::Rename(f()), Act::Folder(f()), Act::Duplicate(f()), Act::Delete(f())] {
            assert!(horizon_allows(&act));
            assert_eq!(act_folder(&act), Some("A"));
        }
        for act in [Act::Mods(f()), Act::AddMods(f()), Act::ChangeLoader(f()), Act::Export(f()), Act::Repair(f())] {
            assert!(!horizon_allows(&act));
        }
        assert!(!horizon_allows(&Act::PlayOffline(f())) && !horizon_allows(&Act::FastStart(f(), true)));
        assert_eq!(act_folder(&Act::AddAccount), None);
    }
}
