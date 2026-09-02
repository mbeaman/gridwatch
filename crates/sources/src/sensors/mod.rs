//! The sensors source (§5 cadence row, §8, brief arc 5 seam 6): the hwmon
//! walker over `/sys/class/hwmon` at 1 s (SMBus/SMN/NVMe-log reads — never
//! faster), the chips' own thresholds once per generation, RAPL package
//! power when `energy_uj` is readable, and the k10temp handover: with this
//! feature compiled in the cpu source stops publishing
//! `sensor.temp_c{k10temp:*}` and this source publishes the same key with
//! the same labels, so the htop tile's `Tccd` column reads unchanged (§16).

pub mod hwmon;
pub mod rapl;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gridwatch_store::keys::sensors::{self, RaplState, SensorsInfo};
use gridwatch_store::{
    Cadence, Datum, Label, Level, MetricId, Sample, Source, SourceCtx, SourceInfo, SourceState,
    SourceStatus, Ts, demo,
};

use hwmon::{Inventory, Kind};
use rapl::Rapl;

/// `[sources.sensors]` (§9): the refresh (clamped 500–10000 ms), the chip
/// filter (globs by name: `"*"`, `"nvme*"`) and the RAPL switch.
pub const OPTION_NAMES: &[&str] = &["refresh_ms", "chips", "rapl"];
pub const MIN_REFRESH: Duration = Duration::from_millis(500);
pub const MAX_REFRESH: Duration = Duration::from_secs(10);
/// The inventory is walked again this often (a chip that appeared, an
/// NVMe that went away).
pub const REWALK: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub refresh: Duration,
    pub chips: Vec<String>,
    pub rapl: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            refresh: Duration::from_secs(1),
            chips: vec!["*".into()],
            rapl: true,
        }
    }
}

pub fn clamp_refresh(ms: i64) -> Duration {
    Duration::from_millis((ms.max(0) as u64).clamp(
        MIN_REFRESH.as_millis() as u64,
        MAX_REFRESH.as_millis() as u64,
    ))
}

impl Options {
    pub fn from_table(t: &toml::Table) -> Options {
        let mut o = Options::default();
        if let Some(ms) = t.get("refresh_ms").and_then(|v| v.as_integer()) {
            o.refresh = clamp_refresh(ms);
            if o.refresh.as_millis() as i64 != ms {
                tracing::warn!(
                    "[sources.sensors] refresh_ms = {ms} clamped to {} (500–10000)",
                    o.refresh.as_millis()
                );
            }
        }
        if let Some(list) = t.get("chips").and_then(|v| v.as_array()) {
            let chips: Vec<String> = list
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !chips.is_empty() {
                o.chips = chips;
            }
        }
        o.rapl = !matches!(t.get("rapl").and_then(|v| v.as_str()), Some("off"));
        o
    }
}

/// `gridwatch doctor`'s row (seam 8): the chips found and the RAPL state —
/// sysfs reads, never a device.
pub fn doctor() -> Vec<(gridwatch_store::Capability, bool, String)> {
    use gridwatch_store::Capability;
    let sys = PathBuf::from("/sys");
    let inv = hwmon::walk(&sys, &["*".to_string()]);
    let names: Vec<&str> = inv.chips.iter().map(|c| c.name.as_str()).collect();
    let r = Rapl::probe(&sys);
    let rapl = match r.state() {
        RaplState::Ok => "RAPL package power readable".to_string(),
        RaplState::RootOnly => format!("RAPL energy_uj is root-only — {}", rapl::UDEV_HINT),
        RaplState::Absent => "no RAPL (intel-rapl:0 absent)".to_string(),
    };
    vec![
        (
            Capability::Hwmon,
            !inv.chips.is_empty(),
            if inv.chips.is_empty() {
                "no hwmon chips under /sys/class/hwmon".to_string()
            } else {
                format!(
                    "{} chips, {} inputs: {}",
                    inv.chips.len(),
                    inv.sensors.len(),
                    names.join(", ")
                )
            },
        ),
        (Capability::Rapl, r.state() == RaplState::Ok, rapl),
    ]
}

fn named(key: &gridwatch_store::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

/// One walk-and-read pass over an inventory: pure over the tree, tested on
/// the fixture.
pub struct Sampler {
    pub sys: PathBuf,
    pub options: Options,
    inv: Inventory,
    rapl: Rapl,
    walked: Option<Instant>,
    thresholds_sent: bool,
    pub walk_ms: f64,
}

impl Sampler {
    pub fn new(sys: PathBuf, options: Options) -> Sampler {
        let rapl = Rapl::probe(&sys);
        Sampler {
            sys,
            options,
            inv: Inventory::default(),
            rapl,
            walked: None,
            thresholds_sent: false,
            walk_ms: 0.0,
        }
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inv
    }

    pub fn rapl_state(&self) -> RaplState {
        if self.options.rapl {
            self.rapl.state()
        } else {
            RaplState::Absent
        }
    }

    pub fn info(&self) -> SensorsInfo {
        SensorsInfo {
            chips: self.inv.chips.clone(),
            rapl: self.rapl_state(),
        }
    }

    /// The samples for one tick: every reading, plus the thresholds and the
    /// inventory Record on the first tick and after a re-walk.
    pub fn sample(&mut self, at: Ts, now: Instant) -> Vec<Sample> {
        let t0 = Instant::now();
        let rewalk = self
            .walked
            .is_none_or(|w| now.saturating_duration_since(w) >= REWALK);
        if rewalk {
            let inv = hwmon::walk(&self.sys, &self.options.chips);
            let changed = inv != self.inv;
            self.inv = inv;
            self.walked = Some(now);
            if changed {
                self.thresholds_sent = false;
            }
        }
        let mut out = Vec::with_capacity(self.inv.sensors.len() * 3 + 3);
        for s in &self.inv.sensors {
            let key = match s.kind {
                Kind::Temp => &sensors::TEMP_C,
                Kind::Fan => &sensors::FAN_RPM,
                Kind::In => &sensors::VOLT_V,
                Kind::Power => &sensors::POWER_W,
            };
            if let Some(v) = hwmon::read(s) {
                out.push(Sample {
                    id: named(key, &s.key),
                    datum: Datum::Scalar(v),
                });
            }
            if !self.thresholds_sent && s.kind == Kind::Temp {
                if let Some(m) = s.max {
                    out.push(Sample {
                        id: named(&sensors::MAX_C, &s.key),
                        datum: Datum::Scalar(m),
                    });
                }
                if let Some(c) = s.crit {
                    out.push(Sample {
                        id: named(&sensors::CRIT_C, &s.key),
                        datum: Datum::Scalar(c),
                    });
                }
            }
        }
        if self.options.rapl
            && let Some(w) = self.rapl.sample(at)
        {
            out.push(Sample {
                id: named(&sensors::POWER_W, "rapl:package-0"),
                datum: Datum::Scalar(w),
            });
        }
        self.walk_ms = t0.elapsed().as_secs_f64() * 1000.0;
        out.push(Sample {
            id: sensors::WALK_MS.id.clone(),
            datum: Datum::Scalar(self.walk_ms),
        });
        if !self.thresholds_sent {
            self.thresholds_sent = true;
            out.push(Sample {
                id: sensors::INFO.id.clone(),
                datum: Datum::Record(Arc::new(self.info())),
            });
        }
        out
    }
}

pub struct SensorsSource {
    options: Options,
}

impl SensorsSource {
    pub fn new(options: &toml::Table) -> SensorsSource {
        SensorsSource {
            options: Options::from_table(options),
        }
    }

    fn cadence(&self) -> Cadence {
        Cadence {
            hidden: Some(self.options.refresh),
            visible: self.options.refresh,
            focused: self.options.refresh,
            always_on: false,
        }
    }
}

fn status(cx: &SourceCtx, state: SourceState, reason: Option<&str>, hint: Option<&str>) {
    cx.status(SourceStatus {
        state,
        reason: reason.map(Arc::from),
        hint: hint.map(Arc::from),
        since: cx.clock.now(),
        last_sample: None,
        dropped: 0,
        restarts: cx.restarts,
    });
}

impl Source for SensorsSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: self.cadence(),
            ..demo::sensors_info_static()
        }
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        status(&cx, SourceState::Starting, None, None);
        let mut sampler = Sampler::new(PathBuf::from("/sys"), self.options.clone());
        let cadence = self.cadence();
        // Prime the pump (P18: every source live within 2 s).
        let mut first = true;
        loop {
            if !first {
                let level = cx.demand.level();
                let Some(period) = cadence.for_level(level) else {
                    // Paused: nothing is published.
                    if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                        return;
                    }
                    continue;
                };
                if !cx.sleep_until(cx.next_deadline(period)) {
                    return;
                }
            }
            first = false;
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            if cx.demand.level() == Level::Paused {
                continue;
            }
            let at = cx.clock.now();
            let samples = sampler.sample(at, Instant::now());
            let chips = sampler.inventory().chips.len();
            if chips == 0 {
                status(
                    &cx,
                    SourceState::Unavailable,
                    Some("no hwmon chips"),
                    Some("nothing under /sys/class/hwmon exports an input"),
                );
                cx.emit(at, samples);
                continue;
            }
            // RootOnly is a hint, not Degraded: the rest works (seam 6).
            let hint = match sampler.rapl_state() {
                RaplState::RootOnly => Some(rapl::UDEV_HINT),
                _ => None,
            };
            let reason = format!(
                "{chips} chips, {} inputs",
                sampler.inventory().sensors.len()
            );
            status(&cx, SourceState::Ok, Some(&reason), hint);
            cx.emit(at, samples);
        }
    }
}

pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(SensorsSource::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_and_clamp() {
        let t: toml::Table = toml::from_str(
            r#"refresh_ms = 100
chips = ["nvme*", "k10temp"]
rapl = "off""#,
        )
        .unwrap();
        let o = Options::from_table(&t);
        assert_eq!(o.refresh, MIN_REFRESH);
        assert_eq!(o.chips, ["nvme*", "k10temp"]);
        assert!(!o.rapl);
        let t: toml::Table = toml::from_str("refresh_ms = 60000").unwrap();
        assert_eq!(Options::from_table(&t).refresh, MAX_REFRESH);
        assert_eq!(Options::from_table(&toml::Table::new()), Options::default());
        assert_eq!(OPTION_NAMES.len(), 3);
    }

    /// The sampler over the torch fixture tree: every reading labelled
    /// `chip:label`, the thresholds and the inventory once, the k10temp
    /// labels the htop tile reads.
    #[test]
    fn samples_the_torch_fixture_with_thresholds_once() {
        let root = std::env::temp_dir().join(format!("gw-sensors-{}", std::process::id()));
        let hw = root.join("class/hwmon");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&hw).unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hwmon/torch");
        for e in std::fs::read_dir(&fixture).unwrap().flatten() {
            let dst = hw.join(e.file_name());
            std::fs::create_dir_all(&dst).unwrap();
            for f in std::fs::read_dir(e.path()).unwrap().flatten() {
                std::fs::copy(f.path(), dst.join(f.file_name())).unwrap();
            }
        }
        let mut s = Sampler::new(root.clone(), Options::default());
        let now = Instant::now();
        let a = s.sample(Ts(1_000_000_000), now);
        let temps = a.iter().filter(|x| x.id.name == "sensor.temp_c").count();
        assert_eq!(temps, 15, "every temp input on torch");
        let labels: Vec<String> = a
            .iter()
            .filter(|x| x.id.name == "sensor.temp_c")
            .map(|x| x.id.label.to_string().trim_matches(['{', '}']).to_string())
            .collect();
        for want in [
            "k10temp:Tctl",
            "k10temp:Tccd1",
            "k10temp:Tccd2",
            "nvme#3:Composite",
        ] {
            assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
        }
        let maxes = a.iter().filter(|x| x.id.name == "sensor.max_c").count();
        assert_eq!(maxes, 7, "nvme×3 composite + r8169×2 + spd5118×2: {maxes}");
        assert_eq!(a.iter().filter(|x| x.id.name == "sensor.crit_c").count(), 5);
        assert_eq!(a.iter().filter(|x| x.id.name == "sensor.info").count(), 1);
        assert_eq!(
            s.rapl_state(),
            RaplState::Absent,
            "no powercap in the fixture"
        );
        assert!(a.iter().any(|x| x.id.name == "sensor.walk_ms"));
        let b = s.sample(Ts(2_000_000_000), now + Duration::from_secs(1));
        assert_eq!(
            b.iter().filter(|x| x.id.name == "sensor.max_c").count(),
            0,
            "once"
        );
        assert_eq!(b.iter().filter(|x| x.id.name == "sensor.info").count(), 0);
        assert_eq!(
            b.iter().filter(|x| x.id.name == "sensor.temp_c").count(),
            15
        );
        // A re-walk after a minute with an unchanged tree republishes nothing.
        let c = s.sample(Ts(70_000_000_000), now + Duration::from_secs(70));
        assert_eq!(c.iter().filter(|x| x.id.name == "sensor.info").count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
