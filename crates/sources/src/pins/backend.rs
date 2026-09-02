//! The backend seam (brief arc 3 seam 3): the i2c chip, the exporter and a
//! test fake answer the same three questions with the same failure
//! vocabulary, so the loop, the redetect counter, the lifecycle bridge and
//! every test run over a fake — the role `gpu::probe::Probe` plays.

use std::fmt;

use astral_watch::decode::Reading;
use gridwatch_store::keys::pins::PinsMode;

/// Why a read produced no telemetry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loss {
    /// The chip/exporter did not answer (EIO, timeout, connection refused).
    Unreachable(String),
    /// It answered, but not with telemetry: a deeply idle GPU's MCU answers
    /// zeros; an exporter says `up 0` or a stale reading age.
    Implausible,
    /// The device or endpoint is gone: re-detect.
    NotFound,
    /// `/dev/i2c-*` refused: not in the `i2c` group.
    Permission,
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Loss::Unreachable(s) => write!(f, "read failed: {s}"),
            Loss::Implausible => {
                f.write_str("implausible reading (chip answered; wrong device or GPU idle?)")
            }
            Loss::NotFound => f.write_str("telemetry source not found"),
            Loss::Permission => f.write_str("permission denied on /dev/i2c-*"),
        }
    }
}

/// What a backend knows about itself, for `pins.info`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Described {
    pub bus: Option<u32>,
    pub addr: u16,
    pub pci: String,
    pub model: Option<String>,
    /// `"block"`, `"bytewise"` or `"unknown"`.
    pub access: String,
}

pub trait PinsBackend {
    fn kind(&self) -> PinsMode;
    fn describe(&mut self) -> Result<Described, Loss>;
    fn read(&mut self) -> Result<Reading, Loss>;
    /// The exporter's own debounced `alert_active` flags, empty elsewhere.
    fn service_active(&self) -> Vec<String> {
        Vec::new()
    }
    /// The sample interval changed (`SetOption`): a backend with a staleness
    /// rule keyed on it re-reads it here.
    fn set_interval(&mut self, _interval: std::time::Duration) {}
    /// After `REDETECT_AFTER` misses: find the card again (a new bus after a
    /// GPU reset). `Ok(true)` when something changed and `describe` should
    /// be re-read.
    fn redetect(&mut self) -> Result<bool, Loss> {
        Ok(false)
    }
}
