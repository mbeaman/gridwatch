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

/// A **partition**, published as a device in its own right (arc 16, D67 §4).
/// `nvme0n1p2` is named inside `nvme0n1`'s `DiskInfo.partitions` and, until
/// this arc, appeared nowhere else — which is how a bug that rendered every
/// partition as its parent drive stayed invisible to every snapshot, component
/// test and pty case. A partition only reaches the store under
/// `[sources.disk] partitions = true`; the synth publishes it always, because
/// a fixture exists to be the harder case.
pub const PARTITION: &str = "nvme0n1p2";

/// A **removable drive that leaves mid-run** (arc 16, D67 §4). Present for the
/// first 45 s of the synth's 180 s cycle and gone after it, so a replay crosses
/// the boundary in both directions. This is D61's risk row — a device that
/// vanishes is re-created from scratch after `max_age` with an empty chart —
/// and nothing modelled it.
pub const REMOVABLE: &str = "sda";
/// When `sda` is unplugged, in seconds into the cycle.
pub const REMOVABLE_LEAVES_S: f64 = 45.0;
/// The synth's cycle.
///
/// **Deliberately no longer `demo::net`'s 60 s** (arc 17). Arc 16 matched them
/// so a replay repeated as a whole, and arc 16's own review then measured that
/// the removable's 15 s absence could never reach the path it was added for:
/// `[store] history` clamps to a **60 s** floor, so the label is still in the
/// store — and still drawn — for the whole gap. Ninety-six consecutive frames
/// across two cycles showed `sda` present in every one.
///
/// The absence is now 135 s against that floor, so a device genuinely leaves
/// the store and D68's quiet path has a fixture. The cost is that a replay no
/// longer repeats on a single 60 s period; that was worth less than a fixture
/// which reaches the thing it exists for.
pub const CYCLE_S: f64 = 180.0;

/// Is the removable drive plugged in at `at`?
pub fn removable_present(at: Ts) -> bool {
    at.as_secs_f64() % CYCLE_S < REMOVABLE_LEAVES_S
}

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
        // The partition, as its own device (arc 16). `device` is its parent's
        // controller — a partition shares the drive's hwmon node, so the
        // temperature join finds the same chip and both rows show it.
        DiskInfo {
            name: PARTITION.into(),
            // **Its parent's model, not empty.** `sysfs::classify` builds a
            // partition with `whole_disk(&parent, …)` and overrides only
            // `size_b` and `partitions`, so the model, scheduler, queue depth
            // and controller are the drive's by construction — the real
            // source's own test asserts exactly this string. An empty model
            // was a shape it cannot produce, and it left `MODEL`, the disk
            // table's one *elastic* column, unexercised by the device type
            // this arc added (arc 16 review).
            model: "Samsung SSD 9100 PRO 4TB".into(),
            size_b: 3_000_000_000_000,
            rotational: false,
            removable: false,
            kind: DiskKind::Partition,
            device: "nvme0".into(),
            partitions: Vec::new(),
            scheduler: "none".into(),
            nr_requests: 1023,
        },
        // The removable, which leaves at 45 s. Flash, with a shallow queue, so the `BUSY`-versus-`Q` argument has a device where high
        // `BUSY` and low `Q` genuinely means "slow", not "barely awake".
        DiskInfo {
            name: REMOVABLE.into(),
            model: "SanDisk Extreme 55AE".into(),
            size_b: 128_043_712_512,
            rotational: false,
            removable: true,
            kind: DiskKind::Drive,
            device: "0:0:0:0".into(),
            partitions: vec!["sda1".into()],
            scheduler: "mq-deadline".into(),
            nr_requests: 64,
        },
    ]
}

/// What the source says about itself, counted from the inventory rather than
/// written down — the real `Selection::reason()` recomputes it every tick.
fn demo_reason() -> String {
    let drives = disk_infos()
        .iter()
        .filter(|i| !matches!(i.kind, DiskKind::Partition))
        .count();
    let parts = disk_infos().len() - drives;
    let mut s = format!("synthetic (demo) — {drives} drives");
    if parts > 0 {
        s.push_str(&format!(", {parts} partition"));
        if parts > 1 {
            s.push('s');
        }
    }
    s
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
    removable_was_present: Option<bool>,
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
            removable_was_present: None,
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

    /// The partition carries a share of its parent's write load and none of
    /// its reads — a mounted data partition under a drive whose reads are
    /// mostly elsewhere. It must never be a *copy* of `nvme0n1`'s draw: a bug
    /// that renders a partition as its parent is exactly what this device
    /// exists to catch, and identical numbers would hide it (arc 16).
    fn partition(&mut self, at: Ts) -> Draw {
        let t = at.as_secs_f64();
        let beat = 0.5 + 0.5 * ((t / 17.0) * std::f64::consts::TAU).sin();
        let write_bps = 1.1e6 + 7.0e6 * beat + self.rng.f64() * 3.0e4;
        Draw {
            read_bps: 0.0,
            write_bps: (write_bps / 4096.0).round() * 4096.0,
            discard_bps: 0.0,
            reads_ps: 0.0,
            writes_ps: (write_bps / 32_768.0).round(),
            busy_pct: t1(1.1 + 7.0 * beat),
            queue: t1(0.03 + 0.5 * beat),
            // Reads never complete here, so the read await is absent, not zero
            // — the same rule the idle drive proves, on a device that is busy.
            read_await_ms: None,
            write_await_ms: Some(t1(0.9 + 2.2 * beat)),
        }
    }

    /// The removable drive: a USB stick being written to, at USB 3 speeds and
    /// with the queue depth of something far slower than an NVMe.
    fn removable(&mut self, at: Ts) -> Draw {
        let t = at.as_secs_f64();
        let beat = 0.5 + 0.5 * ((t / 9.0) * std::f64::consts::TAU).cos();
        let write_bps = 2.0e7 + 8.0e7 * beat + self.rng.f64() * 5.0e5;
        Draw {
            read_bps: 0.0,
            write_bps: (write_bps / 4096.0).round() * 4096.0,
            discard_bps: 0.0,
            reads_ps: 0.0,
            writes_ps: (write_bps / 65_536.0).round(),
            busy_pct: t1(40.0 + 55.0 * beat),
            queue: t1(0.8 + 1.4 * beat),
            read_await_ms: None,
            write_await_ms: Some(t1(8.0 + 22.0 * beat)),
        }
    }

    pub fn tick_at(&mut self, at: Ts) -> Batch {
        // Drives first, then the partition, then the removable — the order the
        // real source publishes in (drives, partitions, `extra`), so the
        // fixture stays a thing that source could have produced.
        let mut feed: Vec<(&str, Draw)> = DEVICES
            .iter()
            .copied()
            .zip([self.busy(at), self.light(at), DiskSynth::idle()])
            .collect();
        // `sda` is a `Drive`, so it comes before the partition: the real
        // source publishes drives first, then partitions, each in name order
        // (`Sampler::select`). The first draft emitted the partition first
        // under a comment claiming to follow that order — the brief's own
        // trap 5, "a fixture that ignores that ordering tests a source that
        // does not exist" (arc 16 review).
        if removable_present(at) {
            feed.push((REMOVABLE, self.removable(at)));
        }
        feed.push((PARTITION, self.partition(at)));
        let mut samples = Vec::with_capacity(feed.len() * 9 + 4);
        for (dev, d) in feed {
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
        // `disk.info` is publish-once per device (D64 §5) — but a device that
        // leaves and comes back is a *new* first sight, and without its Record
        // the tile has rates for a device it cannot name. The real source
        // classifies lazily on seeing a name it has forgotten, which is
        // exactly this (arc 16).
        let present = removable_present(at);
        let returned = self.removable_was_present == Some(false) && present;
        self.removable_was_present = Some(present);
        if !self.info_sent || returned {
            self.info_sent = true;
            for info in disk_infos() {
                if info.name == REMOVABLE && !present {
                    continue;
                }
                if returned && info.name != REMOVABLE {
                    continue;
                }
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
            // Derived, not remembered: this said "3 drives" while the synth
            // published five devices, and the sources tile printed it
            // verbatim — the same defect as `scanned: 103` beside five rows,
            // in the same file, uncorrected by the commit that fixed the
            // other one (arc 16 review).
            reason: Some(Arc::from(demo_reason())),
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
            // The inventory goes out once — plus once more for the removable
            // on the tick it comes back, because a device that left and
            // returned is a *new* first sight and without its Record the tile
            // has rates for a device it cannot name (arc 16).
            let infos = x
                .samples
                .iter()
                .filter(|s| s.id.name == "disk.info")
                .count();
            let returned = !removable_present(Ts((i - 1) * 1_000_000_000)) && removable_present(at);
            let expect = if i == 1 {
                5 // three drives, a partition, the removable
            } else if returned {
                1 // the removable alone
            } else {
                0
            };
            assert_eq!(infos, expect, "disk.info at {i}s");
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

    /// A partition is published as a device, not merely named inside its
    /// parent's Record — the gap that let every partition render as its parent
    /// drive with no snapshot, component test or pty case noticing (arc 16).
    /// Its numbers must also *differ* from its parent's, or a tile that
    /// confused the two would still look right.
    #[test]
    fn the_partition_is_a_device_with_numbers_of_its_own() {
        let batch = DiskSynth::new(7).tick_at(Ts(3_000_000_000));
        let of = |dev: &str, key: &str| {
            batch
                .samples
                .iter()
                .find(|s| s.id.name == key && s.id.label == Label::Name(Arc::from(dev)))
                .and_then(|s| match s.datum {
                    Datum::Scalar(v) => Some(v),
                    _ => None,
                })
        };
        assert!(
            of(PARTITION, "disk.write_bps").is_some(),
            "the partition publishes its own series"
        );
        assert!(
            disk_infos()
                .iter()
                .any(|i| i.name == PARTITION && matches!(i.kind, DiskKind::Partition)),
            "and its own DiskInfo, as a Partition"
        );
        assert_ne!(
            of(PARTITION, "disk.write_bps"),
            of("nvme0n1", "disk.write_bps"),
            "a partition that copies its parent hides the bug it exists to catch"
        );
        // Reads never complete on it, so the read await is absent — the `—`
        // path on a device that is otherwise busy.
        assert!(of(PARTITION, "disk.read_await_ms").is_none());
    }

    /// A device that leaves mid-run: D61's risk row (re-created from scratch
    /// after `max_age` with an empty chart), modelled nowhere until arc 16.
    #[test]
    fn the_removable_leaves_and_comes_back() {
        let present = |s: u64| {
            DiskSynth::new(7)
                .tick_at(Ts(s * 1_000_000_000))
                .samples
                .iter()
                .any(|x| x.id.label == Label::Name(Arc::from(REMOVABLE)))
        };
        assert!(present(10), "plugged in early in the cycle");
        assert!(!present(50), "gone after it is unplugged");
        assert!(
            !present(100),
            "still gone a minute later — the point of arc 17"
        );
        assert!(present(190), "back on the next cycle");
        // The absence must outlast the retention floor, or the label never
        // leaves the store and the quiet path has no fixture (arc 16's review
        // measured exactly that failure).
        let absent = CYCLE_S - REMOVABLE_LEAVES_S;
        assert!(
            absent > 60.0,
            "the absence is {absent}s against a 60s `[store] history` floor"
        );
    }

    /// D64 §7: the join is a string equality between two Records that already
    /// exist. If either side is renamed, the demo stops exercising the join
    /// and no snapshot would notice — so it is asserted here, at the source.
    ///
    /// **It is not "every device has a chip", which is what this asserted
    /// before arc 16.** A USB drive has no `drivetemp` and its cell is `—`
    /// with the reason — D64's own degraded path, which the fixture now
    /// exercises with `sda`. The rule is: every *nvme* device joins, and at
    /// least one device deliberately does not, or the dash path has no fixture.
    #[test]
    fn every_demo_drive_has_a_matching_demo_hwmon_chip() {
        let chips = sensors_info();
        let mut unjoined = 0;
        for info in disk_infos() {
            let Some(chip) = chips.chips.iter().find(|c| c.device == info.device) else {
                assert!(
                    info.removable,
                    "`{}` hangs off `{}` and no demo hwmon chip matches — the join is \
                     silently not exercised for it. Only a removable may miss on purpose.",
                    info.name, info.device
                );
                unjoined += 1;
                continue;
            };
            assert!(chip.name.starts_with("nvme"), "{chip:?}");
        }
        assert_eq!(
            unjoined, 1,
            "exactly one device must fail the join, so the `—` path has a fixture"
        );
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
