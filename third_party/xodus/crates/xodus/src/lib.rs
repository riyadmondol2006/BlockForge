pub mod api;
#[cfg(feature = "xal-auth")]
pub mod auth;
pub mod clep;
pub mod hardware;
pub mod licensing;
pub mod models;
pub mod secrets;
pub mod tokens;

pub const XBOX_LIVE_PACKAGES_PC: &str = "https://packagespc.xboxlive.com";

#[cfg(feature = "xal-auth")]
pub use xal;

pub mod proto {
    pub mod xodus {
        include!(concat!(env!("OUT_DIR"), "/xodus.rs"));
    }
}
