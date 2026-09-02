//! Deterministic sensors synthesis (§12.5, brief arc 5 5b): torch's hwmon
//! inventory (three nvme drives, k10temp's Tctl/Tccd1/Tccd2, two spd5118
//! DIMMs, the Wi-Fi radio, two r8169 NICs) with slow seeded drifts, the
//! nvme thresholds, RAPL `root_only` (torch's `energy_uj` is 0400). Byte
//! deterministic per `(seed, Ts)`.

use std::sync::Arc;
use std::time::Duration;

use crate::demo::XorShift;
use crate::key::{Datum, Label, MetricId};
use crate::keys::sensors::{self, ChipInfo, RaplState, SensorsInfo};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// One synthetic reading: `chip:label`, its idle value, its swing and the
/// chip's thresholds.
struct Row {
    label: &'static str,
    base: f64,
    swing: f64,
    max: Option<f64>,
    crit: Option<f64>,
}

const ROWS: &[Row] = &[
    Row {
        label: "k10temp:Tctl",
        base: 52.0,
        swing: 14.0,
        max: None,
        crit: None,
    },
    Row {
        label: "k10temp:Tccd1",
        base: 50.0,
        swing: 12.0,
        max: None,
        crit: None,
    },
    Row {
        label: "k10temp:Tccd2",
        base: 46.0,
        swing: 10.0,
        max: None,
        crit: None,
    },
    Row {
        label: "nvme:Composite",
        base: 48.0,
        swing: 4.0,
        max: Some(81.85),
        crit: Some(84.85),
    },
    Row {
        label: "nvme:Sensor 1",
        base: 48.0,
        swing: 4.0,
        max: None,
        crit: None,
    },
    Row {
        label: "nvme:Sensor 2",
        base: 49.0,
        swing: 4.0,
        max: None,
        crit: None,
    },
    Row {
        label: "nvme#2:Composite",
        base: 44.0,
        swing: 3.0,
        max: Some(83.85),
        crit: Some(87.85),
    },
    Row {
        label: "nvme#3:Composite",
        base: 36.0,
        swing: 2.0,
        max: Some(74.85),
        crit: Some(79.85),
    },
    Row {
        label: "mt7925_phy0:temp1",
        base: 50.0,
        swing: 2.0,
        max: None,
        crit: None,
    },
    Row {
        label: "r8169_0_b00:00:temp1",
        base: 58.0,
        swing: 2.0,
        max: Some(120.0),
        crit: None,
    },
    Row {
        label: "r8169_0_c00:00:temp1",
        base: 55.0,
        swing: 2.0,
        max: Some(110.0),
        crit: None,
    },
    Row {
        label: "spd5118:temp1",
        base: 40.5,
        swing: 3.0,
        max: Some(55.0),
        crit: Some(85.0),
    },
    Row {
        label: "spd5118#2:temp1",
        base: 39.0,
        swing: 3.0,
        max: Some(55.0),
        crit: Some(85.0),
    },
];

/// The inventory the synth publishes (also the journal exemplar).
pub fn sensors_info() -> SensorsInfo {
    let chip = |name: &str, n: usize, kinds: &[&str]| ChipInfo {
        name: name.into(),
        path: format!("/sys/class/hwmon/hwmon{n}"),
        kinds: kinds.iter().map(|k| k.to_string()).collect(),
    };
    SensorsInfo {
        chips: vec![
            chip("nvme", 0, &["temp"]),
            chip("nvme#2", 1, &["temp"]),
            chip("nvme#3", 2, &["temp"]),
            chip("mt7925_phy0", 3, &["temp"]),
            chip("k10temp", 4, &["temp"]),
            chip("r8169_0_b00:00", 5, &["temp"]),
            chip("r8169_0_c00:00", 6, &["temp"]),
            chip("spd5118", 8, &["temp"]),
            chip("spd5118#2", 9, &["temp"]),
        ],
        rapl: RaplState::RootOnly,
    }
}

#[derive(Clone, Debug)]
pub struct SensorsSynth {
    rng: XorShift,
    info_sent: bool,
}

fn named(key: &crate::key::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

impl SensorsSynth {
    pub fn new(seed: u64) -> SensorsSynth {
        SensorsSynth {
            rng: XorShift::new(seed.wrapping_add(0x0073_656e)),
            info_sent: false,
        }
    }

    /// Every reading at `at`: a slow sine per row plus seeded jitter.
    pub fn tick_at(&mut self, at: Ts) -> Batch {
        let t = at.as_secs_f64();
        let mut samples = Vec::with_capacity(ROWS.len() * 3 + 2);
        for (i, r) in ROWS.iter().enumerate() {
            let phase = i as f64 * 0.7;
            let v = r.base
                + r.swing * (0.5 + 0.5 * ((t / 90.0) * std::f64::consts::TAU + phase).sin())
                + (self.rng.f64() - 0.5) * 0.5;
            samples.push(Sample {
                id: named(&sensors::TEMP_C, r.label),
                datum: Datum::Scalar((v * 8.0).round() / 8.0),
            });
            if !self.info_sent {
                if let Some(m) = r.max {
                    samples.push(Sample {
                        id: named(&sensors::MAX_C, r.label),
                        datum: Datum::Scalar(m),
                    });
                }
                if let Some(c) = r.crit {
                    samples.push(Sample {
                        id: named(&sensors::CRIT_C, r.label),
                        datum: Datum::Scalar(c),
                    });
                }
            }
        }
        samples.push(Sample {
            id: sensors::WALK_MS.id.clone(),
            datum: Datum::Scalar(0.4 + self.rng.f64() * 0.2),
        });
        if !self.info_sent {
            self.info_sent = true;
            samples.push(Sample {
                id: sensors::INFO.id.clone(),
                datum: Datum::Record(Arc::new(sensors_info())),
            });
        }
        Batch {
            source: sensors::SOURCE,
            at,
            samples,
        }
    }
}

/// The sensors source's static info (§5): 1 s at every level — hwmon reads
/// are SMBus/SMN/NVMe-log transactions, never faster.
pub fn sensors_info_static() -> SourceInfo {
    SourceInfo {
        id: sensors::SOURCE,
        produces: &["sensor.*"],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(1)),
            visible: Duration::from_secs(1),
            focused: Duration::from_secs(1),
            always_on: false,
        },
        requires: &[],
    }
}

struct SensorsDemoSource {
    seed: u64,
}

impl Source for SensorsDemoSource {
    fn info(&self) -> SourceInfo {
        sensors_info_static()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = SensorsSynth::new(self.seed);
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from("synthetic (demo)")),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        });
        loop {
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            let level = cx.demand.level();
            let Some(cadence) = self.info().cadence.for_level(level) else {
                if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                    return;
                }
                continue;
            };
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            let at = cx.clock.now();
            let b = synth.tick_at(at);
            cx.emit(at, b.samples);
        }
    }
}

/// The seeded demo source for `--demo` and `SourceDef.demo` (§4.3).
pub fn sensors_demo(seed: u64) -> Box<dyn Source> {
    Box::new(SensorsDemoSource { seed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_in_range_and_the_inventory_once() {
        let (mut a, mut b) = (SensorsSynth::new(3), SensorsSynth::new(3));
        for i in 1..30 {
            let at = Ts(i * 1_000_000_000);
            let x = a.tick_at(at);
            let y = b.tick_at(at);
            assert_eq!(x.samples.len(), y.samples.len());
            for (p, q) in x.samples.iter().zip(&y.samples) {
                assert_eq!(p.id, q.id);
                if let (Datum::Scalar(u), Datum::Scalar(v)) = (&p.datum, &q.datum) {
                    assert_eq!(u, v);
                    if p.id.name == "sensor.temp_c" {
                        assert!((20.0..=95.0).contains(u), "{}: {u}", p.id.label);
                    }
                }
            }
            let infos = x
                .samples
                .iter()
                .filter(|s| s.id.name == "sensor.info")
                .count();
            assert_eq!(
                infos,
                usize::from(i == 1),
                "the inventory on the first tick only"
            );
        }
        assert_eq!(sensors_info().chips.len(), 9);
        assert_eq!(sensors_info().rapl, RaplState::RootOnly);
    }
}
