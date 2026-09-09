//! The disk source (§5 cadence row, §8, D64, brief arc 14 seam 3): one
//! `/proc/diskstats` read per tick (4 205 bytes / 64 lines on torch), rates
//! from the interval the sampler measured, and one lazy sysfs classification
//! per device name it has never seen.
//!
//! Everything it reads is a file any user may read; nothing here needs a
//! capability, and **no tier ever raises `Detail`** — one file read serves
//! every tier, `full` included, and per-process disk I/O is `/proc/<pid>/io`,
//! which htop's I/O screen already owns (§8.1). This is the first vertical
//! with that property and it is worth saying, because it is the cheapest
//! thing a review can check.
//!
//! This is `iostat`, not `df`. Filesystem capacity needs `/proc/self/
//! mountinfo` plus `statvfs`, and `statvfs` **blocks** on a stale NFS mount —
//! the same hazard as `getnameinfo` in arc 7. It has its own `BACKLOG.md`
//! entry and its own decision to come.

pub mod stat;
pub mod sysfs;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gridwatch_store::keys::disk::{self, DiskInfo, DiskKind, MAX_DEVICES};
use gridwatch_store::rules::glob;
use gridwatch_store::{
    Cadence, Datum, Label, Level, MetricId, OptionIssue, Sample, Source, SourceCtx, SourceInfo,
    SourceState, SourceStatus, demo,
};

use crate::options::Reader;

/// `[sources.disk]` (§9).
pub const OPTION_NAMES: &[&str] = &["refresh_ms", "partitions", "extra"];
pub const MIN_REFRESH: Duration = Duration::from_millis(250);
pub const MAX_REFRESH: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub refresh: Duration,
    /// Publish the partitions of every published drive as devices of their
    /// own. Off by default: nine more series a drive, for a number the whole
    /// disk already carries.
    pub partitions: bool,
    /// Globs for the devices the rule correctly refuses and someone
    /// legitimately wants: `dm-*` on LVM, `md0` on RAID, `zram0` on a laptop.
    pub extra: Vec<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            refresh: Duration::from_secs(1),
            partitions: false,
            extra: Vec::new(),
        }
    }
}

impl Options {
    pub fn from_table(t: &toml::Table) -> (Options, Vec<OptionIssue>) {
        let mut r = Reader::new("disk", t);
        let o = Options::read(&mut r);
        (o, r.finish())
    }

    /// Read in `OPTION_NAMES` order — the tripwire below pins it, and the
    /// order is what `config check` prints the issues in.
    fn read(r: &mut Reader) -> Options {
        let mut o = Options::default();
        let d = Options::default();
        let ms = MIN_REFRESH.as_millis() as i64..=MAX_REFRESH.as_millis() as i64;
        if let Some(n) = r.int_ms("refresh_ms", ms, "", d.refresh.as_millis() as i64) {
            o.refresh = Duration::from_millis(n as u64);
        }
        if let Some(b) = r.bool("partitions", d.partitions) {
            o.partitions = b;
        }
        // An empty `extra` is a value — "nothing beyond the rule" — so it is
        // accepted in silence, like `[sources.net] probes`.
        if let Some(list) = r.str_list("extra", true, &[]) {
            o.extra = list;
        }
        o
    }
}

/// The reader `start` runs, without starting anything (§4.3).
pub fn check(t: &toml::Table) -> Vec<OptionIssue> {
    Options::from_table(t).1
}

/// The roots the sampler reads (the net source's pattern, so tests point at
/// a fixture tree).
#[derive(Clone, Debug)]
pub struct Roots {
    pub proc: PathBuf,
    pub sys: PathBuf,
}

impl Default for Roots {
    fn default() -> Roots {
        Roots {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
        }
    }
}

/// Which devices this pass publishes, and what it refused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// In publish order: drives, then partitions, then `extra`.
    pub names: Vec<String>,
    pub drives: usize,
    pub partitions: usize,
    pub extra: usize,
    /// Devices the rule admitted that the 16-device cap left out.
    pub over_cap: usize,
}

impl Selection {
    /// The sentence `SourceStatus.reason` carries — what is published, and
    /// what was refused, because a warning in a log nobody reads is not a
    /// mechanism (D64 §4).
    pub fn reason(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (n, one, many) in [
            (self.drives, "drive", "drives"),
            (self.partitions, "partition", "partitions"),
            (self.extra, "extra", "extra"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        }
        if parts.is_empty() {
            parts.push("nothing".to_string());
        }
        let mut out = parts.join(", ");
        if self.over_cap > 0 {
            out.push_str(&format!(
                " — {} more refused by the {MAX_DEVICES}-device cap",
                self.over_cap
            ));
        }
        out
    }
}

fn named(key: &gridwatch_store::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

/// One pass: counters → rates, plus `disk.info` for whatever changed.
pub struct Sampler {
    pub roots: Roots,
    pub options: Options,
    prev: HashMap<String, (Instant, stat::Counters)>,
    /// One verdict per name, ever (D64 §5). Dropped when the name leaves
    /// diskstats, which also handles a name being reused by another device.
    seen: HashMap<String, sysfs::Verdict>,
    /// The last `disk.info` published per device, so an unchanged one is not
    /// republished every tick (D64 trap 6).
    sent: HashMap<String, DiskInfo>,
    /// Whether the last pass saw a published device with no discard group.
    no_discard: bool,
    pub scan_ms: f64,
}

impl Sampler {
    pub fn new(roots: Roots, options: Options) -> Sampler {
        Sampler {
            roots,
            options,
            prev: HashMap::new(),
            seen: HashMap::new(),
            sent: HashMap::new(),
            no_discard: false,
            scan_ms: 0.0,
        }
    }

    /// Is this name admitted, and as what? A drive always; a partition under
    /// `partitions = true`; anything at all when an `extra` glob names it.
    /// `extra` is an admission predicate over **every** class, so
    /// `extra = ["nvme0n1p1"]` with `partitions = false` is a partition that
    /// is published and fills the partition bucket — the cap's order stays
    /// defined whatever the config says.
    fn admits(&self, name: &str, kind: DiskKind) -> bool {
        if self.options.extra.iter().any(|p| glob(p, name)) {
            return true;
        }
        match kind {
            DiskKind::Drive => true,
            DiskKind::Partition => self.options.partitions,
            DiskKind::Other => false,
        }
    }

    /// The published set for this pass: drives first, then partitions, then
    /// `extra`, each in name order, capped at `MAX_DEVICES` (D64 §4). Nine
    /// scalars over all 64 diskstats lines would be ≈ 22 MB of store against
    /// P17's measured 40.3 MB of a 60 MB budget, so the cap is a mechanism
    /// and not a preference.
    fn select(&self, names: &BTreeMap<String, stat::Counters>) -> Selection {
        let mut buckets: [Vec<&str>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for name in names.keys() {
            let Some(v) = self.seen.get(name) else {
                continue;
            };
            if !self.admits(name, v.kind) {
                continue;
            }
            buckets[match v.kind {
                DiskKind::Drive => 0,
                DiskKind::Partition => 1,
                DiskKind::Other => 2,
            }]
            .push(name);
        }
        let mut sel = Selection::default();
        let admitted: usize = buckets.iter().map(Vec::len).sum();
        for (i, bucket) in buckets.iter().enumerate() {
            for name in bucket {
                if sel.names.len() == MAX_DEVICES {
                    break;
                }
                sel.names.push((*name).to_string());
                match i {
                    0 => sel.drives += 1,
                    1 => sel.partitions += 1,
                    _ => sel.extra += 1,
                }
            }
        }
        sel.over_cap = admitted - sel.names.len();
        sel
    }

    /// One pass over `/proc/diskstats`: every published device's rates, the
    /// `disk.info` Records that changed, and the pass's own wall ms.
    pub fn sample(&mut self, now: Instant) -> (Vec<Sample>, Selection) {
        let t0 = Instant::now();
        let counters = stat::read(&self.roots.proc);
        // Classify what is new, forget what is gone. `/proc/diskstats` lists
        // a device the instant it appears, so this is the whole discovery
        // mechanism — there is no rewalk timer.
        for name in counters.keys() {
            if !self.seen.contains_key(name) {
                self.seen
                    .insert(name.clone(), sysfs::classify(&self.roots.sys, name));
            }
        }
        self.seen.retain(|k, _| counters.contains_key(k));
        self.prev.retain(|k, _| counters.contains_key(k));
        self.sent.retain(|k, _| counters.contains_key(k));

        let sel = self.select(&counters);
        let mut out = Vec::with_capacity(sel.names.len() * 9 + 2);
        for name in &sel.names {
            let c = &counters[name];
            if let Some((then, prev)) = self.prev.get(name) {
                let r = c.rates(prev, now.saturating_duration_since(*then));
                // Seven scalars every tick, whatever happened: an idle drive
                // publishing 0.0 is a fact, and it is also what keeps its
                // label alive for D61's sweep (D64 trap 7).
                for (key, v) in [
                    (&disk::READ_BPS, r.read_bps),
                    (&disk::WRITE_BPS, r.write_bps),
                    (&disk::READS_PS, r.reads_ps),
                    (&disk::WRITES_PS, r.writes_ps),
                    (&disk::BUSY_PCT, r.busy_pct),
                    (&disk::QUEUE, r.queue),
                ] {
                    out.push(Sample {
                        id: named(key, name),
                        datum: Datum::Scalar(v),
                    });
                }
                // …and three that are absent rather than zero when the
                // kernel or the tick cannot say.
                for (key, v) in [
                    (&disk::DISCARD_BPS, r.discard_bps),
                    (&disk::READ_AWAIT_MS, r.read_await_ms),
                    (&disk::WRITE_AWAIT_MS, r.write_await_ms),
                ] {
                    if let Some(v) = v {
                        out.push(Sample {
                            id: named(key, name),
                            datum: Datum::Scalar(v),
                        });
                    }
                }
            }
            if let Some(v) = self.seen.get(name)
                && self.sent.get(name) != Some(&v.info)
            {
                self.sent.insert(name.clone(), v.info.clone());
                out.push(Sample {
                    id: MetricId {
                        name: disk::INFO.id.name,
                        label: Label::Name(Arc::from(name.as_str())),
                    },
                    datum: Datum::Record(Arc::new(v.info.clone())),
                });
            }
        }
        for (name, c) in &counters {
            self.prev.insert(name.clone(), (now, *c));
        }
        self.scan_ms = t0.elapsed().as_secs_f64() * 1000.0;
        out.push(Sample {
            id: disk::SCAN_MS.id.clone(),
            datum: Datum::Scalar((self.scan_ms * 100.0).round() / 100.0),
        });
        // A pre-4.18 kernel has no discard group at all; the status says so,
        // rather than leaving a key silently missing.
        self.no_discard = sel
            .names
            .iter()
            .any(|n| counters[n].discard_sectors.is_none());
        (out, sel)
    }

    /// Whether any published device's diskstats line had no discard group.
    pub fn no_discard(&self) -> bool {
        self.no_discard
    }
}

pub struct DiskSource {
    options: Options,
}

impl DiskSource {
    pub fn new(options: &toml::Table) -> DiskSource {
        DiskSource {
            options: Options::from_table(options).0,
        }
    }
}

impl Source for DiskSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: Cadence {
                hidden: Some(self.options.refresh.max(Duration::from_secs(2))),
                visible: self.options.refresh,
                focused: (self.options.refresh / 2).max(Duration::from_millis(250)),
                always_on: false,
            },
            ..demo::disk_info()
        }
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let cadence = self.info().cadence;
        let mut sampler = Sampler::new(Roots::default(), self.options.clone());
        let mut last: Option<(SourceState, String)> = None;
        let mut set_status =
            |cx: &SourceCtx, state: SourceState, reason: &str, hint: Option<&str>| {
                let key = (state, reason.to_string());
                if last.as_ref() != Some(&key) {
                    last = Some(key);
                    cx.status(SourceStatus {
                        state,
                        reason: Some(Arc::from(reason)),
                        hint: hint.map(Arc::from),
                        since: cx.clock.now(),
                        last_sample: None,
                        dropped: 0,
                        restarts: cx.restarts,
                    });
                }
            };
        set_status(&cx, SourceState::Starting, "starting", None);
        let mut first = true;
        loop {
            if !first {
                let level = cx.demand.level();
                let Some(period) = cadence.for_level(level) else {
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
            let now = Instant::now();
            let at = cx.clock.now();
            let (samples, sel) = sampler.sample(now);
            if !sampler.roots.proc.join("diskstats").exists() {
                set_status(
                    &cx,
                    SourceState::Unavailable,
                    "no /proc/diskstats",
                    Some("this kernel exposes no block-device statistics"),
                );
            } else if sel.names.is_empty() {
                set_status(
                    &cx,
                    SourceState::Unavailable,
                    "no block devices",
                    Some(
                        "nothing has a /sys/block device link — name what you want in \
                          [sources.disk] extra (dm-*, md*, zram0)",
                    ),
                );
            } else {
                let mut reason = sel.reason();
                if sampler.no_discard() {
                    reason.push_str(" · no discard group (kernel older than 4.18)");
                }
                set_status(&cx, SourceState::Ok, &reason, None);
            }
            if !samples.is_empty() {
                cx.emit(at, samples);
            }
        }
    }
}

pub fn start(options: &toml::Table) -> Box<dyn Source> {
    crate::options::log("disk", &check(options));
    Box::new(DiskSource::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::IssueKind;

    fn fixture_roots() -> Roots {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diskstats/torch");
        Roots {
            proc: base.join("proc"),
            sys: base.join("sys"),
        }
    }

    fn sampler(options: Options) -> Sampler {
        Sampler::new(fixture_roots(), options)
    }

    /// D63 trap 3: the ask set is the accepted set, in order.
    #[test]
    fn the_reader_asks_for_exactly_the_accepted_set() {
        let t = toml::Table::new();
        let mut r = Reader::new("disk", &t);
        let o = Options::read(&mut r);
        assert_eq!(r.asked(), OPTION_NAMES);
        assert_eq!(o, Options::default());
        assert!(r.finish().is_empty());
    }

    #[test]
    fn a_wrong_type_keeps_the_default_and_names_it() {
        for (text, want) in [
            (
                "refresh_ms = \"1000\"",
                "`refresh_ms` expects an integer (milliseconds), found a string (\"1000\") \
                 — the default 1000 stands",
            ),
            (
                "partitions = \"yes\"",
                "`partitions` expects a boolean, found a string (\"yes\") — \
                 the default false stands",
            ),
            (
                "extra = \"dm-*\"",
                "`extra` expects a list of strings, found a string (\"dm-*\") — \
                 the default [] stands",
            ),
            (
                "extra = [\"dm-*\", 5]",
                "`extra` expects a list of strings, found 5 at [1] — the default [] stands",
            ),
        ] {
            let t: toml::Table = toml::from_str(text).unwrap();
            let (kind, got) = crate::options::only_issue(check(&t));
            assert_eq!(kind, IssueKind::Rejected, "{text}");
            assert_eq!(got, want, "{text}");
            assert_eq!(Options::from_table(&t).0, Options::default(), "{text}");
        }
        // A positive value out of range is used after clamping.
        let t: toml::Table = toml::from_str("refresh_ms = 30").unwrap();
        let (o, issues) = Options::from_table(&t);
        assert_eq!(o.refresh, MIN_REFRESH);
        assert_eq!(issues[0].kind, IssueKind::Adjusted);
        assert_eq!(
            issues[0].text,
            "`refresh_ms` = 30 clamped to 250 (accepts 250-10000)"
        );
        // "Nothing beyond the rule" is a value.
        let t: toml::Table = toml::from_str("extra = []").unwrap();
        let (o, issues) = Options::from_table(&t);
        assert!(issues.is_empty(), "{issues:?}");
        assert!(o.extra.is_empty());
    }

    /// The acceptance row, against torch's recorded tree: three drives, and
    /// not one of the 52 loop devices.
    #[test]
    fn the_default_publishes_torchs_three_drives_and_no_loop_device() {
        let mut s = sampler(Options::default());
        let t0 = Instant::now();
        let (first, sel) = s.sample(t0);
        assert_eq!(sel.names, ["nvme0n1", "nvme1n1", "nvme2n1"]);
        assert_eq!(sel.reason(), "3 drives");
        assert_eq!(sel.over_cap, 0);
        // A rate needs two samples; the inventory does not.
        assert!(
            !first.iter().any(|x| x.id.name == "disk.read_bps"),
            "no rate on the first tick"
        );
        assert_eq!(
            first.iter().filter(|x| x.id.name == "disk.info").count(),
            3,
            "`disk.info` goes out on first sight"
        );
        assert!(first.iter().any(|x| x.id.name == "disk.scan_ms"));

        let (second, _) = s.sample(t0 + Duration::from_secs(1));
        let names: Vec<&str> = second.iter().map(|x| x.id.name).collect();
        for want in [
            "disk.read_bps",
            "disk.write_bps",
            "disk.discard_bps",
            "disk.reads_ps",
            "disk.writes_ps",
            "disk.busy_pct",
            "disk.queue",
        ] {
            assert!(names.contains(&want), "{want} is missing: {names:?}");
        }
        assert!(
            !names.contains(&"disk.info"),
            "unchanged info is published once, not every tick (D64 trap 6)"
        );
        // The fixture is one still file read twice, so every rate is zero —
        // the honest answer, and the awaits are absent rather than 0.0.
        for x in &second {
            if let Datum::Scalar(v) = x.datum
                && x.id.name != "disk.scan_ms"
            {
                assert_eq!(v, 0.0, "{}", x.id.name);
            }
        }
        assert!(!names.contains(&"disk.read_await_ms"));
        assert!(!s.no_discard(), "torch's kernel has the discard group");
        // Nothing loop-shaped, at either tick.
        for x in first.iter().chain(&second) {
            if let Label::Name(n) = &x.id.label {
                assert!(!n.starts_with("loop"), "a loop device reached the store");
            }
        }
    }

    /// `partitions = true` adds torch's nine partitions and stays inside the
    /// cap; `extra` opts a refused class back in.
    #[test]
    fn partitions_and_extra_are_admitted_in_a_defined_order() {
        let mut s = sampler(Options {
            partitions: true,
            ..Options::default()
        });
        let (_, sel) = s.sample(Instant::now());
        assert_eq!(sel.drives, 3);
        assert_eq!(sel.partitions, 9, "torch's nine partitions");
        assert_eq!(sel.reason(), "3 drives, 9 partitions");
        assert!(sel.names.len() <= MAX_DEVICES);
        assert_eq!(&sel.names[..3], ["nvme0n1", "nvme1n1", "nvme2n1"]);

        // `extra` admits the virtual device the rule correctly refused.
        let mut s = sampler(Options {
            extra: vec!["md*".into()],
            ..Options::default()
        });
        let (_, sel) = s.sample(Instant::now());
        assert_eq!(sel.names, ["nvme0n1", "nvme1n1", "nvme2n1"]);
        assert_eq!(sel.extra, 0, "md0 is not in torch's diskstats");

        // …and one partition by name, with `partitions` still off: it is
        // published, and it fills the *partition* bucket, so the cap's fill
        // order is defined whatever the config says.
        let mut s = sampler(Options {
            extra: vec!["nvme0n1p1".into()],
            ..Options::default()
        });
        let (_, sel) = s.sample(Instant::now());
        assert_eq!(sel.drives, 3);
        assert_eq!(sel.partitions, 1);
        assert_eq!(sel.names, ["nvme0n1", "nvme1n1", "nvme2n1", "nvme0n1p1"]);
    }

    /// The acceptance row for the cap: `extra = ["loop*"]` publishes 16 and
    /// says how many it refused, with the **drives kept** — the fill order is
    /// what makes the cap safe rather than arbitrary.
    #[test]
    fn the_cap_drops_extras_before_partitions_and_partitions_before_drives() {
        let mut s = sampler(Options {
            partitions: true,
            extra: vec!["loop*".into()],
            ..Options::default()
        });
        let (samples, sel) = s.sample(Instant::now());
        assert_eq!(sel.names.len(), MAX_DEVICES);
        assert_eq!(sel.drives, 3, "every drive survives");
        assert_eq!(sel.partitions, 9, "then every partition");
        assert_eq!(sel.extra, 4, "and the loop devices take what is left");
        // 3 + 9 + 52 admitted, 16 published.
        assert_eq!(sel.over_cap, 48);
        assert_eq!(
            sel.reason(),
            "3 drives, 9 partitions, 4 extra — 48 more refused by the 16-device cap"
        );
        // The loops that did get in are the first four by name, not whatever
        // order the kernel listed them in.
        assert_eq!(&sel.names[12..], ["loop0", "loop1", "loop10", "loop11"]);
        let infos = samples.iter().filter(|x| x.id.name == "disk.info").count();
        assert_eq!(infos, MAX_DEVICES);
    }

    /// D64 §5: a device is classified once, and forgotten when its name
    /// leaves diskstats — which is also what handles a name being reused.
    #[test]
    fn a_device_that_vanishes_stops_being_remembered() {
        let dir = std::env::temp_dir().join(format!("gw-disk-vanish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proc")).unwrap();
        let line = |name: &str| format!("   8       0 {name} 1 0 8 1 0 0 0 0 0 1 1 0 0 0 0 0 0\n");
        let write = |text: String| std::fs::write(dir.join("proc/diskstats"), text).unwrap();
        write(format!("{}{}", line("sda"), line("sdb")));
        let mut s = Sampler::new(
            Roots {
                proc: dir.join("proc"),
                sys: fixture_roots().sys,
            },
            Options {
                extra: vec!["sd*".into()],
                ..Options::default()
            },
        );
        let (_, sel) = s.sample(Instant::now());
        assert_eq!(sel.names, ["sda", "sdb"]);
        assert_eq!(s.seen.len(), 2);
        assert_eq!(s.prev.len(), 2);
        assert_eq!(s.sent.len(), 2);
        write(line("sda"));
        let (_, sel) = s.sample(Instant::now());
        assert_eq!(sel.names, ["sda"]);
        assert_eq!(s.seen.len(), 1, "the verdict went with the name");
        assert_eq!(s.prev.len(), 1, "and so did its counters");
        assert_eq!(s.sent.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pre-4.18 kernel: every device is still published, `disk.discard_bps`
    /// is not, and the status says why.
    #[test]
    fn a_kernel_with_no_discard_group_loses_one_key_and_no_device() {
        let roots = fixture_roots();
        let dir = std::env::temp_dir().join(format!("gw-disk-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(roots.proc.join("diskstats-pre-4.18"), dir.join("diskstats")).unwrap();
        let mut s = Sampler::new(
            Roots {
                proc: dir.clone(),
                sys: roots.sys,
            },
            Options {
                extra: vec!["*".into()],
                ..Options::default()
            },
        );
        let t0 = Instant::now();
        let (_, sel) = s.sample(t0);
        assert_eq!(sel.names, ["dm-0", "sda", "sda1"]);
        assert!(s.no_discard());
        let (second, _) = s.sample(t0 + Duration::from_secs(1));
        let names: Vec<&str> = second.iter().map(|x| x.id.name).collect();
        assert!(names.contains(&"disk.read_bps"));
        assert!(
            !names.contains(&"disk.discard_bps"),
            "absent, never a fabricated 0.0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live machine, at the default: whatever this box has, the source
    /// publishes only what the rule admits and costs what P23 says.
    #[test]
    fn the_live_pass_publishes_only_real_drives() {
        let mut s = Sampler::new(Roots::default(), Options::default());
        let t0 = Instant::now();
        let (_, sel) = s.sample(t0);
        let (_, sel2) = s.sample(t0 + Duration::from_millis(50));
        assert_eq!(
            sel.names, sel2.names,
            "the verdict is cached, not re-walked"
        );
        for name in &sel.names {
            assert!(
                !name.starts_with("loop") && !name.starts_with("dm-"),
                "{name} is not a drive"
            );
        }
        // P23's ceiling is 1 ms; this asserts only that the pass is *bounded*
        // — a debug build is several times a release one, and the recorded
        // number comes from `disk.scan_ms` in release (docs/PERFORMANCE.md).
        assert!(s.scan_ms < 50.0, "the pass took {} ms", s.scan_ms);
    }

    #[test]
    fn the_status_sentence_says_what_was_refused() {
        assert_eq!(Selection::default().reason(), "nothing");
        let one = Selection {
            names: vec!["nvme0n1".into()],
            drives: 1,
            ..Selection::default()
        };
        assert_eq!(one.reason(), "1 drive");
        let capped = Selection {
            drives: 3,
            extra: 13,
            over_cap: 39,
            ..Selection::default()
        };
        assert_eq!(
            capped.reason(),
            "3 drives, 13 extra — 39 more refused by the 16-device cap"
        );
    }

    /// The cadence §5's table promises, and the one the options move.
    #[test]
    fn the_cadence_follows_refresh_ms() {
        let c = DiskSource::new(&toml::Table::new()).info().cadence;
        assert_eq!(c.hidden, Some(Duration::from_secs(2)));
        assert_eq!(c.visible, Duration::from_secs(1));
        assert_eq!(c.focused, Duration::from_millis(500));
        let t: toml::Table = toml::from_str("refresh_ms = 4000").unwrap();
        let c = DiskSource::new(&t).info().cadence;
        assert_eq!(c.hidden, Some(Duration::from_secs(4)));
        assert_eq!(c.visible, Duration::from_secs(4));
        assert_eq!(c.focused, Duration::from_secs(2));
        // The focused floor: half of 250 ms is not 125 ms.
        let t: toml::Table = toml::from_str("refresh_ms = 250").unwrap();
        let c = DiskSource::new(&t).info().cadence;
        assert_eq!(c.focused, Duration::from_millis(250));
        assert_eq!(c.hidden, Some(Duration::from_secs(2)));
    }
}

/// The live evidence P23 and the arc-14 acceptance rows are read from: what
/// this machine publishes at the default, with `partitions = true`, and with
/// `extra = ["loop*"]`, plus the release cost of a pass.
///
/// `cargo test --release -p gridwatch-sources --features disk -- --ignored --nocapture live_disk_pass`
#[cfg(test)]
#[test]
#[ignore = "diagnostic; prints what the live machine publishes and what a pass costs"]
fn live_disk_pass() {
    use std::time::Instant;
    for (what, options) in [
        ("default", Options::default()),
        (
            "partitions = true",
            Options {
                partitions: true,
                ..Options::default()
            },
        ),
        (
            "extra = [\"loop*\"]",
            Options {
                extra: vec!["loop*".into()],
                ..Options::default()
            },
        ),
    ] {
        let mut s = Sampler::new(Roots::default(), options);
        let t0 = Instant::now();
        let (_, sel) = s.sample(t0);
        let cold = s.scan_ms;
        std::thread::sleep(Duration::from_millis(200));
        let (samples, _) = s.sample(Instant::now());
        println!(
            "[sources.disk] {what}\n  reason: {}\n  devices ({}): {}\n  scan_ms: {cold:.3} cold, \
             {:.3} warm\n  samples/tick: {}",
            sel.reason(),
            sel.names.len(),
            sel.names.join(" "),
            s.scan_ms,
            samples.len()
        );
    }
}
