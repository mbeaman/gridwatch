//! gridwatch-components: built-in components (§8). `htop` (arc 1b/2a) and
//! `gpu` (arc 2b) beside `clock` (the template) and the `sources` debugging tile.

#![forbid(unsafe_code)]

pub mod alerts;
pub mod clock;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "htop")]
pub mod htop;
#[cfg(feature = "pins")]
pub mod pins;
pub mod registry;
pub mod sources_tile;

pub use registry::builtin_components;
