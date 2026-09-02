//! gridwatch-sources: the supervisor and built-in sources (§4.3, §11).
//! Real sources land per arc; arc 1b ships the cpu source (procfs meters).

#![forbid(unsafe_code)]

#[cfg(feature = "cpu")]
pub mod cpu;
pub mod registry;
pub mod stub;
pub mod supervisor;

pub use registry::builtin_sources;
pub use supervisor::{SourceHandle, spawn_source};
