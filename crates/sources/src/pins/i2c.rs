//! The direct-i2c backend over astral-watch's own reader (digest §1–2):
//! `detect_bus` once, `read_reading` per sample (the crate latches the block
//! path itself), `redetect_card` after ten misses. Contention with a root
//! logger costs latency, never a corrupted reading (the kernel's per-adapter
//! lock) — an error is a `TelemetryLost` sample, not a restart.

use astral_watch::cards;
use astral_watch::decode::Reading;
use astral_watch::i2c::{self, CHIP_ADDR, Detect};
use gridwatch_store::keys::pins::PinsMode;

use super::backend::{Described, Loss, PinsBackend};

pub struct I2cBackend {
    bus: u32,
    pci: String,
}

impl I2cBackend {
    /// `detect_bus` → a backend, or the reason nothing answered.
    pub fn detect() -> Result<I2cBackend, Detect> {
        match i2c::detect_bus(CHIP_ADDR) {
            Detect::Found(bus) => Ok(I2cBackend {
                bus,
                pci: i2c::bus_pci_id(bus).unwrap_or_default(),
            }),
            other => Err(other),
        }
    }

    pub fn bus(&self) -> u32 {
        self.bus
    }

    /// The user-facing reason and fix for a `Detect` that is not `Found`.
    pub fn explain(d: Detect) -> (&'static str, &'static str) {
        match d {
            Detect::Found(_) => ("found", ""),
            Detect::NoBuses => (
                "no NVIDIA i2c adapter",
                "is the NVIDIA driver loaded and i2c-dev present?",
            ),
            Detect::PermissionDenied => (
                "permission denied on /dev/i2c-*",
                "add yourself to the i2c group: `sudo usermod -aG i2c $USER`, then log in again",
            ),
            Detect::NoTelemetry => (
                "waiting for telemetry (GPU idle?)",
                "the chip answers zeros while the GPU is deeply idle; retried every 10 s",
            ),
        }
    }
}

/// Classify an astral-watch read error by its **cause chain** (`{e:#}`): the
/// outermost anyhow context is `opening /dev/i2c-N @ 0x2b`, the io text sits
/// below it (review: `to_string()` never saw "permission denied").
pub fn classify(chain: &str) -> Loss {
    let lower = chain.to_lowercase();
    if lower.contains("permission denied") || lower.contains("os error 13") {
        Loss::Permission
    } else if lower.contains("no such file")
        || lower.contains("no such device")
        || lower.contains("os error 2)")
        || lower.contains("os error 6)")
        || lower.contains("os error 19)")
    {
        Loss::NotFound
    } else {
        Loss::Unreachable(chain.to_string())
    }
}

impl PinsBackend for I2cBackend {
    fn kind(&self) -> PinsMode {
        PinsMode::I2c
    }

    fn describe(&mut self) -> Result<Described, Loss> {
        let model = cards::gpu_at(&self.pci).and_then(|g| g.model().map(str::to_string));
        Ok(Described {
            bus: Some(self.bus),
            addr: CHIP_ADDR,
            pci: self.pci.clone(),
            model,
            // The crate latches block vs bytewise privately; it prints the
            // choice to the (redirected) log once. Not exposed, not guessed.
            access: "unknown".into(),
        })
    }

    fn read(&mut self) -> Result<Reading, Loss> {
        let r = i2c::read_reading(self.bus, CHIP_ADDR).map_err(|e| classify(&format!("{e:#}")))?;
        if r.plausible() {
            Ok(r)
        } else {
            Err(Loss::Implausible)
        }
    }

    fn redetect(&mut self) -> Result<bool, Loss> {
        match i2c::redetect_card(CHIP_ADDR, &self.pci) {
            Some(bus) if bus != self.bus => {
                self.bus = bus;
                Ok(true)
            }
            Some(_) => Ok(false),
            None => Err(Loss::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_reads_the_cause_below_astral_watchs_context() {
        let chain = "opening /dev/i2c-3 @ 0x2b: Permission denied (os error 13)";
        assert_eq!(classify(chain), Loss::Permission);
        let chain = "opening /dev/i2c-3 @ 0x2b: No such file or directory (os error 2)";
        assert_eq!(classify(chain), Loss::NotFound);
        let chain = "reading /dev/i2c-3 @ 0x2b: Input/output error (os error 5)";
        assert!(matches!(classify(chain), Loss::Unreachable(_)));
        // The outermost context alone (what `to_string()` gives) is a plain loss.
        assert!(matches!(
            classify("opening /dev/i2c-3 @ 0x2b"),
            Loss::Unreachable(_)
        ));
    }
}
