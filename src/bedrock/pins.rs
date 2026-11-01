//! The exact engine and runtime builds BlockForge trusts, compiled in. Nothing read from the network or the disk can
//! change these. Ported from BedrockOnLinux bol/config.py (cc6ac323); see the runtime helper's notices.

/// One downloadable, pinned artifact.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactPin {
    /// Also the name of its folder once installed: a new pin installs beside the old one, never over it.
    pub revision: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// The exact compressed length, so a wrong download stops at its first extra byte.
    pub size: u64,
    /// The single top-level folder every archive member lives in.
    pub archive_root: &'static str,
    pub xz: bool,
    /// How many bytes the unpacked files may take, at most.
    pub unpack_limit: u64,
}

/// BOL's managed WineGDK engine: GDK-Proton with WineGDK native5 and the universal vkd3d-proton build. B:
/// bol/config.py:81-141. HTTP length checked at research time and against the downloaded, verified archive.
pub const ENGINE: ArtifactPin = ArtifactPin {
    revision: "wow64-archs-native21",
    url: "https://github.com/Wyze3306/BedrockOnLinux/releases/download/engine-wow64-archs-native21/GDK-Proton-xuser-wow64-archs-native21.tar.gz",
    sha256: "e98512e74b1c3f930fdd213b24ac4a01abfa3a6336276076a509530c540b2b13",
    size: 862_730_720,
    archive_root: "GDK-Proton-xuser",
    xz: false,
    unpack_limit: 8 * 1024 * 1024 * 1024,
};

/// Steam Linux Runtime 3 (sniper), the exact build whose SHA-256 is in Valve's published SHA256SUMS. Resolved once
/// here, never "latest" at Play.
pub const RUNTIME: ArtifactPin = ArtifactPin {
    revision: bedrock_runtime::STEAMRT_BUILD,
    url: "https://repo.steampowered.com/steamrt3/images/3.0.20261021.266025/SteamLinuxRuntime_sniper.tar.xz",
    sha256: "86956ad0ee62e89134f5e30359cd1019d1ecb614c7b677dad82d4d00f51a2696",
    size: 194_771_152,
    archive_root: "SteamLinuxRuntime_sniper",
    xz: true,
    unpack_limit: 4 * 1024 * 1024 * 1024,
};

/// pressure-vessel's own version in that build (its `VERSIONS.txt`).
pub const PRESSURE_VESSEL: &str = "0.20261007.0";

/// The reviewed engine-manifest.json of native21, byte for byte as shipped in the archive. Its critical-file and
/// graphics-variant hashes are what an installed engine is checked against.
pub const ENGINE_MANIFEST: &str = include_str!("fixtures/engine-native21.json");

/// Native entrypoints and the platform mtree are independently anchored before pv-verify runs.
pub const RUNTIME_CRITICAL: &[(&str, &str)] = &[
    ("mtree.txt.gz", "6e97f6df0f34b9f02590b683161eb0b23f72f761294faf5186606e35067916cb"),
    ("pressure-vessel/bin/pv-verify", "e8b68e190ad9243fb2f87f0d3aa130398d2f422b76771eeb86a79b8241c6af95"),
    ("pressure-vessel/bin/pressure-vessel-wrap", "5357ec29b464961bfd1a743d4a28732d971062719933c9cbbea6ae95eb7110ad"),
    (
        "sniper_platform_3.0.20261021.266025/usr-mtree.txt.gz",
        "e14acf4275b3639f014a31bb155e92ce9eb344e1e1cd4214063658baa59fbae7",
    ),
];

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_pin_and_helper_agree_on_one_build() {
        assert!(super::RUNTIME.url.contains(super::RUNTIME.revision));
        assert!(super::RUNTIME_CRITICAL.iter().any(|(p, _)| p.contains(super::RUNTIME.revision)));
    }
}
