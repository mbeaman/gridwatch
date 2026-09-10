//! The disk tile (§8, D64, brief arc 14 seam 6): what the drives are doing,
//! from an 8×3 rate pair to the zoom-only `full` with the per-drive pane.
//!
//! **`BUSY`, never `%util`** (D64 §6). Field 13 of `/proc/diskstats` is what
//! every other tool prints as utilisation, and on an NVMe drive it means
//! almost nothing: it is the share of wall time the queue was non-empty, so
//! one outstanding I/O reads 100 % while a thousand slots sit free. Torch
//! proves it from its own counters — `nvme0n1`'s queue was non-empty for
//! about 1 % of its uptime while holding a mean of about 1.9 I/Os, i.e.
//! roughly 170 in flight whenever it was busy at all, against
//! `nr_requests = 1023`. So the column is `BUSY`, `Q` (the mean in flight) is
//! drawn beside it wherever the width allows, and the `full` pane says what
//! `busy` cannot tell you rather than leaving that in a decision file.
//!
//! The temperature is the **sensors** source's, joined by device: hwmon is
//! never read twice. The component reads nothing but the store — there is no
//! `std::fs` in this crate.
//!
//! This is `iostat`, not `df`: filesystem capacity is a different question
//! with its own backlog entry.

mod view;

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::disk::{self, DiskInfo};
use gridwatch_store::keys::sensors;
use gridwatch_store::{Detail, KeyCode, KeyEvent, Label, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, InputCx, KeyHint, Manifest,
    Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub static MANIFEST: Manifest = Manifest {
    kind: "disk",
    name: "disks",
    summary: "per-device read/write rates, BUSY with the queue depth beside it, service times and the drive's temperature",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 4, h: 2 },
    requires: &[],
    optional: &[],
    sources: &[disk::SOURCE],
    // The temperature comes from here and is never re-read from hwmon
    // (D64 §7). Listing it costs nothing: the sensors source runs at 1 s at
    // every level regardless of who is watching.
    optional_sources: &[sensors::SOURCE],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "s",
            does: "sort",
        },
        KeyHint {
            key: "1-4",
            does: "chart series",
        },
        KeyHint {
            key: "↑/↓",
            does: "scroll",
        },
    ],
    example_options: "options = { devices = [\"nvme*\"], sort = \"traffic\" }",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "rates",
        min: Size::new(8, 3),
        adds: &["the read/write pair over the drives shown", "a busy chip"],
        zoom_only: false,
    },
    Tier {
        name: "sparks",
        min: Size::new(20, 5),
        adds: &["read and write sparklines", "the busiest drive's name"],
        zoom_only: false,
    },
    Tier {
        name: "table",
        min: Size::new(36, 8),
        adds: &[
            "one row per drive: DEVICE READ WRITE BUSY",
            "widening to °C R/S W/S Q and the model",
        ],
        zoom_only: false,
    },
    Tier {
        name: "chart",
        min: Size::new(56, 14),
        adds: &["a braille chart, one line per drive", "the await pair"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "the per-drive pane: model, size, scheduler, queue depth",
            "the controller and its hwmon chip",
            "what busy cannot tell you",
        ],
        zoom_only: true,
    },
];

pub const TIER_RATES: usize = 0;
pub const TIER_SPARKS: usize = 1;
pub const TIER_TABLE: usize = 2;
pub const TIER_CHART: usize = 3;
pub const TIER_FULL: usize = 4;

/// How long a service time may be drawn after it was published, as a
/// multiple of the source's cadence (§11's existing constant). A drive doing
/// one or two I/Os a second completes nothing on most ticks, so a tile that
/// asked "did this arrive on the newest sample?" would strobe every second
/// (D64 trap 7). The cadence is **observed** — the gap between the two most
/// recent samples — because a component cannot read `[sources.disk]`.
pub const AWAIT_HOLD_TICKS: u32 = 3;
/// The cadence assumed until two samples have arrived, and the range an
/// observed one is trusted in (`[sources.disk] refresh_ms`'s own range).
pub const DEFAULT_CADENCE: Duration = Duration::from_secs(1);
const MIN_CADENCE: Duration = Duration::from_millis(250);
const MAX_CADENCE: Duration = Duration::from_secs(10);

/// The project's one glob rule (`store::rules::glob`, D57 amendment 9).
pub use gridwatch_store::rules::glob;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Busiest first (read + write).
    #[default]
    Traffic,
    Name,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Traffic => Sort::Name,
            Sort::Name => Sort::Traffic,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sort::Traffic => "traffic",
            Sort::Name => "name",
        }
    }
}

/// Which metric the `chart` tier draws, one line per shown drive.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SeriesKind {
    #[default]
    Read,
    Write,
    Busy,
    Queue,
}

impl SeriesKind {
    /// The keys `1`–`4` select, in order.
    pub const ALL: [SeriesKind; 4] = [
        SeriesKind::Read,
        SeriesKind::Write,
        SeriesKind::Busy,
        SeriesKind::Queue,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SeriesKind::Read => "read",
            SeriesKind::Write => "write",
            SeriesKind::Busy => "busy",
            SeriesKind::Queue => "queue",
        }
    }

    pub fn key(self) -> gridwatch_store::Key<f64> {
        match self {
            SeriesKind::Read => disk::READ_BPS,
            SeriesKind::Write => disk::WRITE_BPS,
            SeriesKind::Busy => disk::BUSY_PCT,
            SeriesKind::Queue => disk::QUEUE,
        }
    }
}

/// View-only instance options (§9). None of these names is a
/// `[sources.disk]` name, and none of `[sources.disk]`'s is one of these:
/// §9's disjointness test walks the pair and adds nothing to its exemption
/// list.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Globs to show; empty means every device the source publishes.
    pub devices: Vec<String>,
    pub hide: Vec<String>,
    pub sort: Sort,
    pub series: SeriesKind,
}

pub const OPTION_NAMES: &[&str] = &["devices", "hide", "sort", "series"];

/// Why a drive has no temperature — the `full` pane says which (D64 §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoTemp {
    /// A reading is present; nothing to explain.
    #[default]
    None,
    /// The build has no sensors feature, or the source has published nothing.
    NoSource,
    /// No hwmon chip hangs off this drive's controller — a SATA drive with
    /// no `drivetemp` module, most often.
    NoChip,
    /// The chip is there and exports no temperature.
    NoReading,
}

impl NoTemp {
    pub fn why(self) -> &'static str {
        match self {
            NoTemp::None => "",
            NoTemp::NoSource => "no sensors source: nothing publishes hwmon readings",
            NoTemp::NoChip => {
                "no hwmon chip hangs off this controller (a SATA drive needs drivetemp)"
            }
            NoTemp::NoReading => "the chip exports no temperature",
        }
    }
}

/// One device as the tile sees it. Every number is a rate the source
/// measured; the component derives nothing from a counter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drive {
    pub name: String,
    pub read_bps: f64,
    pub write_bps: f64,
    /// `None` on a kernel older than 4.18, whose diskstats has no discard
    /// group — the source refuses to publish a fabricated `0.0` there, and so
    /// must the tile, or the pane prints `discard 0B/s` for a number nobody
    /// measured (arc 14 review).
    pub discard_bps: Option<f64>,
    pub reads_ps: f64,
    pub writes_ps: f64,
    pub busy_pct: f64,
    pub queue: f64,
    /// The last service time and when it arrived; `None` when the drive has
    /// completed nothing since the run began.
    pub read_await: Option<(Ts, f64)>,
    pub write_await: Option<(Ts, f64)>,
    pub info: Option<DiskInfo>,
    pub temp_c: Option<f64>,
    pub crit_c: Option<f64>,
    /// The hwmon reading the temperature came from (`nvme#2:Composite`), for
    /// the `full` pane.
    pub chip: Option<String>,
    pub no_temp: NoTemp,
}

impl Drive {
    pub fn total(&self) -> f64 {
        self.read_bps + self.write_bps
    }

    /// The service time to draw, or `None` for `—`: the last one, while it
    /// is younger than `AWAIT_HOLD_TICKS` of the source's cadence.
    pub fn await_ms(&self, which: SeriesKind, now: Ts, hold: Duration) -> Option<f64> {
        let (at, v) = match which {
            SeriesKind::Write => self.write_await?,
            _ => self.read_await?,
        };
        (now.0.saturating_sub(at.0) <= hold.as_nanos() as u64).then_some(v)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub drives: Vec<Drive>,
    /// Every device the source publishes, before the tile's filter.
    pub all: usize,
}

impl Model {
    pub fn refresh(&mut self, store: &gridwatch_store::Store, options: &Options, sort: Sort) {
        let labels: Vec<Label> = store.labels(disk::READ_BPS.id.name).cloned().collect();
        // The chip inventory, read once per rebuild rather than per drive.
        let chips = store.record(&sensors::INFO).map(|(_, i)| i.clone());
        let temp_labels: Vec<String> = if chips.is_some() {
            store
                .labels(sensors::TEMP_C.id.name)
                .map(gridwatch_store::rules::label_text)
                .collect()
        } else {
            Vec::new()
        };
        self.drives.clear();
        self.all = labels.len();
        for l in labels {
            let Label::Name(name) = &l else { continue };
            let dev = name.to_string();
            if !options.devices.is_empty() && !options.devices.iter().any(|p| glob(p, &dev)) {
                continue;
            }
            if options.hide.iter().any(|p| glob(p, &dev)) {
                continue;
            }
            let last = |k: &gridwatch_store::Key<f64>| {
                store.last(&k.named(name)).map(|(_, v)| v).unwrap_or(0.0)
            };
            let stamped = |k: &gridwatch_store::Key<f64>| store.last(&k.named(name));
            let mut d = Drive {
                read_bps: last(&disk::READ_BPS),
                write_bps: last(&disk::WRITE_BPS),
                discard_bps: store.last(&disk::DISCARD_BPS.named(name)).map(|(_, v)| v),
                reads_ps: last(&disk::READS_PS),
                writes_ps: last(&disk::WRITES_PS),
                busy_pct: last(&disk::BUSY_PCT),
                queue: last(&disk::QUEUE),
                read_await: stamped(&disk::READ_AWAIT_MS),
                write_await: stamped(&disk::WRITE_AWAIT_MS),
                info: store
                    .record(&disk::INFO.named(name))
                    .map(|(_, i)| i.clone()),
                name: dev,
                ..Drive::default()
            };
            join_temperature(&mut d, store, chips.as_ref(), &temp_labels);
            self.drives.push(d);
        }
        match sort {
            Sort::Traffic => self.drives.sort_by(|a, b| {
                b.total()
                    .total_cmp(&a.total())
                    .then(b.busy_pct.total_cmp(&a.busy_pct))
                    .then(a.name.cmp(&b.name))
            }),
            Sort::Name => self.drives.sort_by(|a, b| a.name.cmp(&b.name)),
        }
    }

    /// The drives' rates, summed — what the tile shows, and nothing else
    /// (D64 §2: there is no published total, because one would change
    /// meaning under `extra` and disagree with this).
    pub fn totals(&self) -> (f64, f64) {
        self.drives
            .iter()
            .fold((0.0, 0.0), |(r, w), d| (r + d.read_bps, w + d.write_bps))
    }

    /// The drive the small tiers name: the busiest by traffic, else by
    /// `busy_pct`, else the first shown.
    pub fn busiest(&self) -> Option<&Drive> {
        self.drives
            .iter()
            .max_by(|a, b| {
                a.total()
                    .total_cmp(&b.total())
                    .then(a.busy_pct.total_cmp(&b.busy_pct))
            })
            .filter(|d| d.total() > 0.0 || d.busy_pct > 0.0)
            .or_else(|| self.drives.first())
    }

    pub fn hidden(&self) -> usize {
        self.all.saturating_sub(self.drives.len())
    }
}

/// The cross-source join (D64 §7), the arc's one genuinely new mechanism:
/// `disk.info{nvme0n1}.device` is `nvme0`, and so is the `ChipInfo.device` of
/// the hwmon node for that controller. **Never by hwmon index or by chip
/// suffix** — on torch hwmon0/1/2 are nvme1/nvme2/nvme0, and any agreement
/// between the two numberings is a coincidence of devpath sorting.
fn join_temperature(
    d: &mut Drive,
    store: &gridwatch_store::Store,
    chips: Option<&sensors::SensorsInfo>,
    temp_labels: &[String],
) {
    let Some(chips) = chips else {
        d.no_temp = NoTemp::NoSource;
        return;
    };
    let Some(device) = d
        .info
        .as_ref()
        .map(|i| i.device.as_str())
        .filter(|s| !s.is_empty())
    else {
        d.no_temp = NoTemp::NoChip;
        return;
    };
    let Some(chip) = chips.chips.iter().find(|c| c.device == device) else {
        d.no_temp = NoTemp::NoChip;
        return;
    };
    // `Composite` is the whole-controller reading every NVMe drive exports;
    // anything else falls back to the chip's first temperature. Two
    // namespaces of one controller therefore share a reading, which is
    // correct — the `full` pane says the number is the controller's.
    let prefix = format!("{}:", chip.name);
    let composite = format!("{prefix}Composite");
    let label = temp_labels
        .iter()
        .find(|l| **l == composite)
        .or_else(|| temp_labels.iter().find(|l| l.starts_with(&prefix)));
    let Some(label) = label else {
        d.no_temp = NoTemp::NoReading;
        return;
    };
    let key = std::sync::Arc::from(label.as_str());
    match store.last(&sensors::TEMP_C.named(&key)) {
        Some((_, v)) => {
            d.temp_c = Some(v);
            d.crit_c = store.last(&sensors::CRIT_C.named(&key)).map(|(_, v)| v);
            d.chip = Some(label.clone());
        }
        None => d.no_temp = NoTemp::NoReading,
    }
}

pub struct Disk {
    options: Options,
    model: Model,
    sort: Sort,
    series: SeriesKind,
    scroll: usize,
    seen: Option<Ts>,
    /// The gap between the two most recent samples: the source's cadence, as
    /// observed rather than configured.
    cadence: Duration,
}

impl Disk {
    pub fn new(options: Options) -> Disk {
        Disk {
            sort: options.sort,
            series: options.series,
            options,
            model: Model::default(),
            scroll: 0,
            seen: None,
            cadence: DEFAULT_CADENCE,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Disk, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Disk::new(parsed))
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn series(&self) -> SeriesKind {
        self.series
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// How long a service time may still be drawn (§11's 3 × cadence).
    pub fn await_hold(&self) -> Duration {
        self.cadence * AWAIT_HOLD_TICKS
    }

    fn rebuild(&mut self, store: &gridwatch_store::Store) {
        self.model.refresh(store, &self.options, self.sort);
    }
}

impl Default for Disk {
    fn default() -> Disk {
        Disk::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Disk::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build: Box::new(build),
};

impl Component for Disk {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("disks")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    /// One `/proc/diskstats` read serves every tier, `full` included, so
    /// there is nothing to gate: per-process disk I/O is `/proc/<pid>/io`,
    /// which htop's I/O screen already owns at `Detail::Columns` (D64 §8).
    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let Some(at) = cx.store.last_sample(disk::SOURCE) else {
            return Redraw::No;
        };
        if self.seen == Some(at) {
            return Redraw::No;
        }
        if let Some(prev) = self.seen {
            let gap = Duration::from_nanos(at.0.saturating_sub(prev.0));
            if (MIN_CADENCE..=MAX_CADENCE).contains(&gap) {
                self.cadence = gap;
            }
        }
        self.seen = Some(at);
        self.rebuild(cx.store);
        Redraw::Yes
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        match key.code {
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.rebuild(cx.store);
                Outcome::Consumed
            }
            KeyCode::Char(c @ '1'..='4') => {
                let i = c as usize - '1' as usize;
                self.series = SeriesKind::ALL[i];
                Outcome::Consumed
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Consumed
            }
            KeyCode::Down => {
                self.scroll = (self.scroll + 1).min(self.model.drives.len().saturating_sub(1));
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_RATES | TIER_SPARKS => &["rd"],
            TIER_TABLE => &["DEVICE", "BUSY"],
            TIER_CHART => &["series"],
            _ => &["model"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_the_series_keys() {
        let t: toml::Table = toml::from_str(r#"sort = "name""#).unwrap();
        assert_eq!(Disk::from_table(&t).unwrap().sort(), Sort::Name);
        let t: toml::Table = toml::from_str(r#"series = "queue""#).unwrap();
        assert_eq!(Disk::from_table(&t).unwrap().series(), SeriesKind::Queue);
        let t: toml::Table = toml::from_str("colour = 1").unwrap();
        assert!(Disk::from_table(&t).is_err(), "options are deny_unknown");
        assert_eq!(Sort::Traffic.next().name(), "name");
        // `1`-`4` and the option spell the same four series in the same
        // order, so the key bar and `config check` cannot disagree.
        assert_eq!(
            SeriesKind::ALL.map(SeriesKind::name),
            ["read", "write", "busy", "queue"]
        );
        assert_eq!(SeriesKind::Busy.key().id.name, "disk.busy_pct");
        assert_eq!(SeriesKind::Queue.key().id.name, "disk.queue");
    }

    /// Every reason is a sentence, and the one that means "there is nothing
    /// to explain" is empty.
    #[test]
    fn each_missing_temperature_says_which_of_the_three_it_is() {
        assert_eq!(NoTemp::None.why(), "");
        for n in [NoTemp::NoSource, NoTemp::NoChip, NoTemp::NoReading] {
            assert!(n.why().len() > 10, "{n:?}");
        }
    }
}
