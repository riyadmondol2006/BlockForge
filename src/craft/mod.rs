//! The crafting calculator: pick an item and an amount, get what to gather, what to craft in which order and how
//! much storage it all takes. The recipes are read from the player's own Minecraft jar (so every version works
//! and nothing of Mojang's is bundled); the maths is plain integer arithmetic and runs in well under a millisecond.

pub mod icons;
pub mod load;
pub mod plan;
pub mod saved;
pub mod source;
pub mod stack;
pub mod types;

pub use types::*;
