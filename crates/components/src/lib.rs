//! gridwatch-components: built-in components (§8). Arc 1b ships `htop` beside
//! `clock` (the template) and the `sources` debugging tile.

#![forbid(unsafe_code)]

pub mod clock;
pub mod htop;
pub mod registry;
pub mod sources_tile;

pub use registry::builtin_components;
