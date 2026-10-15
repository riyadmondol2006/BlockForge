//! Keeping BlockForge itself up to date: looking for a newer release on GitHub and, with one click, downloading the
//! file that fits how this copy was installed, checking it and putting it in place.
//!
//! * [`plan`]: version rules, how this copy was installed, which release file fits (no network, no disk)
//! * [`check`]: asking GitHub, the "offer it once" rule and the status the About page shows
//! * [`download`]: the download with progress, size check, SHA-256 check and cancel
//! * [`apply`]: replacing the AppImage or portable program, running the installer or package manager

pub mod apply;
pub mod check;
pub mod download;
pub mod plan;
pub mod release;

pub use apply::Installed;
pub use check::{Found, Offers, UpdateStatus};
pub use download::{Cancelled, DownloadProgress};
pub use plan::{Kind, current_version, display_version, install_kind, is_newer, parse_version, pick_asset};
pub use release::{Asset, Release};

#[cfg(test)]
mod tests;
