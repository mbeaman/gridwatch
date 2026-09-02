//! The narrow seam between the tier logic and a GPU library (§8): every
//! backend — nvml-wrapper, the nvidia-smi CSV tier, a test fake — answers the
//! same questions with the same failure vocabulary, so pruning, diffing and
//! the degraded states are written once and tested without a GPU.

use std::fmt;

/// What a probe call can fail with — nvml-wrapper's vocabulary reduced to the
/// cases the poller treats differently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    /// Never polled again (§8 pruning).
    NotSupported,
    /// Nothing to report this tick (`process_utilization_stats` with no fresh
    /// samples); retried.
    NotFound,
    /// A list grew between the count and the fetch; keep the previous rows
    /// and retry next tick.
    InsufficientSize,
    /// The device fell off the bus: re-initialise with backoff.
    GpuLost,
    /// Driver upgraded under a running library — a reboot fixes it, a retry
    /// never does.
    Mismatch,
    /// `libnvidia-ml.so.1` could not be loaded: the nvidia-smi tier applies.
    Loading(String),
    /// Anything else; logged once, treated as transient.
    Other(String),
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::NotSupported => f.write_str("not supported"),
            Fail::NotFound => f.write_str("not found"),
            Fail::InsufficientSize => f.write_str("insufficient size"),
            Fail::GpuLost => f.write_str("GPU lost"),
            Fail::Mismatch => f.write_str("driver/library mismatch — reboot"),
            Fail::Loading(s) => write!(f, "cannot load NVML: {s}"),
            Fail::Other(s) => f.write_str(s),
        }
    }
}

/// The once-per-generation probe. `None` = the library said `NotSupported`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Static {
    pub name: String,
    pub driver: String,
    pub cuda: u32,
    pub arch: String,
    pub uuid: String,
    /// The PCI *device* id (16 bits), `0x2B85` for the 5090.
    pub pci_id: u32,
    pub bus_id: String,
    pub vbios: String,
    pub cores: Option<u32>,
    pub bus_width: Option<u32>,
    pub clock_gfx_max_mhz: Option<u32>,
    pub clock_mem_max_mhz: Option<u32>,
    pub temp_slowdown_c: Option<u32>,
    pub num_fans: u32,
}

/// One entry of a v3 process list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcMem {
    pub pid: u32,
    pub vram_b: Option<u64>,
}

/// One `process_utilization_stats` sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcUtil {
    pub pid: u32,
    /// CPU wall clock, microseconds — the `last_seen` currency.
    pub timestamp_us: u64,
    pub sm: u32,
    pub mem: u32,
    pub enc: u32,
    pub dec: u32,
}

/// A GPU backend. Every method is one library call (or one cached CSV field).
pub trait Probe {
    /// `"nvml"` or `"nvidia-smi"` — for the status reason.
    fn kind(&self) -> &'static str;
    fn static_info(&mut self) -> Result<Static, Fail>;
    /// `(gpu %, memory-controller %)`.
    fn utilization(&mut self) -> Result<(u32, u32), Fail>;
    fn temperature_c(&mut self) -> Result<u32, Fail>;
    fn power_w(&mut self) -> Result<f64, Fail>;
    fn power_limit_w(&mut self) -> Result<f64, Fail>;
    fn clock_gfx_mhz(&mut self) -> Result<u32, Fail>;
    fn clock_mem_mhz(&mut self) -> Result<u32, Fail>;
    /// 0–15; 32 = unknown.
    fn pstate(&mut self) -> Result<u8, Fail>;
    fn throttle_bits(&mut self) -> Result<u64, Fail>;
    /// `(used, total)` bytes, reserved excluded.
    fn memory_b(&mut self) -> Result<(u64, u64), Fail>;
    fn encoder_pct(&mut self) -> Result<u32, Fail>;
    fn decoder_pct(&mut self) -> Result<u32, Fail>;
    /// `(generation, width)`.
    fn pcie_link(&mut self) -> Result<(u32, u32), Fail>;
    /// Monotonic `(tx, rx)` byte counters (fields 197/198).
    fn pcie_bytes(&mut self) -> Result<(u64, u64), Fail>;
    fn fan_pct(&mut self, fan: u32) -> Result<u32, Fail>;
    fn fan_rpm(&mut self, fan: u32) -> Result<u32, Fail>;
    /// Board-power samples newer than `last_ts`, oldest first: `(ts µs, W)`.
    fn power_samples(&mut self, last_ts: u64) -> Result<Vec<(u64, f32)>, Fail>;
    fn graphics_procs(&mut self) -> Result<Vec<ProcMem>, Fail>;
    fn compute_procs(&mut self) -> Result<Vec<ProcMem>, Fail>;
    fn proc_util(&mut self, last_seen_us: u64) -> Result<Vec<ProcUtil>, Fail>;
}
