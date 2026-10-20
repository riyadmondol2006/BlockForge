// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The editions (`EncodingType`), a minimal version triple and the conversion target.
//!
//! `VersionTriple` is a local stand-in for Chunker's `Version`; swap it for `crate::convert::util::Version` when that
//! lands (same shape: major, minor, patch, `Ord`, `from_string`, `Display`).

use std::fmt;

/// The world formats Chunker converts between (`EncodingType.JAVA` and `BEDROCK`; the internal Preview and Settings
/// types are out of scope).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EncodingType {
    Java,
    Bedrock,
}

impl EncodingType {
    /// Every readable/writable edition, in Chunker's registration order (Java first).
    pub const ALL: [EncodingType; 2] = [EncodingType::Java, EncodingType::Bedrock];

    /// The short name (`getName`).
    pub fn name(self) -> &'static str {
        match self {
            EncodingType::Java => "Java",
            EncodingType::Bedrock => "Bedrock",
        }
    }

    /// The display label (`getLabel`).
    pub fn label(self) -> &'static str {
        match self {
            EncodingType::Java => "Java Edition",
            EncodingType::Bedrock => "Bedrock Edition",
        }
    }

    /// The sort order (`getOrder`).
    pub fn order(self) -> u32 {
        match self {
            EncodingType::Java => 20,
            EncodingType::Bedrock => 10,
        }
    }

    /// Looks an edition up by name, case-insensitively.
    pub fn from_name(name: &str) -> Option<EncodingType> {
        EncodingType::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(name))
    }

    /// The opposite edition.
    pub fn other(self) -> EncodingType {
        match self {
            EncodingType::Java => EncodingType::Bedrock,
            EncodingType::Bedrock => EncodingType::Java,
        }
    }
}

impl fmt::Display for EncodingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A `major.minor.patch` version (`Version`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionTriple(pub u32, pub u32, pub u32);

impl VersionTriple {
    /// Sorts after every real version (`Version.LATEST`).
    pub const LATEST: VersionTriple = VersionTriple(u32::MAX, u32::MAX, u32::MAX);

    pub fn major(self) -> u32 {
        self.0
    }
    pub fn minor(self) -> u32 {
        self.1
    }
    pub fn patch(self) -> u32 {
        self.2
    }

    /// Parses `1`, `1.20` or `1.20.5`; missing parts are 0 (`Version.fromString`).
    pub fn from_string(input: &str) -> Result<VersionTriple, String> {
        let mut parts = [0u32; 3];
        for (i, part) in input.split('.').enumerate() {
            if i >= 3 {
                break;
            }
            parts[i] = part.trim().parse().map_err(|_| format!("invalid version {input:?}"))?;
        }
        Ok(VersionTriple(parts[0], parts[1], parts[2]))
    }
}

impl fmt::Display for VersionTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// An edition and a version: what to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetVersion {
    pub edition: EncodingType,
    /// `None` = the latest version the writer supports.
    pub version: Option<VersionTriple>,
}

impl TargetVersion {
    pub fn new(edition: EncodingType, version: VersionTriple) -> TargetVersion {
        TargetVersion { edition, version: Some(version) }
    }

    /// The newest version of an edition.
    pub fn latest(edition: EncodingType) -> TargetVersion {
        TargetVersion { edition, version: None }
    }

    /// Parses Chunker's writer id: `TYPE[_VERSION]` such as `JAVA_1_20_5` or `BEDROCK_R20_80` (Bedrock `R20` means
    /// `1.20`; underscores are dots; the type is case-insensitive).
    pub fn parse(id: &str) -> Result<TargetVersion, String> {
        let (kind, version) = match id.split_once('_') {
            Some((kind, rest)) => (kind, Some(rest)),
            None => (id, None),
        };
        let edition = EncodingType::from_name(kind).ok_or_else(|| format!("unknown edition {kind:?}"))?;
        let version = match version {
            None => None,
            Some(text) => {
                let text = match text.strip_prefix('R') {
                    Some(rest) => format!("1_{rest}"),
                    None => text.to_owned(),
                };
                Some(VersionTriple::from_string(&text.replace('_', "."))?)
            }
        };
        Ok(TargetVersion { edition, version })
    }

    /// The writer id (`Messenger.toEncodedString`): trailing `.0` dropped, Bedrock as `R<minor>_<patch>`.
    pub fn encoded(&self) -> String {
        let name = self.edition.name().to_uppercase();
        let Some(v) = self.version else { return name };
        let mut text = v.to_string();
        if let Some(stripped) = text.strip_suffix(".0") {
            text = stripped.to_owned();
        }
        let mut text = text.replace('.', "_");
        if self.edition == EncodingType::Bedrock {
            if let Some(rest) = text.strip_prefix("1_") {
                text = format!("R{rest}");
            }
        }
        format!("{name}_{text}")
    }
}

impl fmt::Display for TargetVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            Some(v) => write!(f, "{} {}", self.edition.label(), v),
            None => write!(f, "{} (latest)", self.edition.label()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_and_order() {
        assert_eq!(VersionTriple::from_string("1.20.5").unwrap(), VersionTriple(1, 20, 5));
        assert_eq!(VersionTriple::from_string("26").unwrap(), VersionTriple(26, 0, 0));
        assert!(VersionTriple::from_string("1.x").is_err());
        assert!(VersionTriple(1, 9, 9) < VersionTriple(1, 20, 0));
        assert!(VersionTriple::LATEST > VersionTriple(99, 0, 0));
        assert_eq!(VersionTriple(1, 21, 0).to_string(), "1.21.0");
    }

    #[test]
    fn target_parse() {
        let t = TargetVersion::parse("JAVA_1_20_5").unwrap();
        assert_eq!(t, TargetVersion::new(EncodingType::Java, VersionTriple(1, 20, 5)));
        let t = TargetVersion::parse("bedrock_R20_80").unwrap();
        assert_eq!(t, TargetVersion::new(EncodingType::Bedrock, VersionTriple(1, 20, 80)));
        assert_eq!(TargetVersion::parse("Java").unwrap(), TargetVersion::latest(EncodingType::Java));
        assert!(TargetVersion::parse("PREVIEW_1").is_err());
        assert!(TargetVersion::parse("JAVA_1_x").is_err());
    }

    #[test]
    fn target_encoded_round_trip() {
        for id in ["JAVA_1_20_5", "JAVA_1_21", "BEDROCK_R20_80", "BEDROCK_R21", "JAVA"] {
            assert_eq!(TargetVersion::parse(id).unwrap().encoded(), id, "{id}");
        }
    }

    #[test]
    fn editions() {
        assert_eq!(EncodingType::from_name("BEDROCK"), Some(EncodingType::Bedrock));
        assert_eq!(EncodingType::Java.other(), EncodingType::Bedrock);
        assert_eq!(EncodingType::Java.order(), 20);
    }
}
