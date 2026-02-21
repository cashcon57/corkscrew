//! Game-specific plugin modules.
//!
//! Each sub-module implements [`crate::games::GamePlugin`] for a particular
//! game and provides a `register()` function to insert it into the global
//! plugin registry.

pub mod fallout4;
pub mod skyrim_plugins;
pub mod skyrim_se;
