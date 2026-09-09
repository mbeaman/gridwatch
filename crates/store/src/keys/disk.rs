//! Block-device keys (§8, D64, brief arc 14 seam 1): the rates a
//! `/proc/diskstats` delta yields, per device.
//!
//! Every number here is a **rate over the interval the sampler measured**,
//! never a counter (D64 §1): `[[rules]]` compares a value to a threshold, so
//! `disk.write_bps{nvme0n1} > 500e6` is a rule someone can write and a
//! `Δcounter` is not; a pruned ring keeps only its newest point, so a
//! component diffing two stored counters would get nothing across a prune
//! boundary; and a 2 400-point ring of a monotonically rising counter is
//! 38 KB of a straight line. Lifetime totals live in SMART, which needs
//! `/dev/nvme*` — a capability this dashboard refuses.
//!
//! There are **no aggregate keys** (D64 §2). A `disk.total_read_bps` cannot
//! be given one honest meaning: under `[sources.disk] extra = ["loop*"]` a
//! "total" silently starts including loop traffic, and under a tile's
//! `devices` filter the published total and the drawn total disagree with
//! nothing on screen saying so. The tile sums what it shows; a rule on
//! `disk.write_bps{*}` raises **per device**, which is the alert anyone
//! wants.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, LabelSet, RecordValue, Unit};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("disk");

/// Bytes per second per `{dev}`: sectors × 512 over the measured interval.
/// Diskstats sectors are **always 512 bytes**, whatever `logical_block_size`
/// says.
pub const READ_BPS: Key<f64> = Key::new("disk.read_bps");
pub const WRITE_BPS: Key<f64> = Key::new("disk.write_bps");
/// Only published on a kernel whose diskstats carries the discard group
/// (4.18+); a pre-4.18 line degrades by leaving this out, never by dropping
/// the device.
pub const DISCARD_BPS: Key<f64> = Key::new("disk.discard_bps");
/// Completed I/Os per second per `{dev}` — `iostat`'s `r/s` and `w/s`.
pub const READS_PS: Key<f64> = Key::new("disk.reads_ps");
pub const WRITES_PS: Key<f64> = Key::new("disk.writes_ps");
/// The share of wall time the queue was non-empty (`io_ticks`), clamped
/// 0–100. **Never called `%util`** (D64 §6): on an NVMe drive with a
/// thousand queue slots, one outstanding I/O reads 100 %.
pub const BUSY_PCT: Key<f64> = Key::new("disk.busy_pct");
/// The mean number of I/Os in flight over the interval (`time_in_queue` over
/// wall) — `iostat`'s `aqu-sz`, and the honest companion to `busy_pct`.
pub const QUEUE: Key<f64> = Key::new("disk.queue");
/// Mean service time of a completed I/O, published **only on a tick with
/// completions** — a `0.0` would read as "instant" rather than "nothing
/// happened". §11's 3 × cadence rule decides how long a tile may draw the
/// last one.
pub const READ_AWAIT_MS: Key<f64> = Key::new("disk.read_await_ms");
pub const WRITE_AWAIT_MS: Key<f64> = Key::new("disk.write_await_ms");
/// What the device is, published **on change only** (D64 trap 6).
pub const INFO: Key<DiskInfo> = Key::new("disk.info");
/// The pass's wall ms (the `sources` tile's cost note, P23's evidence).
pub const SCAN_MS: Key<f64> = Key::new("disk.scan_ms");

/// The cap on published devices (D64 §4): nine scalars over all 64 diskstats
/// lines would be ≈ 22 MB of store against P17's measured 40.3 MB of a 60 MB
/// budget, so the filter is a mechanism rather than a tidiness preference and
/// gets a hard number. Filled drives first, then partitions, then `extra`.
pub const MAX_DEVICES: usize = 16;

/// What the device rule decided a name is (D64 §3). It is a **sysfs fact**,
/// not a name prefix: on torch exactly 3 of 55 `/sys/block` entries have a
/// `device` link, and all 52 loop devices resolve under
/// `devices/virtual/block/`, as `dm-*`, `md*`, `ram*` and `zram*` would.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    /// A `/sys/block` entry with a `device` link, `hidden == 0` and
    /// `size > 0` — real hardware.
    #[default]
    Drive,
    /// A diskstats name that is not a `/sys/block` entry: a partition of a
    /// drive. Published only under `[sources.disk] partitions = true` (or by
    /// an explicit `extra` glob).
    Partition,
    /// A whole-disk device the rule correctly refuses — `loop*`, `dm-*`,
    /// `md*`, `ram*`, `zram*`. Published only when an `extra` glob asks for
    /// it.
    Other,
}

impl DiskKind {
    pub fn name(self) -> &'static str {
        match self {
            DiskKind::Drive => "drive",
            DiskKind::Partition => "partition",
            DiskKind::Other => "other",
        }
    }
}

/// What a device is, from sysfs, once per name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskInfo {
    /// The diskstats name (`nvme0n1`, `nvme0n1p2`, `loop3`).
    pub name: String,
    /// `device/model` — the drive's own word for itself, or empty.
    pub model: String,
    /// Bytes. sysfs `size` is in **512-byte sectors** like diskstats, so this
    /// is that number × 512 (torch's `nvme0n1` is 7 814 037 168 → 4.0 TB).
    pub size_b: u64,
    pub rotational: bool,
    pub removable: bool,
    pub kind: DiskKind,
    /// The device the drive hangs off — the `device` link's target name
    /// (`nvme0`, `0:0:0:0`). **This is the join key** for the temperature:
    /// it equals `sensor.info`'s `ChipInfo.device` for the hwmon node of the
    /// same controller. Never join by hwmon index or chip suffix — on torch
    /// hwmon0/1/2 are nvme1/nvme2/nvme0. Empty for a device with no link.
    pub device: String,
    /// The drive's partitions, whether or not they are published as devices.
    pub partitions: Vec<String>,
    /// `queue/scheduler`, with the brackets stripped (`none`, `mq-deadline`).
    pub scheduler: String,
    /// `queue/nr_requests` — the denominator the `full` pane prints `queue`
    /// against (`q 172 of 1023`).
    pub nr_requests: u32,
}

fn decode_info(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<DiskInfo>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

macro_rules! scalar {
    ($name:expr, $unit:ident, $doc:expr) => {
        scalar!($name, $unit, $doc, Dynamic)
    };
    ($name:expr, $unit:ident, $doc:expr, $labels:ident) => {
        KeyMeta {
            name: $name,
            unit: Unit::$unit,
            kind: DatumKind::Scalar,
            source: SOURCE,
            doc: $doc,
            decode: None,
            labels: LabelSet::$labels,
        }
    };
}

pub static METAS: &[KeyMeta] = &[
    scalar!(
        "disk.read_bps",
        BytesPerSec,
        "read rate per {dev}, from /proc/diskstats sector deltas (always 512 B) over the measured interval"
    ),
    scalar!(
        "disk.write_bps",
        BytesPerSec,
        "write rate per {dev}, from /proc/diskstats sector deltas over the measured interval"
    ),
    scalar!(
        "disk.discard_bps",
        BytesPerSec,
        "discard/TRIM rate per {dev}; absent on a kernel older than 4.18, whose diskstats has no discard group"
    ),
    scalar!(
        "disk.reads_ps",
        Count,
        "completed reads per second per {dev} (iostat's r/s)"
    ),
    scalar!(
        "disk.writes_ps",
        Count,
        "completed writes per second per {dev} (iostat's w/s)"
    ),
    scalar!(
        "disk.busy_pct",
        Percent,
        "share of wall time the queue was non-empty per {dev} (io_ticks), clamped 0-100 — not saturation: one I/O outstanding on a 1023-deep NVMe queue reads 100 %, so it is drawn as BUSY with disk.queue beside it"
    ),
    scalar!(
        "disk.queue",
        Count,
        "mean I/Os in flight per {dev} over the interval (time_in_queue over wall) — iostat's aqu-sz, the honest companion to busy_pct"
    ),
    scalar!(
        "disk.read_await_ms",
        Milliseconds,
        "mean service time of a completed read per {dev}; published only on a tick with completions, never a 0.0 that means 'no data'"
    ),
    scalar!(
        "disk.write_await_ms",
        Milliseconds,
        "mean service time of a completed write per {dev}; published only on a tick with completions"
    ),
    scalar!(
        "disk.scan_ms",
        Milliseconds,
        "wall ms of the last /proc/diskstats pass (the sources tile's note, P23's evidence)",
        Static
    ),
    KeyMeta {
        name: "disk.info",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "what the device is per {dev}: model, size, rotational/removable, kind, the controller it hangs off (the temperature join key), its partitions, scheduler and nr_requests — published on change",
        decode: Some(decode_info),
        labels: LabelSet::Dynamic,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eleven_keys_and_what_a_kind_is_called() {
        assert_eq!(METAS.len(), 11, "§8 lists eleven disk keys");
        // Every labelled key is Dynamic (D61): a drive can be unplugged.
        for m in METAS {
            let expect = if m.name == "disk.scan_ms" {
                LabelSet::Static
            } else {
                LabelSet::Dynamic
            };
            assert_eq!(m.labels, expect, "{}", m.name);
            assert_eq!(m.source, SOURCE, "{}", m.name);
        }
        assert_eq!(DiskKind::Drive.name(), "drive");
        assert_eq!(DiskKind::Partition.name(), "partition");
        assert_eq!(DiskKind::Other.name(), "other");
        assert_eq!(DiskKind::default(), DiskKind::Drive);
        // `disk.queue` is a Count (a number of I/Os), never a Ratio: it is
        // 172 in flight, not 0.17 of something.
        let queue = METAS.iter().find(|m| m.name == "disk.queue").unwrap();
        assert_eq!(queue.unit, Unit::Count);
    }
}
