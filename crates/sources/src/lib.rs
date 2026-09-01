//! gridwatch-sources: the supervisor and built-in sources (§4.3, §11).
//! Real sources land per arc; arc 1a ships the supervisor and the cpu stub.

#![forbid(unsafe_code)]

pub mod registry;
pub mod stub;
pub mod supervisor;

pub use registry::builtin_sources;
pub use supervisor::{SourceHandle, spawn_source};
