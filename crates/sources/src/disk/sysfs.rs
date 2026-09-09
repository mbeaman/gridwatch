//! What a block device *is*, from sysfs (brief arc 14 seam 4, D64 §3/§5).
//!
//! **A drive is a sysfs fact, not a name prefix.** `/sys/block/<dev>/device`
//! exists if and only if the device is real hardware: on torch exactly 3 of
//! 55 entries have it, and all 52 loop devices resolve under
//! `devices/virtual/block/`, as `dm-*`, `md*`, `ram*` and `zram*` would. So a
//! drive is a `/sys/block` entry with a `device` link, `hidden == 0` and
//! `size > 0` — `loop2` is `size = 0` on torch today, and `hidden = 1` is how
//! NVMe multipath would otherwise double-count a drive.
//!
//! htop's own rule (`DiskIOMeter.c` 3.4.1) is a name-prefix skip of `dm-`,
//! `zram` and partitions, which does not exclude `loop*` — on this machine
//! that sums 52 squashfs devices into "the disks". `PARITY.md` records the
//! difference as a deviation rather than copying it.
//!
//! **Classification is lazy, not timed** (D64 §5). Copying the sensors
//! source's 60 s rewalk would leave a USB drive invisible for up to a minute;
//! `/proc/diskstats` lists a new device the instant it appears, so a name is
//! classified the first time it is seen, cached, and dropped when the name
//! leaves diskstats. That costs one `readlink` plus a few small reads per
//! *new* name and nothing at all per tick.

use std::path::{Path, PathBuf};

use gridwatch_store::keys::disk::{DiskInfo, DiskKind};

/// sysfs `size` is in 512-byte sectors, exactly like diskstats — torch's
/// `nvme0n1` reads 7 814 037 168, which is 4.0 TB and not 7.8 GB.
const SECTOR_B: u64 = 512;

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u64(p: &Path) -> Option<u64> {
    read_trim(p)?.parse().ok()
}

/// The `/sys/block` entries, in name order.
pub fn blocks(sys: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(sys.join("block"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The device a block device hangs off: the `device` symlink's target name
/// (`nvme0` for an NVMe namespace, `0:0:0:0` for a SCSI disk). `None` is the
/// signal that there is no link at all — `canonicalize` **errors** on a
/// virtual device, which is the answer and not a failure to log (D64 trap 4).
///
/// A fixture tree cannot carry symlinks, so it stands a `device/` directory
/// in the link's place and names the controller in `device_name` — the
/// convention `fixtures/hwmon` already uses. That is detectable rather than
/// guessed at: a real target is never itself called `device`.
pub fn device_of(dir: &Path) -> Option<String> {
    let target = std::fs::canonicalize(dir.join("device")).ok()?;
    let name = target.file_name()?.to_string_lossy().into_owned();
    if name == "device" {
        return Some(read_trim(&dir.join("device_name")).unwrap_or(name));
    }
    Some(name)
}

/// `[none] mq-deadline` → `none`. The kernel marks the active scheduler with
/// brackets; a device with only one has no brackets at all.
fn scheduler_of(dir: &Path) -> String {
    let raw = read_trim(&dir.join("queue/scheduler")).unwrap_or_default();
    match (raw.find('['), raw.find(']')) {
        (Some(a), Some(b)) if b > a + 1 => raw[a + 1..b].to_string(),
        _ => raw,
    }
}

/// The partitions of a whole disk: subdirectories carrying a `partition`
/// file. Listed whether or not they are published as devices, because the
/// `full` pane names them.
fn partitions_of(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("partition").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Everything a whole disk's directory says about itself.
fn whole_disk(dir: &Path, name: &str, kind: DiskKind) -> DiskInfo {
    DiskInfo {
        name: name.to_string(),
        model: read_trim(&dir.join("device/model")).unwrap_or_default(),
        size_b: read_u64(&dir.join("size")).unwrap_or(0) * SECTOR_B,
        rotational: read_u64(&dir.join("queue/rotational")) == Some(1),
        removable: read_u64(&dir.join("removable")) == Some(1),
        kind,
        device: device_of(dir).unwrap_or_default(),
        partitions: partitions_of(dir),
        scheduler: scheduler_of(dir),
        nr_requests: read_u64(&dir.join("queue/nr_requests")).unwrap_or(0) as u32,
    }
}

/// Find the whole disk a partition belongs to, by looking for a directory
/// that contains it. A *scan* rather than trimming digits off the name: what
/// owns a partition is a sysfs fact too, and `nvme0n1p2` versus `sda1`
/// versus `mmcblk0p1` is three different name rules.
fn parent_of(sys: &Path, name: &str) -> Option<PathBuf> {
    blocks(sys)
        .into_iter()
        .map(|b| sys.join("block").join(b))
        .find(|d| d.join(name).join("partition").exists())
}

/// One device name's verdict: what it is, and what it says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub kind: DiskKind,
    pub info: DiskInfo,
}

/// Classify one diskstats name. Called once per name, ever (the source
/// caches the answer and drops it when the name leaves diskstats).
pub fn classify(sys: &Path, name: &str) -> Verdict {
    let dir = sys.join("block").join(name);
    if dir.is_dir() {
        // `/sys/block` lists whole disks only, so a partition never reaches
        // this branch and can never be admitted by accident.
        let real = device_of(&dir).is_some();
        let hidden = read_u64(&dir.join("hidden")).unwrap_or(0) != 0;
        let size = read_u64(&dir.join("size")).unwrap_or(0);
        let kind = if real && !hidden && size > 0 {
            DiskKind::Drive
        } else {
            DiskKind::Other
        };
        return Verdict {
            kind,
            info: whole_disk(&dir, name, kind),
        };
    }
    // Not a `/sys/block` entry: a partition of one, if any disk owns it.
    if let Some(parent) = parent_of(sys, name) {
        let own = parent.join(name);
        let mut info = whole_disk(&parent, name, DiskKind::Partition);
        info.size_b = read_u64(&own.join("size")).unwrap_or(0) * SECTOR_B;
        // A partition has no partitions; the model, scheduler, queue depth
        // and controller are its drive's, which is what the `full` pane
        // wants to say about it.
        info.partitions = Vec::new();
        return Verdict {
            kind: DiskKind::Partition,
            info,
        };
    }
    // In diskstats and nowhere in sysfs: a device that disappeared between
    // the two reads, or a namespace this kernel does not expose. Nameable by
    // `extra` and nothing else.
    Verdict {
        kind: DiskKind::Other,
        info: DiskInfo {
            name: name.to_string(),
            kind: DiskKind::Other,
            ..DiskInfo::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diskstats/torch/sys")
    }

    /// The fixture is torch's own `/sys/block`, recorded: 52 loop devices and
    /// three NVMe namespaces. Two entries are **hand-made** and named here,
    /// because the machine has neither and the rule's other two refusals
    /// would otherwise have no case: `md0` (a virtual whole disk with no
    /// `device` at all) and `nvme0c0n1` (an NVMe multipath node, `hidden`).
    const HAND_MADE: &[&str] = &["md0", "nvme0c0n1"];

    #[test]
    fn three_drives_of_torchs_fifty_five_entries() {
        let sys = tree();
        let all = blocks(&sys);
        assert_eq!(
            all.iter()
                .filter(|b| !HAND_MADE.contains(&b.as_str()))
                .count(),
            55,
            "torch's own /sys/block, recorded"
        );
        let drives: Vec<&String> = all
            .iter()
            .filter(|b| classify(&sys, b).kind == DiskKind::Drive)
            .collect();
        assert_eq!(
            drives,
            ["nvme0n1", "nvme1n1", "nvme2n1"],
            "3 of 55 — the number the whole device rule exists for (D64 §3)"
        );
    }

    #[test]
    fn the_three_refusals_each_have_a_reason() {
        let sys = tree();
        // No `device` link: 52 loop devices, and the hand-made md0.
        assert_eq!(classify(&sys, "loop0").kind, DiskKind::Other);
        assert!(device_of(&sys.join("block/loop0")).is_none());
        assert_eq!(classify(&sys, "md0").kind, DiskKind::Other);
        // A link, but no capacity: loop2 is `size = 0` on torch today.
        assert_eq!(
            std::fs::read_to_string(sys.join("block/loop2/size"))
                .unwrap()
                .trim(),
            "0"
        );
        assert_eq!(classify(&sys, "loop2").kind, DiskKind::Other);
        // A link and capacity, but hidden: NVMe multipath's second view of a
        // drive already counted.
        let hidden = classify(&sys, "nvme0c0n1");
        assert_eq!(hidden.kind, DiskKind::Other);
        assert_eq!(
            hidden.info.device, "nvme0",
            "it really is the same controller — which is why hidden matters"
        );
    }

    #[test]
    fn a_drive_says_what_it_is_and_a_partition_borrows_it() {
        let sys = tree();
        let d = classify(&sys, "nvme0n1").info;
        assert_eq!(d.model, "Samsung SSD 9100 PRO 4TB");
        // 7 814 037 168 **sectors**, so 4.0 TB — not 7.8 GB (the trap that
        // catches whoever forgets sysfs `size` is in 512-byte sectors too).
        assert_eq!(d.size_b, 7_814_037_168 * 512);
        assert!(d.size_b > 3_900_000_000_000);
        assert!(!d.rotational && !d.removable);
        assert_eq!(d.device, "nvme0", "the temperature join key");
        assert_eq!(d.scheduler, "none", "unbracketed from `[none] mq-deadline`");
        assert_eq!(d.nr_requests, 1023);
        assert_eq!(d.partitions, ["nvme0n1p1", "nvme0n1p2"]);

        // A partition is not a `/sys/block` entry, so it is classified by
        // who owns it, and takes its drive's identity with its own size.
        let p = classify(&sys, "nvme0n1p2");
        assert_eq!(p.kind, DiskKind::Partition);
        assert_eq!(p.info.model, "Samsung SSD 9100 PRO 4TB");
        assert_eq!(p.info.device, "nvme0");
        assert_eq!(p.info.nr_requests, 1023);
        assert!(p.info.partitions.is_empty());
        assert!(p.info.size_b > 0 && p.info.size_b < d.size_b);

        // A name in diskstats that sysfs has never heard of.
        let ghost = classify(&sys, "sdz");
        assert_eq!(ghost.kind, DiskKind::Other);
        assert_eq!(ghost.info.name, "sdz");
        assert_eq!(ghost.info.size_b, 0);
        assert!(blocks(Path::new("/nonexistent")).is_empty());
    }
}
