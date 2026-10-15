//! The logic behind dropping files on the launcher, without the windows: how a set of dropped files is described,
//! which instances suit them (the instance picker), how an add is summed up for the toast, and the defaults the
//! New instance page uses for performance mods. The screens call these and draw the answers.

use super::dropped::{DroppedFile, DroppedKind};
use super::kind::ContentKind;
use super::loader_support;
use super::performance_pack;
use super::service::{self, AddFilesResult};
use crate::instances::Instance;

/// One instance in the picker, with what might not work there.
#[derive(Clone, Debug)]
pub struct InstanceOption {
    pub folder: String,
    pub name: String,
    pub problems: Vec<String>,
}

/// Why dropped files might not work in an instance: each file's own problem (once), plus no worlds for data packs.
pub fn problems(instance: &Instance, files: &[DroppedFile]) -> Vec<String> {
    let key = loader_support::key(instance.loader().map(|(kind, _)| kind));
    let mut problems: Vec<String> = Vec::new();
    let mut data_packs = false;
    for file in files {
        if let Some(problem) = file.problem_for(key, instance.mc_version()) {
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }
        data_packs |= file.kind == DroppedKind::DataPack;
    }
    if data_packs && service::worlds(&instance.root).is_empty() {
        problems.push("No worlds yet for the data packs".to_string());
    }
    problems
}

/// Instances with what might not work in each: the ones without problems first, then the most recently played.
pub fn options(instances: &[Instance], files: &[DroppedFile]) -> Vec<InstanceOption> {
    let mut list: Vec<(i64, InstanceOption)> = instances
        .iter()
        .map(|i| {
            (
                i.last_played(),
                InstanceOption { folder: i.folder.clone(), name: i.name().to_string(), problems: problems(i, files) },
            )
        })
        .collect();
    list.sort_by(|a, b| a.1.problems.is_empty().cmp(&b.1.problems.is_empty()).reverse().then(b.0.cmp(&a.0)));
    // `reverse` above puts the problem-free ones first: `true` (no problems) sorts after `false`, reversed
    list.into_iter().map(|(_, o)| o).collect()
}

/// The option to have selected when the picker opens: the first that has no problems, otherwise the first.
pub fn preselect(options: &[InstanceOption]) -> Option<usize> {
    options.iter().position(|o| o.problems.is_empty()).or(if options.is_empty() { None } else { Some(0) })
}

/// "1 mod", "3 resource packs", "2 shaders".
pub fn count_text(n: usize, kind: ContentKind) -> String {
    match kind {
        ContentKind::ResourcePacks => {
            if n == 1 {
                "1 resource pack".to_string()
            } else {
                format!("{n} resource packs")
            }
        }
        ContentKind::Shaders => {
            if n == 1 {
                "1 shader".to_string()
            } else {
                format!("{n} shaders")
            }
        }
        ContentKind::Mods => {
            if n == 1 {
                "1 mod".to_string()
            } else {
                format!("{n} mods")
            }
        }
    }
}

fn data_pack_text(n: usize) -> String {
    if n == 1 { "1 data pack".to_string() } else { format!("{n} data packs") }
}

fn world_text(n: usize) -> String {
    if n == 1 { "1 world".to_string() } else { format!("{n} worlds") }
}

/// "2 mods, 1 resource pack and 1 data pack" (or "nothing").
pub fn join(parts: &[String]) -> String {
    match parts.len() {
        0 => "nothing".to_string(),
        1 => parts[0].clone(),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

/// What a set of dropped files is, for headings.
pub fn describe(files: &[DroppedFile]) -> String {
    let count = |kind: DroppedKind| files.iter().filter(|f| f.kind == kind).count();
    let mut parts = Vec::new();
    if count(DroppedKind::Mod) > 0 {
        parts.push(count_text(count(DroppedKind::Mod), ContentKind::Mods));
    }
    if count(DroppedKind::ResourcePack) > 0 {
        parts.push(count_text(count(DroppedKind::ResourcePack), ContentKind::ResourcePacks));
    }
    if count(DroppedKind::DataPack) > 0 {
        parts.push(data_pack_text(count(DroppedKind::DataPack)));
    }
    if count(DroppedKind::ShaderPack) > 0 {
        parts.push(count_text(count(DroppedKind::ShaderPack), ContentKind::Shaders));
    }
    if count(DroppedKind::WorldSave) > 0 {
        parts.push(world_text(count(DroppedKind::WorldSave)));
    }
    join(&parts)
}

/// "2 mods and 1 shader": what an add did, for the toast.
pub fn summary(result: &AddFilesResult) -> String {
    let mut parts: Vec<String> = result.added.iter().map(|(kind, names)| count_text(names.len(), *kind)).collect();
    if !result.data_packs.is_empty() {
        parts.push(data_pack_text(result.data_packs.len()));
    }
    if !result.worlds.is_empty() {
        parts.push(world_text(result.worlds.len()));
    }
    join(&parts)
}

/// How dropped files are split up before anything is copied.
#[derive(Clone, Debug, Default)]
pub struct Sorted {
    /// become instances of their own
    pub modpacks: Vec<DroppedFile>,
    /// go into an instance
    pub content: Vec<DroppedFile>,
    /// names of what can't be added
    pub unknown: Vec<String>,
}

/// Sorts detected files into modpacks, content for an instance, and the rest.
pub fn sort(found: Vec<DroppedFile>) -> Sorted {
    let mut out = Sorted::default();
    for file in found {
        if file.kind.is_modpack() {
            out.modpacks.push(file);
        } else if file.kind.goes_into_instance() {
            out.content.push(file);
        } else {
            out.unknown.push(file.file_name.clone());
        }
    }
    out
}

/// The text of the dialog that says some files weren't added.
pub fn not_added_text(names: &[String]) -> String {
    format!(
        "Only mods, resource packs, shader packs, data packs, worlds and modpacks can be dropped here. Not added: {}",
        names.join(", ")
    )
}

/// What the New instance page says next to the performance mods option for a loader, or `None` when it isn't
/// offered for it.
pub fn performance_mods_offered(loader_key: Option<&str>) -> Option<String> {
    performance_pack::offered_for(loader_key).then(|| performance_pack::describe(loader_key))
}

/// The toast after adding the performance mods to a new instance, or `None` for nothing to say.
pub fn performance_toast(result: &service::InstallResult) -> Option<String> {
    if !result.installed.is_empty() {
        Some(format!("Added {}", result.installed.join(", ")))
    } else if !result.skipped.is_empty() {
        Some("No performance mods are available for this Minecraft version yet".to_string())
    } else {
        None
    }
}

/// The toast after adding the performance mods from the content manager.
pub fn performance_manager_toast(result: &service::InstallResult, minecraft_version: &str) -> String {
    if result.installed.is_empty() {
        if result.skipped.is_empty() {
            "Performance mods are already installed".to_string()
        } else {
            format!("No performance mods are available for Minecraft {minecraft_version} yet")
        }
    } else {
        format!("Added {}", result.installed.join(", "))
    }
}

/// Below this much memory (in MB) a computer counts as low end. 4 GB machines report a little less than 4096 MB,
/// and shared graphics memory takes some too.
pub const LOW_RAM_MB: u64 = 4608;
pub const LOW_CORES: usize = 2;

/// True when a computer with this much memory (0 when unknown) and this many processor cores counts as low end.
pub fn is_low_end_computer(total_ram_mb: u64, cores: usize) -> bool {
    (total_ram_mb > 0 && total_ram_mb <= LOW_RAM_MB) || (cores > 0 && cores <= LOW_CORES)
}

/// Whether "add performance mods" starts switched on: it does on slower computers, where it helps the most.
pub fn performance_mods_default_on() -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let ram_mb = system.total_memory() / (1024 * 1024);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    is_low_end_computer(ram_mb, cores)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::dropped::classify;

    fn file(name: &str, kind: DroppedKind) -> DroppedFile {
        DroppedFile {
            path: None,
            file_name: name.into(),
            kind,
            loader: None,
            name: None,
            version: None,
            minecraft_versions: vec![],
        }
    }

    #[test]
    fn describes_and_joins() {
        assert_eq!(join(&[]), "nothing");
        assert_eq!(join(&["1 mod".into()]), "1 mod");
        assert_eq!(join(&["2 mods".into(), "1 shader".into()]), "2 mods and 1 shader");
        assert_eq!(join(&["a".into(), "b".into(), "c".into()]), "a, b and c");
        let files = vec![
            file("a.jar", DroppedKind::Mod),
            file("b.jar", DroppedKind::Mod),
            file("p.zip", DroppedKind::ResourcePack),
            file("d.zip", DroppedKind::DataPack),
        ];
        assert_eq!(describe(&files), "2 mods, 1 resource pack and 1 data pack");
    }

    #[test]
    fn summarises_an_add() {
        let mut r = AddFilesResult::default();
        r.added.entry(ContentKind::Shaders).or_default().push("s.zip".into());
        r.added.entry(ContentKind::Mods).or_default().extend(["a.jar".to_string(), "b.jar".to_string()]);
        r.data_packs.push("d.zip (World)".into());
        assert_eq!(summary(&r), "2 mods, 1 shader and 1 data pack");
        assert_eq!(r.added_count(), 4);
    }

    #[test]
    fn sorts_dropped_files() {
        let s = sort(vec![
            file("a.jar", DroppedKind::Mod),
            file("m.mrpack", DroppedKind::ModrinthModpack),
            file("x.txt", DroppedKind::Unknown),
            file("w", DroppedKind::WorldSave),
        ]);
        assert_eq!(s.content.len(), 2);
        assert_eq!(s.modpacks.len(), 1);
        assert_eq!(s.unknown, vec!["x.txt".to_string()]);
    }

    #[test]
    fn picks_the_first_instance_without_problems() {
        let fine = InstanceOption { folder: "a".into(), name: "A".into(), problems: vec!["x".into()] };
        let good = InstanceOption { folder: "b".into(), name: "B".into(), problems: vec![] };
        assert_eq!(preselect(&[fine.clone(), good]), Some(1));
        assert_eq!(preselect(&[fine]), Some(0));
        assert_eq!(preselect(&[]), None);
    }

    #[test]
    fn low_end_computers() {
        assert!(is_low_end_computer(4096, 8));
        assert!(is_low_end_computer(16000, 2));
        assert!(!is_low_end_computer(16000, 8));
        assert!(!is_low_end_computer(0, 0));
        let _ = classify; // the detector is covered in dropped.rs
    }

    #[test]
    fn performance_toasts() {
        let none = service::InstallResult::default();
        assert_eq!(performance_toast(&none), None);
        assert_eq!(performance_manager_toast(&none, "1.21.1"), "Performance mods are already installed");
        let skipped = service::InstallResult { skipped: vec!["Sodium".into()], ..Default::default() };
        assert_eq!(
            performance_toast(&skipped).unwrap(),
            "No performance mods are available for this Minecraft version yet"
        );
        let added = service::InstallResult { installed: vec!["Sodium".into(), "Lithium".into()], ..Default::default() };
        assert_eq!(performance_toast(&added).unwrap(), "Added Sodium, Lithium");
        assert_eq!(performance_mods_offered(Some("fabric")).unwrap(), "Sodium, Lithium, FerriteCore, ModernFix");
        assert_eq!(performance_mods_offered(None), None);
    }
}
