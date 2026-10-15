//! Content: the mods, resource packs, shader packs, data packs and worlds a player adds to an instance.
//!
//! * [`kind`] and [`routing`]: which folder each kind goes to
//! * [`loader_support`]: loader keys, Modrinth ids, what shaders need
//! * [`version_ranges`] and [`version_picker`]: matching and choosing versions
//! * [`performance_pack`]: the optional speed-up mods
//! * [`dropped`]: looking at a dropped file or folder
//! * [`mr`]: the Modrinth objects and API calls
//! * [`service`]: installing, updating, removing and adding files, with a seam for CurseForge
//! * [`drop_flow`]: the pure logic behind the drop screens and the New instance option

pub mod drop_flow;
pub mod dropped;
pub mod kind;
pub mod loader_support;
pub mod mr;
pub mod performance_pack;
pub mod routing;
pub mod service;
pub mod version_picker;
pub mod version_ranges;

pub use dropped::{DroppedFile, DroppedKind};
pub use kind::ContentKind;
pub use service::{ContentService, ServiceOptions};
