// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Chunker's `Version`: a `major.minor.patch` triple with comparisons.

use std::fmt;
use std::str::FromStr;

/// A version number. Ordering is major, then minor, then patch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Version {
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
}

/// A version string that has a non-numeric part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseVersionError(pub String);

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid version \"{}\"", self.0)
    }
}

impl std::error::Error for ParseVersionError {}

impl Version {
    /// Newer than any real version.
    pub const LATEST: Version = Version::new(i32::MAX, i32::MAX, i32::MAX);

    pub const fn new(major: i32, minor: i32, patch: i32) -> Version {
        Version { major, minor, patch }
    }

    /// Parse `1`, `1.20` or `1.20.4`; missing parts are 0 and parts beyond the third are ignored (as Chunker does).
    pub fn parse(input: &str) -> Result<Version, ParseVersionError> {
        let mut parts = input.split('.');
        let mut next = |required: bool| -> Result<i32, ParseVersionError> {
            match parts.next() {
                Some(p) => p.trim().parse::<i32>().map_err(|_| ParseVersionError(input.to_owned())),
                None if required => Err(ParseVersionError(input.to_owned())),
                None => Ok(0),
            }
        };
        let major = next(true)?;
        let minor = next(false)?;
        let patch = next(false)?;
        Ok(Version { major, minor, patch })
    }

    fn triple(&self) -> (i32, i32, i32) {
        (self.major, self.minor, self.patch)
    }

    pub fn is_greater_than_or_equal(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.triple() >= (major, minor, patch)
    }

    pub fn is_greater_than(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.triple() > (major, minor, patch)
    }

    pub fn is_less_than_or_equal(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.triple() <= (major, minor, patch)
    }

    pub fn is_less_than(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.triple() < (major, minor, patch)
    }

    /// The same checks against another version.
    pub fn at_least(&self, other: Version) -> bool {
        *self >= other
    }

    pub fn is_before(&self, other: Version) -> bool {
        *self < other
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Version::parse(s)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl From<(i32, i32, i32)> for Version {
    fn from((major, minor, patch): (i32, i32, i32)) -> Self {
        Version { major, minor, patch }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(Version::parse("1.20.4").unwrap(), Version::new(1, 20, 4));
        assert_eq!(Version::parse("1.21").unwrap(), Version::new(1, 21, 0));
        assert_eq!(Version::parse("26").unwrap(), Version::new(26, 0, 0));
        assert_eq!(Version::parse("1.2.3.4").unwrap(), Version::new(1, 2, 3));
        assert!(Version::parse("1.x").is_err());
        assert!(Version::parse("").is_err());
        assert_eq!("1.19.4".parse::<Version>().unwrap().to_string(), "1.19.4");
    }

    #[test]
    fn comparisons() {
        let v = Version::new(1, 20, 4);
        assert!(v.is_greater_than_or_equal(1, 20, 4));
        assert!(v.is_greater_than_or_equal(1, 19, 99));
        assert!(!v.is_greater_than_or_equal(1, 20, 5));
        assert!(v.is_less_than(1, 21, 0));
        assert!(!v.is_less_than(1, 20, 4));
        assert!(v.is_less_than_or_equal(1, 20, 4));
        assert!(v.is_greater_than(1, 20, 3));
        assert!(!v.is_greater_than(1, 20, 4));
        assert!(Version::LATEST > v);
        assert!(v.at_least(Version::new(1, 20, 0)));
        assert!(Version::new(1, 9, 0) < Version::new(1, 10, 0));
    }
}
