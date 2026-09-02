//! The sensors component (§8, brief arc 5 seam 7): every hwmon reading the
//! sensors source publishes, from the hottest chip in 8×3 to the zoom-only
//! `full` with fans, volts, power, the RAPL line, the PSI row and the GPU
//! row — the GPU's temp/fan/power come from the gpu source's keys (optional
//! source), never polled twice. Components never read sysfs: everything is
//! `sensor.*` in the store.

mod view;

use std::borrow::Cow;

use gridwatch_store::keys::{gpu, sensors};
use gridwatch_store::{Detail, KeyCode, KeyEvent, Label, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, InputCx, KeyHint, Manifest,
    Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub static MANIFEST: Manifest = Manifest {
    kind: "sensors",
    name: "sensors",
    summary: "every hwmon chip's temperatures, fans, volts and power, hottest first; RAPL and the GPU row",
    contract: 1,
    footprints: &[
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 1 },
        Footprint { w: 6, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 6, h: 1 },
    requires: &[],
    optional: &[
        gridwatch_store::Capability::Hwmon,
        gridwatch_store::Capability::Rapl,
    ],
    sources: &[sensors::SOURCE],
    optional_sources: &[gpu::SOURCE],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "↑/↓",
            does: "scroll",
        },
        KeyHint {
            key: "o",
            does: "sort hottest / by chip",
        },
    ],
    example_options: "options = { chips = [\"nvme*\", \"k10temp\"], sort = \"chip\" }",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "hottest",
        min: Size::new(8, 3),
        adds: &["the hottest reading with its chip", "▲ over max"],
        zoom_only: false,
    },
    Tier {
        name: "strip",
        min: Size::new(24, 4),
        adds: &["up to six chips as chips, hottest first"],
        zoom_only: false,
    },
    Tier {
        name: "table",
        min: Size::new(40, 8),
        adds: &["CHIP · SENSOR · VALUE · MAX · BAR", "scrolling"],
        zoom_only: false,
    },
    Tier {
        name: "chart",
        min: Size::new(60, 14),
        adds: &["braille chart of the four hottest over ten minutes"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "fans, volts, power",
            "the RAPL line",
            "the PSI row",
            "the gpu row",
        ],
        zoom_only: true,
    },
];

pub const TIER_HOTTEST: usize = 0;
pub const TIER_STRIP: usize = 1;
pub const TIER_TABLE: usize = 2;
pub const TIER_CHART: usize = 3;
pub const TIER_FULL: usize = 4;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Hottest,
    Chip,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Hottest => Sort::Chip,
            Sort::Chip => Sort::Hottest,
        }
    }
}

/// View-only instance options (§9): a chip filter (globs by name) and the
/// sort.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub chips: Vec<String>,
    pub sort: Sort,
}

pub const OPTION_NAMES: &[&str] = &["chips", "sort"];

/// A reading older than this is not shown: the store has no retraction, so
/// a removed NVMe would otherwise stay "hottest" for ever (review). Three
/// times the source's 1 s cadence, with room for a slow re-walk.
pub const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// One temperature reading as the tile sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub chip: String,
    pub label: String,
    /// `chip:label` — the store label.
    pub key: String,
    pub value: f64,
    pub max: Option<f64>,
    pub crit: Option<f64>,
}

/// The limit assumed for a chip that exports none, so a reading without a
/// threshold can still be ranked (review: k10temp exports no `max`, and
/// ranking by the margin alone hid the CPU behind every DIMM). AMD documents
/// Tctl's ceiling at 95 °C; anything else gets a conservative 100 °C.
pub fn assumed_limit(chip: &str, label: &str) -> f64 {
    if chip.starts_with("k10temp") || label.starts_with("Tctl") || label.starts_with("Tccd") {
        95.0
    } else {
        100.0
    }
}

impl Reading {
    /// The threshold this reading is judged against: the chip's own `max`,
    /// else the assumed one.
    pub fn limit(&self) -> f64 {
        self.max
            .filter(|m| m.is_finite() && *m > 0.0)
            .unwrap_or_else(|| assumed_limit(&self.chip, &self.label))
    }

    /// True when the limit is this crate's assumption, not the chip's.
    pub fn assumed(&self) -> bool {
        self.max.is_none()
    }

    /// How close to its limit, 0..: **the rank**. The column the table
    /// prints is this number, so the order is visible (review).
    pub fn heat(&self) -> f64 {
        self.value / self.limit()
    }

    /// Over the chip's `max`; `crit` when it exports one and the value is
    /// past it.
    pub fn over_max(&self) -> bool {
        self.max.is_some_and(|m| self.value >= m)
    }

    pub fn over_crit(&self) -> bool {
        self.crit.is_some_and(|c| self.value >= c)
    }

    /// The margin to its limit in °C (shown; no longer the sort key).
    pub fn margin(&self) -> f64 {
        self.limit() - self.value
    }

    /// The bar's fraction: the same `heat` the order uses, capped at 1.5.
    pub fn frac(&self) -> f32 {
        self.heat().clamp(0.0, 1.5) as f32
    }
}

/// A non-temperature reading (fan / volt / power) for `full`.
#[derive(Clone, Debug, PartialEq)]
pub struct Other {
    pub kind: &'static str,
    pub key: String,
    pub value: f64,
}

/// Hottest first: anything past its critical threshold, then past its max,
/// then by how close it is to its limit, then by the raw value. A reading
/// whose chip exports no threshold is ranked against `assumed_limit`, so a
/// 62 °C `Tctl` outranks a 44 °C DIMM at 55 °C max (review).
pub fn hottest_first(a: &Reading, b: &Reading) -> std::cmp::Ordering {
    b.over_crit()
        .cmp(&a.over_crit())
        .then(b.over_max().cmp(&a.over_max()))
        .then(b.heat().total_cmp(&a.heat()))
        .then(b.value.total_cmp(&a.value))
        .then(a.key.cmp(&b.key))
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub temps: Vec<Reading>,
    pub others: Vec<Other>,
    pub info: Option<sensors::SensorsInfo>,
}

fn split_key(label: &Label) -> Option<(String, String)> {
    let Label::Name(s) = label else { return None };
    let s: &str = s;
    // `chip:label` — the chip may itself contain `:` (`r8169_0_b00:00`), so
    // the split is at the *last* colon only when the tail is the label.
    let (chip, label) = s.rsplit_once(':')?;
    Some((chip.to_string(), label.to_string()))
}

fn glob(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

impl Model {
    /// Derive from the store: every `sensor.temp_c` label with its
    /// thresholds; fans/volts/power; the inventory.
    pub fn refresh(
        &mut self,
        store: &gridwatch_store::Store,
        chips: &[String],
        sort: Sort,
        now: Ts,
    ) {
        let allowed = |chip: &str| chips.is_empty() || chips.iter().any(|p| glob(p, chip));
        let labels: Vec<Label> = store.labels(sensors::TEMP_C.id.name).cloned().collect();
        self.temps.clear();
        for l in labels {
            let Some((chip, label)) = split_key(&l) else {
                continue;
            };
            if !allowed(&chip) {
                continue;
            }
            let Label::Name(name) = &l else { continue };
            let key = sensors::TEMP_C.named(name);
            let Some((at, value)) = store.last(&key) else {
                continue;
            };
            // A chip that stopped answering keeps its last value in the
            // store for ever; the tile drops it instead (review).
            if now.since(at) > STALE_AFTER || !value.is_finite() {
                continue;
            }
            self.temps.push(Reading {
                key: name.to_string(),
                max: store.last(&sensors::MAX_C.named(name)).map(|(_, v)| v),
                crit: store.last(&sensors::CRIT_C.named(name)).map(|(_, v)| v),
                chip,
                label,
                value,
            });
        }
        match sort {
            Sort::Hottest => self.temps.sort_by(hottest_first),
            Sort::Chip => self.temps.sort_by(|a, b| a.key.cmp(&b.key)),
        }
        self.others.clear();
        for (kind, key) in [
            ("fan", &sensors::FAN_RPM),
            ("volt", &sensors::VOLT_V),
            ("power", &sensors::POWER_W),
        ] {
            let labels: Vec<Label> = store.labels(key.id.name).cloned().collect();
            for l in labels {
                let Label::Name(name) = &l else { continue };
                if let Some((_, v)) = store.last(&key.named(name)) {
                    self.others.push(Other {
                        kind,
                        key: name.to_string(),
                        value: v,
                    });
                }
            }
        }
        self.info = store.record(&sensors::INFO).map(|(_, i)| i.clone());
    }

    /// The hottest reading, whatever the tile's sort is.
    pub fn hottest(&self) -> Option<&Reading> {
        self.temps.iter().min_by(|a, b| hottest_first(a, b))
    }

    /// One reading per chip, hottest first (the strip).
    pub fn per_chip(&self) -> Vec<&Reading> {
        let mut out: Vec<&Reading> = Vec::new();
        let mut sorted: Vec<&Reading> = self.temps.iter().collect();
        sorted.sort_by(|a, b| hottest_first(a, b));
        for r in sorted {
            if !out.iter().any(|o| o.chip == r.chip) {
                out.push(r);
            }
        }
        out
    }
}

pub struct Sensors {
    options: Options,
    model: Model,
    sort: Sort,
    seen: Option<Ts>,
    scroll: usize,
}

impl Sensors {
    pub fn new(options: Options) -> Sensors {
        Sensors {
            sort: options.sort,
            options,
            model: Model::default(),
            seen: None,
            scroll: 0,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Sensors, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Sensors::new(parsed))
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }
}

impl Default for Sensors {
    fn default() -> Sensors {
        Sensors::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Sensors::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

impl Component for Sensors {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("sensors")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let Some(at) = cx.store.last_sample(sensors::SOURCE) else {
            return Redraw::No;
        };
        if self.seen == Some(at) {
            return Redraw::No;
        }
        self.seen = Some(at);
        self.model
            .refresh(cx.store, &self.options.chips, self.sort, cx.now);
        Redraw::Yes
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        match key.code {
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Consumed
            }
            KeyCode::Down => {
                self.scroll = (self.scroll + 1).min(self.model.temps.len().saturating_sub(1));
                Outcome::Consumed
            }
            KeyCode::Char('o') => {
                self.sort = self.sort.next();
                self.model
                    .refresh(cx.store, &self.options.chips, self.sort, cx.store.latest());
                self.scroll = 0;
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
            TIER_HOTTEST | TIER_STRIP => &["°"],
            TIER_TABLE => &["chip"],
            TIER_CHART => &["chart"],
            _ => &["RAPL"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split_at_the_last_colon_and_globs_match() {
        let l = Label::Name(std::sync::Arc::from("r8169_0_b00:00:temp1"));
        assert_eq!(
            split_key(&l),
            Some(("r8169_0_b00:00".to_string(), "temp1".to_string()))
        );
        let l = Label::Name(std::sync::Arc::from("k10temp:Tccd1"));
        assert_eq!(
            split_key(&l),
            Some(("k10temp".to_string(), "Tccd1".to_string()))
        );
        assert_eq!(split_key(&Label::None), None);
        assert!(glob("nvme*", "nvme#2"));
        assert!(!glob("nvme", "nvme#2"));
        let r = Reading {
            chip: "nvme".into(),
            label: "Composite".into(),
            key: "nvme:Composite".into(),
            value: 82.0,
            max: Some(81.85),
            crit: Some(84.85),
        };
        assert!(r.over_max() && !r.over_crit());
        assert!(r.margin() < 0.0);
        let t: toml::Table = toml::from_str(r#"sort = "chip""#).unwrap();
        assert_eq!(Sensors::from_table(&t).unwrap().sort(), Sort::Chip);
        let t: toml::Table = toml::from_str(r#"colour = 1"#).unwrap();
        assert!(Sensors::from_table(&t).is_err());
    }
}
