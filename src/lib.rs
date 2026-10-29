//! BlockForge core: everything the launcher does that isn't drawing a window.
//!
//! It reads and writes the same files as the Java edition (`instance.json`, `accounts.json`,
//! `BlockForge.json`, the shared `libraries/` and `assets/` folders and the Mojang-layout `runtimes/`
//! folder), so an existing install keeps working in both.

// Style lints that would make the code harder to read here: nested `if`s that separate unrelated checks, long
// parameter lists that mirror the Java edition's APIs, and message enums with one large variant.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]
#![allow(
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::large_enum_variant,
    clippy::unnecessary_sort_by,
    clippy::manual_is_multiple_of,
    clippy::chunks_exact_to_as_chunks,
    clippy::needless_range_loop,
    clippy::if_same_then_else,
    clippy::collapsible_match,
    clippy::manual_range_patterns
)]

pub mod accounts;
pub mod backups;
pub mod canary;
pub mod cf_seam;
pub mod classcache;
pub mod config;
pub mod content;
#[cfg(feature = "converter")]
pub mod convert;
pub mod craft;
pub mod curseforge;
pub mod dependencies;
pub mod desktop_integration;
pub mod diagnostics;
pub mod disk;
pub mod export;
pub mod ftb;
pub mod github;
pub mod gpu;
pub mod http;
pub mod i18n;
pub mod imports;
pub mod install;
pub mod instances;
pub mod java;
pub mod launch;
pub mod leftovers;
pub mod loaders;
pub mod logs;
pub mod modcheck;
pub mod model;
pub mod modrecord;
pub mod modrinth;
pub mod mods;
pub mod msauth;
pub mod net;
pub mod news;
pub mod packs;
pub mod packs_cli;
pub mod packutil;
pub mod paths;
pub mod platform;
pub mod privacy;
mod ranged;
pub mod retry;
pub mod runtime;
pub mod servers;
pub mod skins;
mod skins_data;
pub mod smithed;
pub mod snapshots;
pub mod svcsettings;
#[cfg(test)]
pub mod testutil;
pub mod tools;
pub mod update;
pub mod warmer;

pub const NAME: &str = "BlockForge";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub mod gui;

/// When the program started, for measuring how long the window takes to appear.
pub static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
