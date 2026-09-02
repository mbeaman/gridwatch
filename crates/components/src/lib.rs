//! gridwatch-components: built-in components (§8). `htop` (arc 1b/2a) and
//! `gpu` (arc 2b) beside `clock` (the template) and the `sources` debugging tile.

#![forbid(unsafe_code)]

pub mod alerts;
#[cfg(feature = "audio")]
pub mod audio;
pub mod clock;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "htop")]
pub mod htop;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "pins")]
pub mod pins;
pub mod registry;
#[cfg(feature = "sensors")]
pub mod sensors;
pub mod sources_tile;
#[cfg(feature = "mpris")]
pub mod winamp;

pub use registry::builtin_components;
