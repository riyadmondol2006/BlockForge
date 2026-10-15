//! The questions and warnings that come up around starting the game and the launcher, as plain data a window can show.
//! The Java edition opens a dialog at each of these places; here each one is a [`Prompt`] with a title, a message and
//! the buttons, written in the language in use. The texts are the Java edition's own (so its translations fit), and
//! keep their `<br/>` and `<b>` markup in [`Prompt::html`]; [`Prompt::message`] is the same text as plain lines.

use std::fmt;
use std::path::Path;

use crate::config::Settings;
use crate::i18n::{self, Arg};
use crate::instances::Instance;
use crate::launch::javapick::JavaPick;
use crate::launch::memory;
use crate::launch::settings::LaunchSettings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// What the question is about, so the window can decide how to show it and what the answers do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptId {
    AlreadyRunning,
    InstanceCorrupt,
    NoAccount,
    WrongJavaVersion,
    InsufficientRam,
    InsufficientPermgen,
    MissingLibraries,
    LoginFailed,
    CommandFailed,
    ModsInfected,
    MemoryShort,
    JavaProblem,
    // asked when the launcher starts
    JavaOptionsVariable,
    InstalledInOneDrive,
    InstalledInProgramFiles,
    CannotWriteFiles,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptKind {
    /// Only tells the player something (an OK button, or a toast).
    Notice,
    /// Asks; the answer is the index of the button in [`Prompt::options`].
    Confirm,
}

#[derive(Clone, Debug)]
pub struct Prompt {
    pub id: PromptId,
    pub severity: Severity,
    pub kind: PromptKind,
    pub title: String,
    /// The text with the Java edition's markup (`<br/>`, `<b>`).
    pub html: String,
    /// The same text as lines.
    pub message: String,
    /// The buttons for a [`PromptKind::Confirm`], the first being "yes".
    pub options: Vec<String>,
    /// Stops whatever was being started (a notice about a problem) rather than only warning.
    pub blocking: bool,
    /// The setting to switch on when the player picks "Don't remind me again" (the option at index 2).
    pub dont_remind_setting: Option<&'static str>,
}

impl Prompt {
    fn new(id: PromptId, severity: Severity, kind: PromptKind, title: String, html: String) -> Prompt {
        Prompt {
            id,
            severity,
            kind,
            title,
            message: html_to_text(&html),
            html,
            options: if kind == PromptKind::Confirm {
                vec![i18n::tr("Yes"), i18n::tr("No")]
            } else {
                vec![i18n::tr("OK")]
            },
            blocking: false,
            dont_remind_setting: None,
        }
    }

    fn blocking(mut self) -> Prompt {
        self.blocking = true;
        self
    }

    fn with_option(mut self, text: String) -> Prompt {
        self.options.push(text);
        self
    }
}

/// An error that carries the prompt to show, for a launch that cannot go on. Find it with
/// `err.downcast_ref::<LaunchProblem>()`.
#[derive(Clone, Debug)]
pub struct LaunchProblem {
    pub prompt: Prompt,
}

impl fmt::Display for LaunchProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.prompt.title, self.prompt.message.replace('\n', " "))
    }
}

impl std::error::Error for LaunchProblem {}

/// The Java edition's dialog texts are small HTML; this turns one into lines of plain text.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            out.push(c);
            continue;
        }
        let mut tag = String::new();
        for t in chars.by_ref() {
            if t == '>' {
                break;
            }
            tag.push(t);
        }
        let name = tag.trim_start_matches('/').trim().to_ascii_lowercase();
        if name.starts_with("br") || name == "p" || name == "/p" {
            out.push('\n');
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace("&nbsp;", " ")
}

// ---- before a game starts -----------------------------------------------------------------------------------

pub fn already_running() -> Prompt {
    Prompt::new(
        PromptId::AlreadyRunning,
        Severity::Info,
        PromptKind::Notice,
        String::new(),
        i18n::tr("Minecraft is already running"),
    )
}

pub fn instance_corrupt() -> Prompt {
    Prompt::new(
        PromptId::InstanceCorrupt,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("Instance Corrupt"),
        i18n::tr("Cannot play instance as it's corrupted. Please reinstall, update or delete it."),
    )
    .blocking()
}

pub fn no_account() -> Prompt {
    Prompt::new(
        PromptId::NoAccount,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("No Account Selected"),
        i18n::tr("Cannot play instance as you have no account selected."),
    )
    .blocking()
}

pub fn wrong_java_version(needed: &str) -> Prompt {
    Prompt::new(
        PromptId::WrongJavaVersion,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("Cannot launch instance due to your Java version"),
        i18n::tr_args(
            "There was an issue launching this instance.<br/><br/>This version of the pack requires a Java version which you are not using.<br/><br/>Please install that version of Java and try again.<br/><br/>Java version needed: {0}",
            &[Arg::from(needed)],
        ),
    )
    .blocking()
}

pub fn insufficient_ram(required_mb: u32) -> Prompt {
    Prompt::new(
        PromptId::InsufficientRam,
        Severity::Error,
        PromptKind::Confirm,
        i18n::tr("Insufficient Ram"),
        i18n::tr_args(
            "This pack has set a minimum amount of ram needed to <b>{0}</b> MB.<br/><br/>Do you want to continue loading the instance anyway?",
            &[Arg::from(required_mb)],
        ),
    )
}

pub fn insufficient_permgen(required_mb: u32) -> Prompt {
    // the Java edition's text has this typo, and its translations are keyed on it
    Prompt::new(
        PromptId::InsufficientPermgen,
        Severity::Error,
        PromptKind::Confirm,
        i18n::tr("Insufficent Permgen"),
        i18n::tr_args(
            "This pack has set a minimum amount of permgen to <b>{0}</b> MB.<br/><br/>Do you want to continue loading the instance anyway?",
            &[Arg::from(required_mb)],
        ),
    )
}

pub fn missing_libraries() -> Prompt {
    Prompt::new(
        PromptId::MissingLibraries,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("Missing Libraries Found"),
        i18n::tr("This instance cannot be started due to missing libraries that cannot be downloaded.<br/><br/>Please reinstall the instance to create those libraries and be able to start this instance again."),
    )
    .blocking()
}

pub fn login_failed() -> Prompt {
    Prompt::new(
        PromptId::LoginFailed,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("Error Logging In"),
        i18n::tr("Couldn't login with Microsoft account"),
    )
    .blocking()
}

pub fn command_failed(detail: Option<&str>) -> Prompt {
    let mut text = i18n::tr("Error executing command");
    if let Some(d) = detail.filter(|d| !d.is_empty()) {
        text.push_str(":\n");
        text.push_str(d);
    }
    text.push('\n');
    text.push_str(&i18n::tr("Check the console for details"));
    Prompt::new(PromptId::CommandFailed, Severity::Error, PromptKind::Notice, i18n::tr("Error executing command"), text)
        .blocking()
}

/// Mods that look like the Fractureiser malware were found; the game is not started.
pub fn mods_infected(paths: &[std::path::PathBuf]) -> Prompt {
    let list = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
    Prompt::new(
        PromptId::ModsInfected,
        Severity::Error,
        PromptKind::Notice,
        i18n::tr("Scanning mods for Fractureiser"),
        format!("Infections have been found in your mods. See the below list of paths\n{list}"),
    )
    .blocking()
}

/// The little warning the Java edition pops up when the computer cannot give the game the memory it needs. The text is
/// English in the Java edition and stays so.
pub fn memory_short(warning: &str) -> Prompt {
    Prompt::new(PromptId::MemoryShort, Severity::Warning, PromptKind::Notice, String::new(), warning.to_string())
}

pub fn java_problem(text: &str) -> Prompt {
    Prompt::new(PromptId::JavaProblem, Severity::Warning, PromptKind::Notice, String::new(), format!("Java: {text}"))
}

/// Everything to check before starting this instance, in the order the Java edition asks. Blocking prompts mean the
/// game cannot start; a [`PromptKind::Confirm`] one needs an answer first (the first option, "yes", goes on).
pub fn preflight(
    settings: &Settings,
    instance: &Instance,
    ls: &LaunchSettings,
    java: &JavaPick,
    has_account: bool,
    game_running: bool,
) -> Vec<Prompt> {
    let mut out = Vec::new();
    if game_running {
        out.push(already_running().blocking());
        return out;
    }
    let launcher_value = serde_json::to_value(&instance.file.launcher).unwrap_or_default();
    if launcher_value.get("isPlayable").and_then(|v| v.as_bool()) == Some(false) {
        out.push(instance_corrupt());
        return out;
    }
    if !ls.ignore_java_on_instance_launch {
        if let (Some(req), Some(major)) = (ls.java_requirement, java.major) {
            if !req.conforms(major) {
                out.push(wrong_java_version(&req.version_string()));
                return out;
            }
        }
    }
    if !has_account {
        out.push(no_account());
        return out;
    }
    // automatic memory gives the pack what it needs whenever the computer can; only a value set by hand can be too low
    let l = &instance.file.launcher;
    let maximum = l.maximum_memory.unwrap_or(settings.maximum_memory);
    let required = l.required_memory.max(0) as u32;
    let (total, _) = crate::platform::memory_mb();
    if !memory::is_automatic(instance, settings)
        && maximum < required
        && required <= memory::safe_maximum_mb(total as i64, !memory::is_32_bit_java(java.home.as_deref()))
    {
        out.push(insufficient_ram(required));
    }
    let perm_gen = l.perm_gen.unwrap_or(settings.metaspace);
    if (perm_gen as i64) < l.required_perm_gen {
        out.push(insufficient_permgen(l.required_perm_gen as u32));
    }
    out
}

// ---- when the launcher starts ---------------------------------------------------------------------------------

/// The checks the Java edition makes as it starts: a `_JAVA_OPTIONS` variable that sets the memory, the data folder
/// inside OneDrive or Program Files, and a folder that cannot be written to. `ignore*Warning` settings that the player
/// switched on with "Don't remind me again" are respected; `dont_remind_setting` says which one to switch on.
pub fn startup_prompts(base_dir: &Path, settings: &Settings) -> Vec<Prompt> {
    let mut out = Vec::new();
    let flag = |key: &str| settings.extras.get(key).and_then(|v| v.as_bool()).unwrap_or(false);

    if let Ok(options) = std::env::var("_JAVA_OPTIONS") {
        let lower = options.to_lowercase();
        if (lower.contains("-xmx") || lower.contains("-xms") || lower.contains("-xss"))
            && !flag("ignoreJavaOptionsWarning")
        {
            let mut p = Prompt::new(
                PromptId::JavaOptionsVariable,
                Severity::Error,
                PromptKind::Confirm,
                "Warning".to_string(),
                "We've detected that you have a _JAVA_OPTIONS environment variable which may cause issues installing and playing Minecraft.<br/><br/>Do you want to fix this now so you don't have any issues in the future?".to_string(),
            )
            .with_option(i18n::tr("Don't remind me again"));
            p.dont_remind_setting = Some("ignoreJavaOptionsWarning");
            out.push(p);
        }
    }

    let path = base_dir.to_string_lossy();
    if path.contains("OneDrive") && !flag("ignoreOneDriveWarning") {
        let mut p = Prompt::new(
            PromptId::InstalledInOneDrive,
            Severity::Warning,
            PromptKind::Confirm,
            i18n::tr("BlockForge installed within OneDrive"),
            i18n::tr("We have detected that you're running BlockForge from within OneDrive.<br/><br/>This can cause serious issues and you should move the folder outside of OneDrive.<br/><br/>Do you want to close the launcher and do this now?"),
        )
        .with_option(i18n::tr("Don't remind me again"));
        p.dont_remind_setting = Some("ignoreOneDriveWarning");
        out.push(p);
    }
    if crate::platform::os() == crate::platform::Os::Windows
        && path.contains("Program Files")
        && !flag("ignoreProgramFilesWarning")
    {
        let mut p = Prompt::new(
            PromptId::InstalledInProgramFiles,
            Severity::Warning,
            PromptKind::Confirm,
            i18n::tr("BlockForge installed within Program Files"),
            i18n::tr("We have detected that you're running BlockForge from within Program Files.<br/><br/>This can cause serious issues and you should move the folder outside of Program Files.<br/><br/>Do you want to close the launcher and do this now?"),
        )
        .with_option(i18n::tr("Don't remind me again"));
        p.dont_remind_setting = Some("ignoreProgramFilesWarning");
        out.push(p);
    }

    if !can_write(base_dir) {
        out.push(
            Prompt::new(
                PromptId::CannotWriteFiles,
                Severity::Error,
                PromptKind::Notice,
                i18n::tr("BlockForge cannot write files"),
                i18n::tr("We have detected that BlockForge cannot write files in it's current location.<br/><br/>We cannot continue to run, you must move this folder somewhere else with write access.<br/><br/>Try moving to a folder in your Desktop or another drive.<br/><br/>You can also try running BlockForge as administrator, but this is not recommended."),
            )
            .blocking(),
        );
    }
    out
}

/// Whether a file can be made in the folder (the Java edition writes `.test` there).
pub fn can_write(dir: &Path) -> bool {
    let test = dir.join(".test");
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(&test).is_ok()
}

/// Turns on the `ignore...Warning` setting a "Don't remind me again" answer stands for. The caller saves the settings.
pub fn remember_dont_remind(settings: &mut Settings, prompt: &Prompt) {
    if let Some(key) = prompt.dont_remind_setting {
        settings.extras.insert(key.to_string(), serde_json::Value::Bool(true));
    }
}

// ---- the first time the launcher runs --------------------------------------------------------------------------

/// What the Java edition's setup dialog asks the first time: the language and whether to send anonymous analytics.
#[derive(Clone, Debug)]
pub struct FirstRun {
    pub languages: Vec<String>,
    pub selected_language: String,
    pub title: String,
    pub language_label: String,
    pub analytics_label: String,
    pub analytics_help: String,
    pub save_label: String,
}

/// The setup questions, if this is the first run (the `firstTimeRun` setting, on by default and switched off when the
/// setup is saved).
pub fn first_run(settings: &Settings) -> Option<FirstRun> {
    let first = settings.extras.get("firstTimeRun").and_then(|v| v.as_bool()).unwrap_or(true);
    if !first {
        return None;
    }
    Some(first_run_texts())
}

/// The setup texts in the language in use now (call again after the language changes).
pub fn first_run_texts() -> FirstRun {
    FirstRun {
        languages: i18n::languages(),
        selected_language: i18n::selected(),
        title: i18n::tr_args("Setting up {0}", &[Arg::from(crate::NAME)]),
        language_label: format!("{}: ", i18n::tr("Language")),
        analytics_label: format!("{}? ", i18n::tr("Enable Anonymous Analytics")),
        analytics_help: i18n::tr(
            "The Launcher sends back anonymous analytics to our own servers in a non identifying way in order to track what people do and don't use in the launcher. This helps determine what new features we implement in the future. All analytics are anonymous and contain no user/instance information in it at all. If you don't want to send anonymous analytics, you can disable this option.",
        ),
        save_label: i18n::tr("Save"),
    }
}

/// Saves the answers of the setup dialog into the settings (language, analytics, and that setup is done). The caller
/// saves the settings.
pub fn finish_first_run(settings: &mut Settings, language: &str, enable_analytics: bool) {
    i18n::choose_language(settings, language);
    settings.extras.insert("enableAnalytics".into(), serde_json::Value::Bool(enable_analytics));
    settings.extras.insert("firstTimeRun".into(), serde_json::Value::Bool(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::InstanceFile;

    fn english() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn inst(launcher: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher = serde_json::from_str(launcher).unwrap();
        Instance { folder: "x".into(), mod_count: 0, root: "/nonexistent".into(), file }
    }

    fn java(major: Option<u32>) -> JavaPick {
        JavaPick {
            home: Some("/jdk".into()),
            binary: "/jdk/bin/java".into(),
            major,
            arch: None,
            is_minecraft_runtime: false,
            component: None,
            runtime_missing: false,
            reasons: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn html_becomes_lines() {
        assert_eq!(html_to_text("a<br/><br/>b <b>1024</b> MB"), "a\n\nb 1024 MB");
        assert_eq!(html_to_text("x &lt;y&gt;"), "x <y>");
        assert_eq!(html_to_text("<font color=\"red\"><b>Soon</b></font>"), "Soon");
    }

    #[test]
    fn the_java_version_check_blocks_until_ignored() {
        let _g = english();
        let s = Settings::default();
        let i = inst(r#"{"name":"A","java":{"min":21,"max":0}}"#);
        let ls = LaunchSettings::resolve(&s, &i);
        let p = preflight(&s, &i, &ls, &java(Some(17)), true, false);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, PromptId::WrongJavaVersion);
        assert!(p[0].blocking);
        assert!(p[0].message.ends_with("Java version needed: Java 21 minimum"));
        assert_eq!(p[0].title, "Cannot launch instance due to your Java version");
        // the right Java, an unknown Java, and the "ignore" setting all let it through
        assert!(preflight(&s, &i, &ls, &java(Some(21)), true, false).is_empty());
        assert!(preflight(&s, &i, &ls, &java(None), true, false).is_empty());
        let mut ignore = Settings::default();
        ignore.extras.insert("ignoreJavaOnInstanceLaunch".into(), true.into());
        let ls2 = LaunchSettings::resolve(&ignore, &i);
        assert!(preflight(&ignore, &i, &ls2, &java(Some(17)), true, false).is_empty());
    }

    #[test]
    fn no_account_corrupt_and_running() {
        let s = Settings::default();
        let i = inst(r#"{"name":"A"}"#);
        let ls = LaunchSettings::resolve(&s, &i);
        assert_eq!(preflight(&s, &i, &ls, &java(Some(21)), false, false)[0].id, PromptId::NoAccount);
        assert_eq!(preflight(&s, &i, &ls, &java(Some(21)), true, true)[0].id, PromptId::AlreadyRunning);
        // an instance made by the Rust edition has no isPlayable at all, and is fine
        assert!(preflight(&s, &i, &ls, &java(Some(21)), true, false).is_empty());
        let broken = inst(r#"{"name":"A","isPlayable":false}"#);
        let bl = LaunchSettings::resolve(&s, &broken);
        assert_eq!(preflight(&s, &broken, &bl, &java(Some(21)), true, false)[0].id, PromptId::InstanceCorrupt);
        let fine = inst(r#"{"name":"A","isPlayable":true}"#);
        assert!(preflight(&s, &fine, &LaunchSettings::resolve(&s, &fine), &java(Some(21)), true, false).is_empty());
    }

    #[test]
    fn a_pack_that_needs_more_than_is_set_asks() {
        let _g = english();
        let mut s = Settings::default();
        s.automatic_memory = Some(false);
        s.maximum_memory = 2048;
        let i = inst(r#"{"name":"A","requiredMemory":4096,"requiredPermGen":512}"#);
        let ls = LaunchSettings::resolve(&s, &i);
        let p = preflight(&s, &i, &ls, &java(Some(21)), true, false);
        let ids: Vec<_> = p.iter().map(|p| p.id).collect();
        // the memory question only comes up when this computer could give that much
        let (total, _) = crate::platform::memory_mb();
        if total >= 8192 {
            assert!(ids.contains(&PromptId::InsufficientRam));
        }
        assert!(ids.contains(&PromptId::InsufficientPermgen));
        let perm = p.iter().find(|p| p.id == PromptId::InsufficientPermgen).unwrap();
        assert_eq!(perm.kind, PromptKind::Confirm);
        assert_eq!(perm.options, vec!["Yes", "No"]);
        assert_eq!(perm.title, "Insufficent Permgen");
        assert!(perm.message.starts_with("This pack has set a minimum amount of permgen to 512 MB."));
        // automatic memory never asks
        let auto = Settings::default();
        let ids: Vec<_> = preflight(&auto, &i, &LaunchSettings::resolve(&auto, &i), &java(Some(21)), true, false)
            .iter()
            .map(|p| p.id)
            .collect();
        assert!(!ids.contains(&PromptId::InsufficientRam));
    }

    #[test]
    fn startup_checks_respect_dont_remind() {
        let _g = english();
        let dir = std::env::temp_dir().join(format!("bf-startup-{}", std::process::id())).join("OneDrive");
        let mut s = Settings::default();
        let found = startup_prompts(&dir, &s);
        let one = found.iter().find(|p| p.id == PromptId::InstalledInOneDrive).unwrap();
        assert_eq!(one.options, vec!["Yes", "No", "Don't remind me again"]);
        remember_dont_remind(&mut s, one);
        assert!(startup_prompts(&dir, &s).iter().all(|p| p.id != PromptId::InstalledInOneDrive));
        assert!(can_write(&dir));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn first_run_asks_once() {
        let _g = english();
        let mut s = Settings::default();
        let q = first_run(&s).unwrap();
        assert_eq!(q.title, "Setting up BlockForge");
        assert_eq!(q.language_label, "Language: ");
        assert_eq!(q.analytics_label, "Enable Anonymous Analytics? ");
        finish_first_run(&mut s, "English", false);
        assert!(first_run(&s).is_none());
        assert_eq!(s.extras["enableAnalytics"], false);
    }

    #[test]
    fn command_failures_read_like_the_java_edition() {
        let _g = english();
        let p = command_failed(Some("exit code 1"));
        assert_eq!(p.message, "Error executing command:\nexit code 1\nCheck the console for details");
        let none = command_failed(None);
        assert_eq!(none.message, "Error executing command\nCheck the console for details");
    }
}
