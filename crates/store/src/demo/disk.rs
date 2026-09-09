//! Deterministic block-device synthesis (§12.5, brief arc 14 seam 2):
//! torch's three NVMe drives, one working hard, one ticking over and one
//! **completely idle**, so the `—` await path has data with no hardware.
//!
//! The `device` of each drive is `nvme0` / `nvme1` / `nvme2`, matching
//! [`crate::demo::sensors_info`] exactly — so the cross-source temperature
//! join (D64 §7) is exercised by `--demo`, by every snapshot and by CI on a
//! machine with no NVMe drive at all. That match is the point of this file
//! and not a coincidence; a test asserts it.
//!
//! Byte-deterministic per `(seed, Ts)` like the other synths. `tick_at` takes
//! **no `Detail`** on purpose: the disk source never raises one (D64 §8), and
//! a synth that structurally cannot vary by detail is the cheapest proof.

use std::sync::Arc;
use std::time::Duration;

use crate::demo::XorShift;
use crate::key::{Datum, Label, MetricId};
use crate::keys::disk::{self, DiskInfo, DiskKind};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

fn named(key: &crate::key::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

/// The three drives, in the order the synth publishes them.
pub const DEVICES: [&str; 3] = ["nvme0n1", "nvme1n1", "nvme2n1"];

/// The inventory the synth publishes (also the journal exemplar).
pub fn disk_infos() -> Vec<DiskInfo> {
    vec![
        DiskInfo {
            name: "nvme0n1".into(),
            model: "Samsung SSD 9100 PRO 4TB".into(),
            size_b: 7_814_037_168 * 512,
            rotational: false,
            removable: false,
            kind: DiskKind::Drive,
            device: "nvme0".into(),
            partitions: vec!["nvme0n1p1".into(), "nvme0n1p2".into()],
            scheduler: "none".into(),
            nr_requests: 1023,
        },
        DiskInfo {
            name: "nvme1n1".into(),
            model: "WD_BLACK SN850X 2000GB".into(),
            size_b: 3_907_029_168 * 512,
            rotational: false,
            removable: false,
            kind: DiskKind::Drive,
            device: "nvme1".into(),
            partitions: vec!["nvme1n1p1".into(), "nvme1n1p2".into()],
            scheduler: "none".into(),
            nr_requests: 1023,
        },
        DiskInfo {
            name: "nvme2n1".into(),
            model: "Samsung SSD 990 PRO 1TB".into(),
            size_b: 1_953_525_168 * 512,
            rotational: false,
            removable: false,
            kind: DiskKind::Drive,
            device: "nvme2".into(),
            partitions: vec![
                "nvme2n1p1".into(),
                "nvme2n1p2".into(),
                "nvme2n1p3".into(),
                "nvme2n1p4".into(),
                "nvme2n1p5".into(),
            ],
            scheduler: "none".into(),
            nr_requests: 1023,
        },
    ]
}

/// The Record the journal round-trip test uses.
pub fn disk_info_exemplar() -> DiskInfo {
    disk_infos().remove(0)
}

/// One drive's numbers at an instant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Draw {
    read_bps: f64,
    write_bps: f64,
    discard_bps: f64,
    reads_ps: f64,
    writes_ps: f64,
    busy_pct: f64,
    queue: f64,
    /// `None` when nothing completed on this tick — the `—` await path.
    read_await_ms: Option<f64>,
    write_await_ms: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct DiskSynth {
    rng: XorShift,
    info_sent: bool,
}

/// Round to a tenth, so a snapshot pins a number a person would read.
fn t1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl DiskSynth {
    pub fn new(seed: u64) -> DiskSynth {
        DiskSynth {
            rng: XorShift::new(seed.wrapping_add(0x0064_7368)),
            info_sent: false,
        }
    }

    /// The busy drive: a slow swell of a mixed read/write load with a queue
    /// that rises with it, so `BUSY` is high while `Q` says how much of the
    /// drive that actually is (D64 §6).
    fn busy(&mut self, at: Ts) -> Draw {
        let t = at.as_secs_f64();
        let swell = 0.5 + 0.5 * ((t / 40.0) * std::f64::consts::TAU).sin();
        let read_bps = 2.4e8 + 1.5e9 * swell + self.rng.f64() * 8.0e6;
        let write_bps = 3.0e7 + 3.4e8 * swell + self.rng.f64() * 2.0e6;
        let reads_ps = (read_bps / 131_072.0).round();
        let writes_ps = (write_bps / 65_536.0).round();
        Draw {
            read_bps: (read_bps / 4096.0).round() * 4096.0,
            write_bps: (write_bps / 4096.0).round() * 4096.0,
            discard_bps: if (t % 30.0) < 2.0 { 5.24288e8 } else { 0.0 },
            reads_ps,
            writes_ps,
            busy_pct: t1(18.0 + 76.0 * swell),
            queue: t1(1.6 + 38.0 * swell + self.rng.f64() * 0.4),
            read_await_ms: Some(t1(0.22 + 1.1 * swell)),
            write_await_ms: Some(t1(1.4 + 8.0 * swell)),
        }
    }

    /// The second drive: a steady trickle — a journal flush and a log write,
    /// the shape a workstation's data drive actually has.
    fn light(&mut self, at: Ts) -> Draw {
        let t = at.as_secs_f64();
        let beat = 0.5 + 0.5 * ((t / 11.0) * std::f64::consts::TAU).cos();
        let write_bps = 4.0e5 + 2.6e6 * beat + self.rng.f64() * 1.0e4;
        let read_bps = 6.0e4 + 9.0e5 * beat;
        Draw {
            read_bps: (read_bps / 4096.0).round() * 4096.0,
            write_bps: (write_bps / 4096.0).round() * 4096.0,
            discard_bps: 0.0,
            reads_ps: (read_bps / 65_536.0).round(),
            writes_ps: (write_bps / 32_768.0).round(),
            busy_pct: t1(0.4 + 3.0 * beat),
            queue: t1(0.02 + 0.3 * beat),
            read_await_ms: Some(t1(0.18 + 0.4 * beat)),
            write_await_ms: Some(t1(0.6 + 1.4 * beat)),
        }
    }

    /// The third drive is **idle**: every rate is an honest zero and nothing
    /// completes, so neither await is ever published and the tile has to draw
    /// `—` rather than a fabricated `0.0 ms`.
    fn idle() -> Draw {
        Draw::default()
    }

    pub fn tick_at(&mut self, at: Ts) -> Batch {
        let draws = [self.busy(at), self.light(at), DiskSynth::idle()];
        let mut samples = Vec::with_capacity(DEVICES.len() * 9 + 4);
        for (dev, d) in DEVICES.iter().zip(draws) {
            for (key, v) in [
                (&disk::READ_BPS, d.read_bps),
                (&disk::WRITE_BPS, d.write_bps),
                (&disk::DISCARD_BPS, d.discard_bps),
                (&disk::READS_PS, d.reads_ps),
                (&disk::WRITES_PS, d.writes_ps),
                (&disk::BUSY_PCT, d.busy_pct),
                (&disk::QUEUE, d.queue),
            ] {
                samples.push(Sample {
                    id: named(key, dev),
                    datum: Datum::Scalar(v),
                });
            }
            // The awaits are the one pair that is *absent* rather than zero
            // when nothing happened (D64 trap 7).
            for (key, v) in [
                (&disk::READ_AWAIT_MS, d.read_await_ms),
                (&disk::WRITE_AWAIT_MS, d.write_await_ms),
            ] {
                if let Some(v) = v {
                    samples.push(Sample {
                        id: named(key, dev),
                        datum: Datum::Scalar(v),
                    });
                }
            }
        }
        samples.push(Sample {
            id: disk::SCAN_MS.id.clone(),
            datum: Datum::Scalar(t1(0.2 + self.rng.f64() * 0.2)),
        });
        if !self.info_sent {
            self.info_sent = true;
            for info in disk_infos() {
                samples.push(Sample {
                    id: MetricId {
                        name: disk::INFO.id.name,
                        label: Label::Name(Arc::from(info.name.as_str())),
                    },
                    datum: Datum::Record(Arc::new(info)),
                });
            }
        }
        Batch {
            source: disk::SOURCE,
            at,
            samples,
        }
    }
}

/// The disk source's static info (§5): 2 s hidden, 1 s visible, 500 ms
/// focused. One `/proc/diskstats` read per tick serves every tier, so
/// `requires` is empty and **no tier ever raises `Detail`**.
pub fn disk_info() -> SourceInfo {
    SourceInfo {
        id: disk::SOURCE,
        produces: &["disk.*"],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(2)),
            visible: Duration::from_secs(1),
            focused: Duration::from_millis(500),
            always_on: false,
        },
        requires: &[],
    }
}

struct DiskDemoSource {
    seed: u64,
}

impl Source for DiskDemoSource {
    fn info(&self) -> SourceInfo {
        disk_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = DiskSynth::new(self.seed);
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from("synthetic (demo) — 3 drives")),
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
            let Some(cadence) = self.info().cadence.for_level(cx.demand.level()) else {
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

pub fn disk_demo(seed: u64) -> Box<dyn Source> {
    Box::new(DiskDemoSource { seed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::sensors_info;

    #[test]
    fn deterministic_with_one_busy_drive_and_one_that_never_completes() {
        let (mut a, mut b) = (DiskSynth::new(7), DiskSynth::new(7));
        for i in 1..=60 {
            let at = Ts(i * 1_000_000_000);
            let x = a.tick_at(at);
            let y = b.tick_at(at);
            assert_eq!(x.samples.len(), y.samples.len(), "deterministic at {i}s");
            for (p, q) in x.samples.iter().zip(&y.samples) {
                assert_eq!(p.id, q.id);
                if let (Datum::Scalar(u), Datum::Scalar(v)) = (&p.datum, &q.datum) {
                    assert_eq!(u, v);
                }
            }
            // The idle drive never publishes an await, on any tick.
            for s in &x.samples {
                if let Label::Name(n) = &s.id.label {
                    assert!(
                        !(n.as_ref() == "nvme2n1" && s.id.name.ends_with("await_ms")),
                        "the idle drive completed nothing: it must not publish {}",
                        s.id.name
                    );
                }
            }
            // …but it does publish a rate, every tick, so D61's sweep keeps
            // its label alive (trap 7).
            assert!(
                x.samples.iter().any(|s| s.id.name == "disk.read_bps"
                    && s.id.label == Label::Name(Arc::from("nvme2n1"))),
                "the idle drive still publishes 0.0"
            );
            // The inventory goes out once.
            let infos = x
                .samples
                .iter()
                .filter(|s| s.id.name == "disk.info")
                .count();
            assert_eq!(infos, if i == 1 { 3 } else { 0 });
        }
        // The busy drive is actually busy.
        let mut s = DiskSynth::new(7);
        let peak = (1..=60)
            .map(|i| {
                let batch = s.tick_at(Ts(i * 1_000_000_000));
                batch
                    .samples
                    .iter()
                    .filter(|x| {
                        x.id.name == "disk.busy_pct"
                            && x.id.label == Label::Name(Arc::from("nvme0n1"))
                    })
                    .filter_map(|x| match x.datum {
                        Datum::Scalar(v) => Some(v),
                        _ => None,
                    })
                    .fold(0.0_f64, f64::max)
            })
            .fold(0.0_f64, f64::max);
        assert!(peak > 80.0, "the busy drive peaked at {peak}%");
    }

    /// D64 §7: the join is a string equality between two Records that already
    /// exist. If either side is renamed, the demo stops exercising the join
    /// and no snapshot would notice — so it is asserted here, at the source.
    #[test]
    fn every_demo_drive_has_a_matching_demo_hwmon_chip() {
        let chips = sensors_info();
        for info in disk_infos() {
            let chip = chips
                .chips
                .iter()
                .find(|c| c.device == info.device)
                .unwrap_or_else(|| {
                    panic!(
                        "no demo hwmon chip hangs off `{}` — the temperature join is not \
                         exercised for {}",
                        info.device, info.name
                    )
                });
            assert!(chip.name.starts_with("nvme"), "{chip:?}");
        }
        // And the numbering deliberately disagrees, so a build that joined by
        // index instead of by `device` would draw the wrong drive's
        // temperature and this fixture would catch it.
        let by_device: Vec<&str> = chips
            .chips
            .iter()
            .filter(|c| c.device.starts_with("nvme"))
            .map(|c| c.path.as_str())
            .collect();
        assert_eq!(
            by_device,
            [
                "/sys/class/hwmon/hwmon1",
                "/sys/class/hwmon/hwmon2",
                "/sys/class/hwmon/hwmon0"
            ],
            "hwmon numbering must not agree with controller numbering"
        );
    }

    #[test]
    fn the_static_info_matches_the_cadence_table() {
        let i = disk_info();
        assert_eq!(i.id, disk::SOURCE);
        assert_eq!(i.cadence.hidden, Some(Duration::from_secs(2)));
        assert_eq!(i.cadence.visible, Duration::from_secs(1));
        assert_eq!(i.cadence.focused, Duration::from_millis(500));
        assert!(!i.cadence.always_on);
        assert!(i.requires.is_empty());
    }
}
