//! The one place that decides whether anything may be sent to a server about how BlockForge is used. Everything that
//! reports usage asks here first, and the answer is "no" unless the player turned it on.

use serde_json::Value;

use crate::config::Settings;

/// The settings and start-up flags that decide what may leave the computer.
#[derive(Clone, Debug)]
pub struct Privacy {
    /// `enableAnalytics`: anonymous usage statistics.
    pub enable_analytics: bool,
    /// `enableLogs`: counts of installs and time played for the packs that have a public page.
    pub enable_logs: bool,
    /// `enableConsole`: shows the game's console (not a privacy matter, kept with the others the Java edition tests).
    pub enable_console: bool,
    /// `--disable-analytics` on the command line.
    pub disable_analytics_flag: bool,
    /// Error reporting stays off unless `--enable-error-reporting` is given, so this is on by default.
    pub disable_error_reporting: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Privacy {
            enable_analytics: false,
            enable_logs: false,
            enable_console: false,
            disable_analytics_flag: false,
            disable_error_reporting: true,
        }
    }
}

impl Privacy {
    pub fn from_settings(settings: &Settings) -> Privacy {
        let flag = |k: &str| match k {
            "enableAnalytics" => settings.enable_analytics,
            "enableLogs" => settings.enable_logs,
            "enableConsole" => settings.enable_console,
            _ => settings.extras.get(k).and_then(Value::as_bool).unwrap_or(false),
        };
        Privacy {
            enable_analytics: flag("enableAnalytics"),
            enable_logs: flag("enableLogs"),
            enable_console: flag("enableConsole"),
            ..Default::default()
        }
    }

    /// Anonymous usage statistics (which screens are used, which instances are launched).
    pub fn analytics_allowed(&self) -> bool {
        self.enable_analytics && !self.disable_analytics_flag
    }

    /// Counts of installs and time played for the packs that have a public page.
    pub fn usage_logs_allowed(&self) -> bool {
        self.enable_logs
    }

    /// Crash reports to a third party service.
    pub fn error_reporting_allowed(&self) -> bool {
        !self.disable_error_reporting
    }
}

/// Usage events waiting to be sent. Nothing is even collected unless the player agreed, and what was collected
/// is thrown away, not sent, if they change their mind.
#[derive(Debug, Default)]
pub struct EventQueue<E> {
    pub privacy: Privacy,
    pending: Vec<E>,
}

impl<E> EventQueue<E> {
    pub fn new(privacy: Privacy) -> EventQueue<E> {
        EventQueue { privacy, pending: Vec::new() }
    }

    pub fn track(&mut self, event: E) {
        if self.privacy.analytics_allowed() {
            self.pending.push(event);
        }
    }

    /// Events waiting to be sent.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Hands every waiting event to `send`, or throws them away when statistics are off.
    pub fn send_all(&mut self, send: impl FnOnce(Vec<E>)) {
        let events = std::mem::take(&mut self.pending);
        if self.privacy.analytics_allowed() {
            send(events);
        }
    }
}

/// Whether error reporting has been started. It can only start when the gate allows it, so a remote setting can't
/// switch it on.
#[derive(Debug, Default)]
pub struct ErrorReporting {
    pub initialised: bool,
}

impl ErrorReporting {
    pub fn enable(&mut self, privacy: &Privacy) {
        if !privacy.error_reporting_allowed() {
            return;
        }
        self.initialised = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_off_by_default() {
        let s = Settings::default();
        let p = Privacy::from_settings(&s);
        assert!(!p.enable_analytics && !p.enable_logs && !p.enable_console);
        assert!(!p.analytics_allowed());
        assert!(!p.usage_logs_allowed());
        assert!(!p.error_reporting_allowed());
    }

    #[test]
    fn usage_events_are_not_even_collected_while_off() {
        let mut q = EventQueue::new(Privacy::default());
        q.track("something_happened");
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn turning_statistics_on_collects_and_turning_them_off_discards() {
        let mut q = EventQueue::new(Privacy { enable_analytics: true, ..Default::default() });
        q.track("something_happened");
        assert_eq!(q.pending_count(), 1);

        // changed their mind before anything was sent: what was collected is thrown away, not sent
        q.privacy.enable_analytics = false;
        let mut sent = false;
        q.send_all(|_| sent = true);
        assert!(!sent);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn statistics_are_sent_only_when_allowed() {
        let mut q = EventQueue::new(Privacy { enable_analytics: true, ..Default::default() });
        q.track(1);
        q.track(2);
        let mut got = Vec::new();
        q.send_all(|events| got = events);
        assert_eq!(got, vec![1, 2]);
        // the command line switch wins over the setting
        q.privacy.disable_analytics_flag = true;
        q.track(3);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn settings_from_a_java_file_are_read() {
        let s: Settings = serde_json::from_str(r#"{"enableAnalytics":true,"enableLogs":true}"#).unwrap();
        let p = Privacy::from_settings(&s);
        assert!(p.analytics_allowed() && p.usage_logs_allowed());
    }

    #[test]
    fn error_reporting_cannot_be_switched_on_by_the_server() {
        // the launcher's remote config asks for it to start, which must be ignored while it is off
        let mut reporting = ErrorReporting::default();
        reporting.enable(&Privacy::default());
        assert!(!reporting.initialised);
    }

    #[test]
    fn an_explicit_flag_turns_error_reporting_on() {
        let p = Privacy { disable_error_reporting: false, ..Default::default() };
        assert!(p.error_reporting_allowed());
        let mut reporting = ErrorReporting::default();
        reporting.enable(&p);
        assert!(reporting.initialised);
    }
}
