//! gridwatch-sources: the supervisor and built-in sources (§4.3, §11).
//! Arc 1b shipped the cpu source (procfs meters); arc 2b the gpu source (NVML).

#![forbid(unsafe_code)]

#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "cpu")]
pub mod cpu;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "mpris")]
pub mod mpris;
#[cfg(feature = "pins")]
pub mod pins;
pub mod registry;
#[cfg(feature = "sensors")]
pub mod sensors;
pub mod stub;
pub mod supervisor;

pub use registry::builtin_sources;
pub use supervisor::{SourceHandle, spawn_source};

/// The probes that read only sysfs (arc 5b): safe under `--offline`, so
/// `doctor --offline` still lists the hwmon chips and the RAPL state.
pub fn doctor_offline() -> Vec<(gridwatch_store::Capability, bool, String)> {
    let mut out = Vec::new();
    #[cfg(feature = "sensors")]
    out.extend(sensors::doctor());
    out
}

/// The live probes the sources own, for `gridwatch doctor` (§11, brief arc
/// 3 seam 10): each enabled feature contributes `(capability, ok, what)`.
/// These do real I/O — one exporter GET, `detect_bus` over `/dev/i2c-*` —
/// which the startup probe never does (P18).
#[allow(unused_variables)]
pub fn doctor(exporter: Option<&str>) -> Vec<(gridwatch_store::Capability, bool, String)> {
    let mut out = Vec::new();
    #[cfg(feature = "pins")]
    out.extend(pins::doctor(exporter));
    #[cfg(feature = "audio")]
    out.extend(audio::doctor());
    #[cfg(feature = "mpris")]
    out.extend(mpris::doctor());
    out
}
