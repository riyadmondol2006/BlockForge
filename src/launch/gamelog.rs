//! Reading what the game prints: spotting the well known crashes so the player can be told what to do, and hiding the
//! account's details before a line is shown or saved. The Java edition does both in the loop that reads the game's
//! output.

use crate::i18n::{self, Arg};
use crate::launch::legacy::match_version;
use crate::launch::prompts::{Prompt, PromptId, PromptKind, Severity};

/// A crash the launcher knows the cure for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    OutOfMemory,
    /// Forge on Minecraft 1.6 and older with a newer Java.
    ConcurrentModification16,
    /// A game that wants Java 8 started on a newer one.
    NewerJavaThan8,
    NeedJava16OrNewer,
    NeedJava17OrNewer,
}

/// What a line of the game's output says about how it is going.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct LineFindings {
    pub error: Option<GameError>,
    /// The game wrote a crash report.
    pub crashed: bool,
}

/// Looks at one line of output (`minecraft_version` is the instance's, for the Forge 1.6 case).
pub fn inspect_line(line: &str, minecraft_version: &str) -> LineFindings {
    let mut f = LineFindings::default();
    if line.contains("java.lang.OutOfMemoryError")
        || line.contains("There is insufficient memory for the Java Runtime Environment")
    {
        f.error = Some(GameError::OutOfMemory);
    }
    if line.contains("java.util.ConcurrentModificationException") && match_version(minecraft_version, "1.6", true, true)
    {
        f.error = Some(GameError::ConcurrentModification16);
    }
    if line.contains("has been compiled by a more recent version of the Java Runtime (class file version 60.0)") {
        f.error = Some(GameError::NeedJava16OrNewer);
    }
    if line.contains("has been compiled by a more recent version of the Java Runtime (class file version 61.0)") {
        f.error = Some(GameError::NeedJava17OrNewer);
    }
    if line.contains("class jdk.internal.loader.ClassLoaders$AppClassLoader cannot be cast to class") {
        f.error = Some(GameError::NewerJavaThan8);
    }
    if line.contains("Crash report saved to") || line.contains("Minecraft Crash Report") {
        f.crashed = true;
    }
    f
}

/// Watches a game's output for the well known crashes; the last one seen is the cause to explain if the game then
/// closes with an error.
#[derive(Clone, Debug, Default)]
pub struct CrashWatch {
    minecraft_version: String,
    pub cause: Option<GameError>,
    pub crashed: bool,
}

impl CrashWatch {
    pub fn new(minecraft_version: &str) -> CrashWatch {
        CrashWatch { minecraft_version: minecraft_version.to_string(), ..Default::default() }
    }

    pub fn line(&mut self, line: &str) {
        let f = inspect_line(line, &self.minecraft_version);
        if f.error.is_some() {
            self.cause = f.error;
        }
        self.crashed |= f.crashed;
    }

    /// What to tell the player after the game closed with `code`: nothing after a clean exit or an unknown crash.
    pub fn advice(&self, code: Option<i32>) -> Option<Prompt> {
        let failed = code.is_some_and(|c| c != 0) || self.crashed;
        self.cause.filter(|_| failed).map(explain)
    }
}

/// The "About Your Crash" explanation for a known crash.
pub fn explain(error: GameError) -> Prompt {
    let title = i18n::tr("About Your Crash");
    let (html, options) = match error {
        GameError::OutOfMemory => (
            i18n::tr(
                "Minecraft has crashed due to insufficent memory being allocated.<br/><br/>Please go to the settings tab and increase the maximum memory option and then try launching the instance again.",
            ),
            vec![i18n::tr("OK")],
        ),
        GameError::ConcurrentModification16 => (
            i18n::tr(
                "Minecraft has crashed due to an incompatability with Forge and your version of Java.<br/><br/>Please reinstall the instance to automatically fix the problem, and then try launching the instance again.",
            ),
            vec![i18n::tr("OK")],
        ),
        GameError::NewerJavaThan8 => (
            i18n::tr(
                "Minecraft has crashed due to not being compatible with your Java version.<br/><br/>Most modded Minecraft is only compatible with Java 8, so you must install Java 8 on your computer.",
            ),
            vec![i18n::tr("OK"), i18n::tr("Download Java 8")],
        ),
        GameError::NeedJava16OrNewer | GameError::NeedJava17OrNewer => {
            let version = if error == GameError::NeedJava16OrNewer { "16" } else { "17" };
            (
                i18n::tr_args(
                    "Minecraft has crashed due to not being compatible with your Java version.<br/><br/>This version of Minecraft requires Java {0} or newer.<br/><br/>Make sure you've selected the correct Java version in this instances<br/>settings or not disabled the \"Use Java Provided By Minecraft\" setting.<br/><br/>If it's still not working, you may be using a mod that requires Java {0},<br/>but Minecraft only requires Java 8, so you can go into the instances<br/>settings and in the \"Java\" tab, change the \"Runtime Overide\"<br/>option to one that is Java {0} or newer.",
                    &[Arg::from(version)],
                ),
                vec![i18n::tr("OK")],
            )
        }
    };
    Prompt {
        id: PromptId::JavaProblem,
        severity: Severity::Info,
        kind: if options.len() > 1 { PromptKind::Confirm } else { PromptKind::Notice },
        title,
        message: crate::launch::prompts::html_to_text(&html),
        html,
        options,
        blocking: false,
        dont_remind_setting: None,
    }
}

/// Where the "Download Java 8" button goes.
pub const JAVA_8_DOWNLOAD_URL: &str = "https://atl.pw/java8download";

/// Hides the player's details in the game's output before it is shown or saved.
#[derive(Clone, Debug, Default)]
pub struct LineCensor {
    names: Vec<String>,
    ids: Vec<String>,
    token: Option<String>,
    hide_personal: bool,
}

impl LineCensor {
    /// `hide_personal` is the opposite of the debug log setting: with debug on, names, ids and addresses stay visible
    /// (the access token never does).
    pub fn new(names: &[&str], uuid: &str, access_token: Option<&str>, hide_personal: bool) -> LineCensor {
        let mut ids = vec![uuid.to_string(), uuid.replace('-', "")];
        ids.dedup();
        LineCensor {
            names: names.iter().filter(|n| !n.is_empty()).map(|n| n.to_string()).collect(),
            ids: ids.into_iter().filter(|i| !i.is_empty()).collect(),
            token: access_token.filter(|t| t.len() > 3).map(str::to_string),
            hide_personal,
        }
    }

    pub fn apply(&self, line: &str) -> String {
        let mut out = line.to_string();
        if self.hide_personal {
            for n in &self.names {
                out = out.replace(n.as_str(), "**MINECRAFTUSERNAME**");
            }
            for id in &self.ids {
                out = out.replace(id.as_str(), "**UUID**");
            }
            out = hide_ip_addresses(&out);
        }
        if let Some(t) = &self.token {
            out = out.replace(t.as_str(), "**ACCESSTOKEN**");
        }
        out
    }
}

/// Replaces `1.2.3.4` style addresses (each part of one to three digits) with `**IPADDRESS**`.
pub fn hide_ip_addresses(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < b.len() {
        // try to read d{1,3}(.d{1,3}){3} starting here, at a word boundary
        if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) {
            let mut j = i;
            let mut parts = 0;
            loop {
                let start = j;
                while j < b.len() && b[j].is_ascii_digit() && j - start < 3 {
                    j += 1;
                }
                if j == start {
                    break;
                }
                parts += 1;
                if parts == 4 {
                    break;
                }
                if j < b.len() && b[j] == b'.' {
                    j += 1;
                } else {
                    break;
                }
            }
            let ends_cleanly = j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_');
            if parts == 4 && ends_cleanly {
                out.push_str("**IPADDRESS**");
                i = j;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn known_crashes_are_spotted() {
        let f = |l: &str| inspect_line(l, "1.20.1");
        assert_eq!(
            f("Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space").error,
            Some(GameError::OutOfMemory)
        );
        assert_eq!(
            f("There is insufficient memory for the Java Runtime Environment to continue.").error,
            Some(GameError::OutOfMemory)
        );
        assert_eq!(f("X has been compiled by a more recent version of the Java Runtime (class file version 61.0), this version").error, Some(GameError::NeedJava17OrNewer));
        assert_eq!(
            f("has been compiled by a more recent version of the Java Runtime (class file version 60.0)").error,
            Some(GameError::NeedJava16OrNewer)
        );
        assert_eq!(
            f("class jdk.internal.loader.ClassLoaders$AppClassLoader cannot be cast to class java.net.URLClassLoader")
                .error,
            Some(GameError::NewerJavaThan8)
        );
        assert!(f("#@!@# Game crashed! Crash report saved to: #@!@# /x/crash.txt").crashed);
        assert!(f("---- Minecraft Crash Report ----").crashed);
        assert_eq!(f("[12:00:00] [main/INFO]: Setting user: Steve"), LineFindings::default());
        // the Forge problem only affects old versions
        let cme = "java.util.ConcurrentModificationException";
        assert_eq!(inspect_line(cme, "1.5.2").error, Some(GameError::ConcurrentModification16));
        assert_eq!(inspect_line(cme, "1.12.2").error, None);
    }

    #[test]
    fn crash_explanations_read_like_the_java_edition() {
        let _g = english();
        let oom = explain(GameError::OutOfMemory);
        assert_eq!(oom.title, "About Your Crash");
        assert!(oom.message.starts_with(
            "Minecraft has crashed due to insufficent memory being allocated.\n\nPlease go to the settings tab"
        ));
        let j17 = explain(GameError::NeedJava17OrNewer);
        assert!(j17.message.contains("requires Java 17 or newer"));
        assert!(j17.message.contains("Java 17,\nbut Minecraft only requires Java 8"));
        let j8 = explain(GameError::NewerJavaThan8);
        assert_eq!(j8.options, vec!["OK", "Download Java 8"]);
    }

    #[test]
    fn account_details_are_hidden() {
        let c = LineCensor::new(
            &["Steve", "steve@example.com"],
            "0123456789abcdef0123456789abcdef",
            Some("tok_secret_1"),
            true,
        );
        assert_eq!(c.apply("Setting user: Steve"), "Setting user: **MINECRAFTUSERNAME**");
        assert_eq!(c.apply("uuid 0123456789abcdef0123456789abcdef"), "uuid **UUID**");
        assert_eq!(
            c.apply("connecting to 192.168.1.20:25565 with tok_secret_1"),
            "connecting to **IPADDRESS**:25565 with **ACCESSTOKEN**"
        );
        // with debug logging the names and addresses stay, the token never does
        let debug = LineCensor::new(&["Steve"], "u", Some("tok_secret_1"), false);
        assert_eq!(debug.apply("Steve at 10.0.0.1 tok_secret_1"), "Steve at 10.0.0.1 **ACCESSTOKEN**");
    }

    #[test]
    fn ip_addresses_need_four_parts() {
        assert_eq!(hide_ip_addresses("1.2.3.4"), "**IPADDRESS**");
        assert_eq!(hide_ip_addresses("a 255.255.255.255 b"), "a **IPADDRESS** b");
        assert_eq!(hide_ip_addresses("version 1.20.1"), "version 1.20.1");
        // the first four parts are an address as far as the Java edition's pattern is concerned
        assert_eq!(hide_ip_addresses("1.2.3.4.5"), "**IPADDRESS**.5");
        assert_eq!(hide_ip_addresses("v1.2.3.4"), "v1.2.3.4");
        assert_eq!(hide_ip_addresses("héllo 8.8.8.8"), "héllo **IPADDRESS**");
        assert_eq!(hide_ip_addresses("1234.5.6.7"), "1234.5.6.7");
    }
}

#[cfg(test)]
mod crash_watch_tests {
    use super::*;

    #[test]
    fn a_known_crash_is_explained_only_when_the_game_failed() {
        let mut w = CrashWatch::new("1.20.1");
        w.line("[main/INFO]: Loading");
        w.line("Exception in thread \"Render thread\" java.lang.OutOfMemoryError: Java heap space");
        assert_eq!(w.cause, Some(GameError::OutOfMemory));
        let advice = w.advice(Some(1)).expect("explained");
        assert!(advice.message.to_lowercase().contains("memory"), "{}", advice.message);
        // a game that recovered and closed normally needs no explanation
        assert!(w.advice(Some(0)).is_none());
        // a crash report counts as failing even with a 0 exit code
        w.line("#@!@# Game crashed! Crash report saved to: #@!@# crash-reports/crash.txt");
        assert!(w.advice(Some(0)).is_some());
    }

    #[test]
    fn the_wrong_java_is_recognised() {
        let mut w = CrashWatch::new("1.18.2");
        w.line("java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime (class file version 61.0)");
        assert_eq!(w.cause, Some(GameError::NeedJava17OrNewer));
        assert!(w.advice(Some(1)).unwrap().message.contains("17"));
        assert!(CrashWatch::new("1.20.1").advice(Some(1)).is_none(), "an unknown crash has nothing to say");
    }
}
