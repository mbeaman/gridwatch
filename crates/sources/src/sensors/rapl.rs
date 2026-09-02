//! RAPL package power (brief arc 5 seam 6): `intel-rapl:0/energy_uj`
//! readable ⇒ `sensor.power_w{rapl:package-0}` from `Δenergy mod
//! max_energy_range_uj / Δt`; EACCES ⇒ `RootOnly` (the udev rule is the
//! documented fix — torch's file is 0400, CVE-2020-8694); absent ⇒ `Absent`.
//! The intel_rapl driver serves AMD Zen too.

use std::path::{Path, PathBuf};

use gridwatch_store::Ts;
use gridwatch_store::keys::sensors::RaplState;

/// The fix `doctor` prints: a group-readable rule, never world-readable —
/// the kernel locked `energy_uj` because of Platypus (CVE-2020-8694), so
/// widening it to everyone is the wrong advice (review).
pub const UDEV_HINT: &str = "RAPL power needs a udev rule: SUBSYSTEM==\"powercap\", KERNEL==\"intel-rapl:0\", GROUP=\"powermon\", MODE=\"0440\" (and add yourself to that group)";

/// A package draw above this is not a reading, it is a counter reset: the
/// sample is dropped and the baseline re-seeded (review).
pub const MAX_PLAUSIBLE_W: f64 = 2_000.0;

#[derive(Clone, Debug)]
pub struct Rapl {
    state: RaplState,
    energy: PathBuf,
    /// The counter wraps at this many µJ.
    range: u64,
    last: Option<(Ts, u64)>,
}

fn read_u64(p: &Path) -> std::io::Result<u64> {
    let s = std::fs::read_to_string(p)?;
    s.trim()
        .parse::<u64>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

impl Rapl {
    /// Probe `<sys>/class/powercap/intel-rapl:0` once.
    pub fn probe(sys: &Path) -> Rapl {
        let dir = sys.join("class/powercap/intel-rapl:0");
        let energy = dir.join("energy_uj");
        let range = read_u64(&dir.join("max_energy_range_uj")).unwrap_or(u64::MAX);
        let state = if !dir.exists() {
            RaplState::Absent
        } else {
            match read_u64(&energy) {
                Ok(_) => RaplState::Ok,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => RaplState::RootOnly,
                Err(_) => RaplState::Absent,
            }
        };
        Rapl {
            state,
            energy,
            range,
            last: None,
        }
    }

    pub fn state(&self) -> RaplState {
        self.state
    }

    /// Re-probe (a udev rule applied, `powercap` loaded after start): called
    /// on the source's re-walk so a fix takes effect without a restart.
    pub fn reprobe(&mut self, sys: &Path) {
        if self.state != RaplState::Ok {
            let fresh = Rapl::probe(sys);
            if fresh.state != self.state {
                *self = fresh;
            }
        }
    }

    /// The package power since the last call, in W; `None` on the first
    /// call, when unreadable, or when Δt is zero.
    pub fn sample(&mut self, at: Ts) -> Option<f64> {
        if self.state != RaplState::Ok {
            return None;
        }
        let now = read_u64(&self.energy).ok()?;
        let prev = self.last.replace((at, now));
        let (t0, e0) = prev?;
        let dt = at.since(t0).as_secs_f64();
        if dt <= 0.0 {
            return None;
        }
        let delta = if now >= e0 {
            now - e0
        } else {
            // Wrapped at `max_energy_range_uj` — or the counter reset
            // (a resume, a driver reload). Both look the same; only the
            // resulting power tells them apart.
            self.range.saturating_sub(e0).saturating_add(now)
        };
        let w = delta as f64 / 1e6 / dt;
        if !w.is_finite() || w > MAX_PLAUSIBLE_W {
            // Re-seeded by the `replace` above; this sample is not a number
            // anyone should read.
            return None;
        }
        Some(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(perm_ok: bool) -> PathBuf {
        let root = std::env::temp_dir().join(format!("gw-rapl-{}-{}", std::process::id(), perm_ok));
        let dir = root.join("class/powercap/intel-rapl:0");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("name"), "package-0\n").unwrap();
        std::fs::write(dir.join("max_energy_range_uj"), "1000\n").unwrap();
        std::fs::write(dir.join("energy_uj"), "100\n").unwrap();
        if !perm_ok {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                dir.join("energy_uj"),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();
        }
        root
    }

    #[test]
    fn states_and_the_wrapping_delta() {
        assert_eq!(
            Rapl::probe(Path::new("/nonexistent")).state(),
            RaplState::Absent
        );
        let root = tree(true);
        let mut r = Rapl::probe(&root);
        assert_eq!(r.state(), RaplState::Ok);
        assert_eq!(r.sample(Ts(1_000_000_000)), None, "no delta yet");
        let e = root.join("class/powercap/intel-rapl:0/energy_uj");
        std::fs::write(&e, "600\n").unwrap();
        let w = r.sample(Ts(2_000_000_000)).unwrap();
        assert!((w - 500e-6).abs() < 1e-12, "500 µJ in 1 s: {w}");
        // Wrap: 600 → 50 over a 1000 µJ range = 450 µJ.
        std::fs::write(&e, "50\n").unwrap();
        let w = r.sample(Ts(4_000_000_000)).unwrap();
        assert!((w - 450e-6 / 2.0).abs() < 1e-12, "{w}");
        assert_eq!(r.sample(Ts(4_000_000_000)), None, "Δt = 0");
        // A counter reset (resume, driver reload) is not a 65 kW package.
        std::fs::write(&e, "999\n").unwrap();
        r.sample(Ts(5_000_000_000));
        std::fs::write(&e, "1\n").unwrap();
        let mut big = Rapl {
            state: RaplState::Ok,
            energy: e.clone(),
            range: u64::MAX,
            last: Some((Ts(5_000_000_000), 999)),
        };
        assert_eq!(big.sample(Ts(6_000_000_000)), None, "implausible → dropped");
        std::fs::write(&e, "500001\n").unwrap();
        let w = big.sample(Ts(7_000_000_000)).unwrap();
        assert!((w - 0.5).abs() < 1e-9, "the baseline was re-seeded: {w}");
        let _ = std::fs::remove_dir_all(&root);
        // Root-only (skipped when running as root, where 0o000 still reads).
        let root = tree(false);
        let r = Rapl::probe(&root);
        if std::fs::read(root.join("class/powercap/intel-rapl:0/energy_uj")).is_err() {
            assert_eq!(r.state(), RaplState::RootOnly);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
