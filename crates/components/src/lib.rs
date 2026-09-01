//! gridwatch-components: built-in components (§8). Arc 1a ships `clock` (the
//! template) and the `sources` debugging tile; htop lands in arc 1b.

#![forbid(unsafe_code)]

pub mod clock;
pub mod registry;
pub mod sources_tile;

pub use registry::builtin_components;
