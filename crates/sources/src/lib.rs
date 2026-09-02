//! gridwatch-sources: the supervisor and built-in sources (§4.3, §11).
//! Arc 1b shipped the cpu source (procfs meters); arc 2b the gpu source (NVML).

#![forbid(unsafe_code)]

#[cfg(feature = "cpu")]
pub mod cpu;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod registry;
pub mod stub;
pub mod supervisor;

pub use registry::builtin_sources;
pub use supervisor::{SourceHandle, spawn_source};
