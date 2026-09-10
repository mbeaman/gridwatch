//! `/proc/diskstats` (brief arc 14 seam 3): one line per block device, and
//! the rates two lines a second apart become.
//!
//! **The field numbers in the kernel's documentation are 1-based and include
//! `major`, `minor` and `name`**, so after splitting a line the stats start at
//! index 3 and documented field 13 (`io_ticks`) is index **12**. Read the
//! table as 0-based and every number is off by three, plausible and wrong.
//!
//! The layout has grown twice, and both older shapes are still out there:
//!
//! | kernel | stats | fields on the line | groups |
//! |---|---|---|---|
//! | < 4.18 | 11 | 14 | reads, writes, in-flight/ticks/queue |
//! | 4.18+ | 15 | 18 | + discards (4) |
//! | 5.5+ | 17 | 20 | + flushes (2) |
//!
//! So the guard is **per group**, not a single length: the core needs 14
//! fields, `discard_bps` needs the whole discard group (18), and anything a
//! future kernel appends is ignored. A short line degrades by publishing
//! fewer keys and **never** by skipping the device.
//!
//! Two fields are deliberately not read. Field 12 (`in_flight`) samples a
//! quantity that changes microsecond to microsecond, where field 14 gives the
//! time-weighted mean for free (D64 §6). The merge counts are in
//! `BACKLOG.md`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// Diskstats sectors are **always 512 bytes**, whatever the device's
/// `logical_block_size` says. A 4Kn drive reports 512-byte sectors here too.
pub const SECTOR_B: f64 = 512.0;

/// The fields the core rates need: major, minor, name and eleven stats.
const CORE_FIELDS: usize = 14;
/// …plus the whole discard group (4.18+): completed, merged, sectors, ms.
const DISCARD_FIELDS: usize = 18;

/// One device's counters at an instant. Every member is a monotonically
/// rising lifetime total; nothing here is ever published (D64 §1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Field 4 — completed reads.
    pub reads: u64,
    /// Field 6 — sectors read.
    pub read_sectors: u64,
    /// Field 7 — ms spent reading.
    pub read_ms: u64,
    /// Field 8 — completed writes.
    pub writes: u64,
    /// Field 10 — sectors written.
    pub write_sectors: u64,
    /// Field 11 — ms spent writing.
    pub write_ms: u64,
    /// Field 13 (index 12) — ms the queue was non-empty. What every other
    /// tool calls `%util`; gridwatch calls it `BUSY` and draws `queue` beside
    /// it (D64 §6).
    pub io_ticks: u64,
    /// Field 14 (index 13) — weighted ms doing I/O, i.e. Σ(in-flight × ms).
    /// Over wall time this is the mean queue depth, `iostat`'s `aqu-sz`.
    pub time_in_queue: u64,
    /// Field 17 (index 16), and `None` on a kernel with no discard group.
    pub discard_sectors: Option<u64>,
}

/// The per-second rates between two samples. `None` is "this tick cannot say"
/// and is never published; `0.0` is "nothing happened", which is a fact and
/// is (D64 trap 7).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub read_bps: f64,
    pub write_bps: f64,
    pub discard_bps: Option<f64>,
    pub reads_ps: f64,
    pub writes_ps: f64,
    pub busy_pct: f64,
    pub queue: f64,
    pub read_await_ms: Option<f64>,
    pub write_await_ms: Option<f64>,
}

/// The mean service time of a completed I/O over one interval, or `None` when
/// the interval cannot say: nothing completed, or the service-time counter
/// went backwards (a 32-bit wrap, or a device re-created on the same name).
/// Never `Some(0.0)` from a wrap — that would read as "instant" (D64 R1).
fn await_ms(completions: u64, now: u64, then: u64) -> Option<f64> {
    if completions == 0 || now < then {
        return None;
    }
    Some((now - then) as f64 / completions as f64)
}

impl Counters {
    /// Rates from the previous sample over `dt` — the interval the sampler
    /// *measured*, never the configured one (D57 amendment 1).
    ///
    /// A counter that went backwards means the device was re-created (a loop
    /// device re-attached, a USB drive re-plugged onto the same name): the
    /// delta is zero, never a negative or a number the size of the counter.
    pub fn rates(&self, prev: &Counters, dt: Duration) -> Rates {
        let secs = dt.as_secs_f64().max(1e-3);
        let ms = secs * 1000.0;
        let d = |now: u64, then: u64| now.saturating_sub(then);
        let per_s = |now: u64, then: u64| d(now, then) as f64 / secs;
        let reads = d(self.reads, prev.reads);
        let writes = d(self.writes, prev.writes);
        Rates {
            read_bps: per_s(self.read_sectors, prev.read_sectors) * SECTOR_B,
            write_bps: per_s(self.write_sectors, prev.write_sectors) * SECTOR_B,
            discard_bps: match (self.discard_sectors, prev.discard_sectors) {
                (Some(now), Some(then)) => Some(per_s(now, then) * SECTOR_B),
                _ => None,
            },
            reads_ps: reads as f64 / secs,
            writes_ps: writes as f64 / secs,
            // The share of wall time the queue was non-empty. Clamped,
            // because a device re-created mid-interval or a clock that
            // stepped can otherwise produce 130 %.
            busy_pct: (d(self.io_ticks, prev.io_ticks) as f64 / ms * 100.0).clamp(0.0, 100.0),
            // ms-of-I/O per ms of wall = the mean number in flight.
            queue: d(self.time_in_queue, prev.time_in_queue) as f64 / ms,
            // Published only on a tick that completed something *and* whose
            // service-time counter moved forward: a `0.0` await would read as
            // "instant", not "nothing happened". The backwards case is not
            // hypothetical — diskstats prints the service-time counters
            // truncated to 32 bits, so on a busy drive they wrap every few
            // days (D64 amendment 1: torch's `write_ms` was three days from
            // wrapping when this was written), and `saturating_sub` would
            // otherwise turn that tick into an impossibly fast one.
            read_await_ms: await_ms(reads, self.read_ms, prev.read_ms),
            write_await_ms: await_ms(writes, self.write_ms, prev.write_ms),
        }
    }
}

/// Parse the whole file: device name → counters, in name order.
///
/// A `BTreeMap` rather than a `HashMap` because the device rule's cap (D64
/// §4) publishes the first `MAX_DEVICES` of each class, and "first" has to
/// mean something a test can write down.
pub fn parse(text: &str) -> BTreeMap<String, Counters> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < CORE_FIELDS {
            continue;
        }
        let name = f[2];
        if name.is_empty() {
            continue;
        }
        // Index 0 and 1 are major and minor: a line whose name column is not
        // where it should be is junk, not a device.
        if f[0].parse::<u32>().is_err() || f[1].parse::<u32>().is_err() {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        out.insert(
            name.to_string(),
            Counters {
                reads: n(3),
                read_sectors: n(5),
                read_ms: n(6),
                writes: n(7),
                write_sectors: n(9),
                write_ms: n(10),
                io_ticks: n(12),
                time_in_queue: n(13),
                discard_sectors: (f.len() >= DISCARD_FIELDS).then(|| n(16)),
            },
        );
    }
    out
}

/// Read and parse `<proc>/diskstats`. An unreadable file is an empty map,
/// which the source reports as `Unavailable` rather than as no devices.
pub fn read(proc: &Path) -> BTreeMap<String, Counters> {
    std::fs::read_to_string(proc.join("diskstats"))
        .map(|t| parse(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/diskstats/torch/proc")
            .join(name);
        std::fs::read_to_string(p).expect("the fixture reads")
    }

    /// The 20-field parse, against torch's own file — and against the field
    /// numbers, which is the whole risk in this module.
    #[test]
    fn the_twenty_field_line_lands_on_the_right_fields() {
        let m = parse(&fixture("diskstats"));
        assert_eq!(m.len(), 64, "torch's file is 64 lines");
        let d = m["nvme0n1"];
        // Read straight off the recorded line:
        //   17894911 6285408 1627419539 733606591 5784941 17484780 747710516
        //   4011320838 0 2758549 466175438 208479 0 4795115808 15978696 …
        assert_eq!(d.reads, 17_894_911);
        assert_eq!(d.read_sectors, 1_627_419_539);
        assert_eq!(d.read_ms, 733_606_591);
        assert_eq!(d.writes, 5_784_941);
        assert_eq!(d.write_sectors, 747_710_516);
        assert_eq!(d.write_ms, 4_011_320_838);
        assert_eq!(d.io_ticks, 2_758_549, "field 13 is index 12, not index 9");
        assert_eq!(d.time_in_queue, 466_175_438, "field 14 is index 13");
        assert_eq!(d.discard_sectors, Some(4_795_115_808));
        // D64's own arithmetic, reproduced from the recorded counters: the
        // NOT a lifetime derivation: diskstats prints the service-time
        // counters truncated to 32 bits, and this fixture's own `nvme0n1`
        // has already wrapped — its four service-time accumulators sum to
        // more than 2**32 ms, so `time_in_queue` is that sum mod 2**32 and
        // any "mean in flight since boot" computed from it is an artifact
        // (D64 amendment 1, which retracted exactly that arithmetic from the
        // decision entry). What the fixture can honestly pin is the *wrap*:
        // the identity below is what proves the truncation, and it is the
        // reason every disk number gridwatch draws comes from an interval.
        let sum = u128::from(d.read_ms) + u128::from(d.write_ms);
        assert!(
            sum > u128::from(u32::MAX),
            "this fixture is the wrapped case; it stops being evidence if the \
             service-time sum ({sum}) no longer exceeds 2**32"
        );
        assert!(
            u128::from(d.time_in_queue) < u128::from(u32::MAX),
            "time_in_queue is printed truncated to 32 bits"
        );
        // The partitions are on the same file, so `partitions = true` costs
        // no extra read.
        assert!(m.contains_key("nvme0n1p2"));
        assert_eq!(m.keys().filter(|k| k.starts_with("loop")).count(), 52);
        // And every loop device's rates are **zero**: their counters are
        // cumulative mount-time reads from boot, not live traffic (D64).
        assert!(m["loop0"].reads > 0, "a lifetime counter, not a rate");
    }

    /// A service-time counter that went backwards means the interval cannot
    /// say what an I/O cost — a 32-bit wrap, or a device re-created on the
    /// same name. It must publish nothing, never `Some(0.0)`, which would
    /// read as "instant" (D64 amendment 1). Torch's `write_ms` was three days
    /// from wrapping when this was written, so it is not hypothetical.
    #[test]
    fn a_wrapped_service_time_counter_cannot_say_and_says_nothing() {
        let prev = Counters {
            reads: 10,
            writes: 10,
            read_ms: u64::from(u32::MAX) - 5,
            write_ms: u64::from(u32::MAX) - 5,
            ..Default::default()
        };
        // Both wrapped past 2**32 and restarted low.
        let now = Counters {
            reads: 20,
            writes: 20,
            read_ms: 7,
            write_ms: 7,
            ..prev
        };
        let r = now.rates(&prev, Duration::from_secs(1));
        assert_eq!(r.read_await_ms, None, "a wrap is not an instant read");
        assert_eq!(r.write_await_ms, None, "a wrap is not an instant write");
        // And the ordinary case still reports.
        let fine = Counters {
            reads: 20,
            read_ms: prev.read_ms + 40,
            ..prev
        };
        let r = fine.rates(&prev, Duration::from_secs(1));
        assert_eq!(r.read_await_ms, Some(4.0), "40 ms over 10 completions");
    }

    /// A pre-4.18 kernel: eleven stats, no discard group. The device must
    /// still be published, with one key fewer.
    #[test]
    fn a_short_line_degrades_by_one_key_and_is_never_skipped() {
        let m = parse(&fixture("diskstats-pre-4.18"));
        assert_eq!(m.len(), 3);
        let d = m["sda"];
        assert_eq!(d.reads, 12_045);
        assert_eq!(d.io_ticks, 9_880);
        assert_eq!(d.time_in_queue, 27_321);
        assert_eq!(d.discard_sectors, None, "there is no discard group to read");
        let later = Counters {
            read_sectors: d.read_sectors + 2048,
            ..d
        };
        let r = later.rates(&d, Duration::from_secs(1));
        assert_eq!(r.read_bps, 2048.0 * 512.0);
        assert_eq!(r.discard_bps, None, "absent, never a fabricated 0.0");
    }

    /// The 4.18 shape (discards, no flushes) and a future one (a longer tail
    /// nobody here understands): both parse, and the tail is ignored.
    #[test]
    fn the_discard_group_must_be_whole_and_a_longer_tail_is_ignored() {
        let stats = |n: usize| {
            let mut v: Vec<String> = vec!["259".into(), "0".into(), "nvme0n1".into()];
            v.extend((1..=n).map(|i| i.to_string()));
            v.join(" ")
        };
        // 15 stats = 18 fields: the 4.18 layout. Field 17 (index 16) is the
        // fourteenth stat, so `discard_sectors` is 14.
        let m = parse(&stats(15));
        assert_eq!(m["nvme0n1"].discard_sectors, Some(14));
        // 12 stats = 15 fields: the discard group has started but is not
        // whole. Reading index 16 there would read past the line, and
        // reading index 14 alone would call "discards completed" a sector
        // count — so the whole group is refused and the device is kept.
        let m = parse(&stats(12));
        assert_eq!(m["nvme0n1"].discard_sectors, None);
        assert_eq!(m["nvme0n1"].io_ticks, 10, "the core is unaffected");
        // 24 stats: a kernel that appended something. Everything known is
        // read; the tail is not guessed at.
        let m = parse(&stats(24));
        assert_eq!(m["nvme0n1"].io_ticks, 10);
        assert_eq!(m["nvme0n1"].time_in_queue, 11);
        assert_eq!(m["nvme0n1"].discard_sectors, Some(14));
        // 10 stats = 13 fields: below the core. The line is not a device.
        assert!(parse(&stats(10)).is_empty());
    }

    #[test]
    fn rates_are_per_second_and_a_reset_is_zero() {
        let prev = Counters {
            reads: 100,
            read_sectors: 2_000,
            read_ms: 40,
            writes: 50,
            write_sectors: 1_000,
            write_ms: 500,
            io_ticks: 1_000,
            time_in_queue: 4_000,
            discard_sectors: Some(10),
        };
        let now = Counters {
            reads: 300,
            read_sectors: 6_000,
            read_ms: 120,
            writes: 90,
            write_sectors: 3_000,
            write_ms: 900,
            io_ticks: 1_500,
            time_in_queue: 12_000,
            discard_sectors: Some(2_058),
        };
        let r = now.rates(&prev, Duration::from_secs(2));
        assert_eq!(r.read_bps, 4_000.0 / 2.0 * 512.0);
        assert_eq!(r.write_bps, 2_000.0 / 2.0 * 512.0);
        assert_eq!(r.discard_bps, Some(2_048.0 / 2.0 * 512.0));
        assert_eq!(r.reads_ps, 100.0);
        assert_eq!(r.writes_ps, 20.0);
        // 500 ms of a 2 000 ms interval.
        assert_eq!(r.busy_pct, 25.0);
        // 8 000 weighted ms over 2 000 ms of wall = four I/Os in flight,
        // while the drive was "busy" only a quarter of the time — the whole
        // point of drawing Q beside BUSY.
        assert_eq!(r.queue, 4.0);
        assert_eq!(r.read_await_ms, Some(80.0 / 200.0));
        assert_eq!(r.write_await_ms, Some(400.0 / 40.0));

        // A device re-created under the same name.
        let reset = Counters {
            reads: 3,
            read_sectors: 8,
            io_ticks: 1,
            // Same kernel, so the discard group is still there — it is the
            // *device* that restarted, not the layout of the file.
            discard_sectors: Some(0),
            ..Counters::default()
        };
        let r = reset.rates(&now, Duration::from_secs(1));
        assert_eq!(r.read_bps, 0.0, "a reset is not a negative rate");
        assert_eq!(r.busy_pct, 0.0);
        assert_eq!(r.queue, 0.0);
        assert_eq!(r.read_await_ms, None, "no completions to divide by");
        assert_eq!(r.discard_bps, Some(0.0));

        // A drive that completed nothing: rates are an honest zero, awaits
        // are absent (D64 trap 7).
        let r = prev.rates(&prev, Duration::from_secs(1));
        assert_eq!(r.read_bps, 0.0);
        assert_eq!(r.busy_pct, 0.0);
        assert_eq!(r.read_await_ms, None);
        assert_eq!(r.write_await_ms, None);

        // A zero interval cannot divide by zero, and `io_ticks` cannot
        // exceed the wall it is a share of.
        let r = now.rates(&prev, Duration::ZERO);
        assert!(r.read_bps.is_finite() && r.queue.is_finite());
        assert_eq!(r.busy_pct, 100.0, "clamped, not 50 000");
    }

    #[test]
    fn junk_lines_are_skipped_and_a_missing_file_is_empty() {
        assert!(parse("").is_empty());
        assert!(parse("not a device line at all\n").is_empty());
        assert!(
            parse("   x       0 sda 1 2 3 4 5 6 7 8 9 10 11\n").is_empty(),
            "a name where the major should be is junk"
        );
        assert!(read(Path::new("/nonexistent")).is_empty());
    }
}
