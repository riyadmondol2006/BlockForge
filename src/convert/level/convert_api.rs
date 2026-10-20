// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The public types of a conversion: request, summary, error and the progress sink. Types only; the entry point
//! (`convert_world`) is written in a later phase.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use super::encoding::TargetVersion;
use super::settings::ConversionSettings;

/// What to convert and how.
#[derive(Clone, Debug)]
pub struct ConvertRequest {
    /// The input world folder (or the unpacked contents of a `.mcworld`).
    pub input: PathBuf,
    /// The output world folder (created if missing).
    pub output: PathBuf,
    /// The edition and version to write.
    pub target: TargetVersion,
    pub settings: ConversionSettings,
}

impl ConvertRequest {
    pub fn new(input: impl Into<PathBuf>, output: impl Into<PathBuf>, target: TargetVersion) -> ConvertRequest {
        ConvertRequest { input: input.into(), output: output.into(), target, settings: ConversionSettings::default() }
    }
}

/// The outcome of a finished conversion.
#[derive(Clone, Debug, Default)]
pub struct ConvertSummary {
    /// The number of columns (chunks) written.
    pub columns: u64,
    /// Identifiers that had no mapping, with how often each was met (sorted by identifier).
    pub blocks_unmapped: Vec<(String, u64)>,
    /// Non-fatal problems (Chunker's `logNonFatalException`) and notes.
    pub warnings: Vec<String>,
    pub elapsed: Duration,
}

/// Why a conversion failed.
#[derive(Debug)]
pub enum ConvertError {
    /// The input or the requested output is not supported (format, version, combination).
    Unsupported(String),
    /// Reading or writing a file failed.
    Io(std::io::Error),
    /// The input data is damaged or not what its format says.
    Malformed(String),
    /// The conversion was cancelled.
    Cancelled,
    /// A bug: an invariant broke or a task panicked.
    Internal(String),
}

impl ConvertError {
    /// True for [`ConvertError::Cancelled`].
    pub fn is_cancelled(&self) -> bool {
        matches!(self, ConvertError::Cancelled)
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            ConvertError::Io(error) => write!(f, "I/O error: {error}"),
            ConvertError::Malformed(msg) => write!(f, "malformed world data: {msg}"),
            ConvertError::Cancelled => f.write_str("The process was cancelled"),
            ConvertError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for ConvertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConvertError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConvertError {
    fn from(error: std::io::Error) -> Self {
        ConvertError::Io(error)
    }
}

/// Receives progress while a conversion runs. All methods have empty defaults; calls come from worker threads.
pub trait ProgressSink: Send + Sync {
    /// Overall progress in `0.0..=1.0`.
    fn progress(&self, _fraction: f64) {}
    /// What the conversion is doing now ("Reading Level", "Writing World").
    fn stage(&self, _text: &str) {}
    /// A named state change, for example `("Compacting output", true)`.
    fn signal(&self, _name: &str, _active: bool) {}
    /// A non-fatal problem.
    fn warning(&self, _message: &str) {}
}

/// A sink that ignores everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;

impl ProgressSink for NullSink {}

/// A closure taking the fraction is a sink.
impl<F: Fn(f64) + Send + Sync> ProgressSink for F {
    fn progress(&self, fraction: f64) {
        self(fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::level::encoding::{EncodingType, VersionTriple};
    use std::sync::Mutex;

    #[test]
    fn error_display() {
        assert_eq!(ConvertError::Cancelled.to_string(), "The process was cancelled");
        assert!(ConvertError::Cancelled.is_cancelled());
        let io: ConvertError = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert!(io.to_string().contains("gone"));
        assert!(std::error::Error::source(&io).is_some());
        assert!(ConvertError::Malformed("x".into()).to_string().contains("malformed"));
        assert!(ConvertError::Unsupported("y".into()).to_string().starts_with("unsupported"));
    }

    #[test]
    fn closure_sink() {
        let seen = Mutex::new(Vec::new());
        let sink = |f: f64| seen.lock().unwrap().push(f);
        let sink: &dyn ProgressSink = &sink;
        sink.progress(0.5);
        sink.stage("ignored");
        NullSink.signal("x", true);
        assert_eq!(*seen.lock().unwrap(), [0.5]);
    }

    #[test]
    fn request_defaults() {
        let request = ConvertRequest::new("in", "out", TargetVersion::new(EncodingType::Java, VersionTriple(1, 21, 0)));
        assert!(request.settings.process_maps);
        assert_eq!(request.output, PathBuf::from("out"));
        assert_eq!(ConvertSummary::default().columns, 0);
    }
}
