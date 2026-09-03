//! The gpu component (§8, §8.1, brief 2b task 2): nvtop 3.2.0's face as a view
//! tree — six cumulative tiers from an 8×3 badge through nvtop's header, the
//! ten-minute charts and the process table to the zoom-only `full`. Every
//! number comes from the gpu source through the store; the four joined
//! columns come from the cpu source's `proc.table` (optional) and render `—`
//! without it. No I/O, no colours, no glyphs, no `Instant` (§4.6).

pub mod format;
pub mod table;
mod view;

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::{cpu, gpu};
use gridwatch_store::{ActionId, Detail, KeyCode, KeyEvent, Ts};
use gridwatch_ui::actions::ProcAction;

/// The signals the `F9` picker offers (`components::signals`), which the
/// htop tile shares — it lives outside both so the gpu tile builds without
/// the htop feature.
pub use crate::signals::SIGNALS;
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, InputCx, KeyHint,
    Manifest, Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

use table::{Col, Derived};

pub static MANIFEST: Manifest = Manifest {
    kind: "gpu",
    name: "GPU",
    summary: "nvtop's header, gauges, ten-minute charts and the GPU process table over NVML",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 6, h: 3 },
    // The component renders honestly from the store without NVML (a replay,
    // `--demo`); the *source* requires the capability.
    requires: &[],
    optional: &[gridwatch_store::Capability::Nvml],
    sources: &[gpu::SOURCE],
    optional_sources: &[cpu::SOURCE],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "↑/↓ j/k PgUp/PgDn Home/End",
            does: "select a process",
        },
        KeyHint {
            key: "< > F6",
            does: "sort column",
        },
        KeyHint {
            key: "I",
            does: "invert the sort",
        },
        KeyHint {
            key: "1-6",
            does: "toggle a chart series (util, vram, temp, power, clock, load)",
        },
        KeyHint {
            key: "r",
            does: "reverse the chart (recent on the left, nvtop -r)",
        },
    ],
    example_options: "options = { table_rows = 10, sort = \"gpu_mem\", series = [\"util\", \"vram\", \"power\"] }",
};

/// Rows in brackets are what the tier occupies (§8): `badge` [3], `gauges`
/// [5], `header` [8], `charts` [8 + 4..8], `procs` [+ header row + 5..], `full`.
static TIERS: &[Tier] = &[
    Tier {
        name: "badge",
        min: Size::new(8, 3),
        adds: &["GPU%", "°C", "util sparkline"],
        zoom_only: false,
    },
    Tier {
        name: "gauges",
        min: Size::new(24, 5),
        adds: &[
            "GPU / VRAM / MEMCTL gauges",
            "MHz · W/limit · °C · fan",
            "throttle chip",
        ],
        zoom_only: false,
    },
    Tier {
        name: "header",
        min: Size::new(56, 8),
        adds: &[
            "nvtop's three header lines",
            "PCIe gen@width RX/TX",
            "ENC/DEC auto-hidden after 30 s idle",
            "20 ms power sparkline",
        ],
        zoom_only: false,
    },
    Tier {
        name: "charts",
        min: Size::new(56, 12),
        adds: &[
            "ten-minute charts (util, vram, temp, power, clock, load)",
            "GPU-Z spec column at ≥ 100 wide",
        ],
        zoom_only: false,
    },
    Tier {
        name: "procs",
        min: Size::new(56, 18),
        adds: &["top-N GPU process table"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "every process row",
            "USER",
            "the Power sub-panel (pins, arc 3)",
        ],
        zoom_only: true,
    },
];

pub const TIER_BADGE: usize = 0;
pub const TIER_GAUGES: usize = 1;
pub const TIER_HEADER: usize = 2;
pub const TIER_CHARTS: usize = 3;
pub const TIER_PROCS: usize = 4;
pub const TIER_FULL: usize = 5;

/// Rows nvtop's header occupies inside the richer tiers (§8.1 `rows_above`).
pub const HEADER_ROWS: u16 = 8;
/// The chart band's bounds (§8.1): `clamp(inner.height − 8 − 1 − table_rows, 4, 8)`.
pub const BAND_MIN: u16 = 4;
pub const BAND_MAX: u16 = 8;
/// nvtop's `encode_decode_hiding_timer`: 30 s.
pub const ENCDEC_HIDE_AFTER: Duration = Duration::from_secs(30);
/// nvtop's ring buffer: ten minutes.
pub const CHART_SPAN: Duration = Duration::from_secs(600);

/// The chart series, in key order (`1`–`6`).
pub const SERIES: &[&str] = &["util", "vram", "temp", "power", "clock", "load"];

/// View-only instance options (§9).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub sort: String,
    pub table_rows: u16,
    pub columns: Vec<String>,
    pub command_min: u16,
    pub series: Vec<String>,
    pub reverse: bool,
    pub spec_column: bool,
    pub power_panel: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            sort: "gpu_mem".into(),
            table_rows: 10,
            columns: DEFAULT_COLUMNS.iter().map(|c| c.to_string()).collect(),
            command_min: 12,
            series: vec!["util".into(), "vram".into(), "power".into()],
            reverse: false,
            spec_column: true,
            power_panel: true,
        }
    }
}

/// The grid default set (§8.1): `USER` is off (D27), `ENC`/`DEC` off as in
/// nvtop's default field set.
pub const DEFAULT_COLUMNS: &[&str] = &[
    "pid", "dev", "type", "gpu", "gpu_mem", "cpu", "host_mem", "command",
];

/// nvtop's sort criteria as `sort` / `columns` identifiers.
pub const SORT_KEYS: &[&str] = &[
    "pid", "user", "dev", "type", "gpu", "enc", "dec", "gpu_mem", "cpu", "host_mem", "command",
];

pub const OPTION_NAMES: &[&str] = &[
    "sort",
    "table_rows",
    "columns",
    "command_min",
    "series",
    "reverse",
    "spec_column",
    "power_panel",
];

impl Options {
    fn validate(self) -> Result<Options, BuildError> {
        if !SORT_KEYS.contains(&self.sort.as_str()) {
            return Err(BuildError(format!(
                "sort = \"{}\" is not one of {}",
                self.sort,
                SORT_KEYS.join(" ")
            )));
        }
        for c in &self.columns {
            if !SORT_KEYS.contains(&c.as_str()) {
                return Err(BuildError(format!("unknown column \"{c}\"")));
            }
        }
        for s in &self.series {
            if !SERIES.contains(&s.as_str()) {
                return Err(BuildError(format!(
                    "unknown series \"{s}\" (one of {})",
                    SERIES.join(" ")
                )));
            }
        }
        Ok(Options {
            table_rows: self.table_rows.max(5),
            ..self
        })
    }
}

pub struct Gpu {
    options: Options,
    columns: Vec<Col>,
    derived: Derived,
    /// The `gpu.procs` and `proc.table` timestamps the rows were derived from.
    procs_seen: Option<Ts>,
    table_seen: Option<Ts>,
    sort: Col,
    desc: bool,
    selected: Option<i32>,
    scroll: usize,
    /// Chart series on (indexes into `SERIES`).
    series_on: [bool; 6],
    reverse: bool,
    /// nvtop's ENC/DEC hiding: the last time either was non-zero (or first
    /// seen), so the bars vanish 30 s into idleness.
    encdec_active_at: Option<Ts>,
    /// `h`/`l` in the zoomed `full` tier: nvtop scrolls its process table
    /// horizontally four columns at a time (arc 8a).
    col_scroll: usize,
    /// `F9`: which signal the picker is on, if it is open.
    signal_menu: Option<usize>,
}

impl Gpu {
    pub fn new(options: Options) -> Gpu {
        let sort = Col::from_id(&options.sort).unwrap_or(Col::GpuMem);
        let columns = options
            .columns
            .iter()
            .filter_map(|c| Col::from_id(c))
            .collect();
        let mut series_on = [false; 6];
        for s in &options.series {
            if let Some(i) = SERIES.iter().position(|n| n == s) {
                series_on[i] = true;
            }
        }
        Gpu {
            desc: table::default_dir(sort),
            sort,
            columns,
            derived: Derived::default(),
            procs_seen: None,
            table_seen: None,
            selected: None,
            scroll: 0,
            series_on,
            reverse: options.reverse,
            encdec_active_at: None,
            col_scroll: 0,
            signal_menu: None,
            options,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Gpu, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Gpu::new(parsed.validate()?))
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn rows(&self) -> &[table::Row] {
        &self.derived.rows
    }

    pub fn sort(&self) -> (Col, bool) {
        (self.sort, self.desc)
    }

    /// Is this instance *drawing* the zoom-only `full` tier? The shell
    /// says so; size cannot (arc 8a review, D58 amendment 7).
    fn zoomed(&self, cx: &InputCx<'_>) -> bool {
        cx.tier >= TIER_FULL
    }

    pub fn col_scroll(&self) -> usize {
        self.col_scroll
    }

    pub fn signal_menu(&self) -> Option<usize> {
        self.signal_menu
    }

    pub fn selected(&self) -> Option<i32> {
        self.selected
    }

    pub fn series_on(&self) -> &[bool; 6] {
        &self.series_on
    }

    pub fn reversed(&self) -> bool {
        self.reverse
    }

    pub(crate) fn columns(&self) -> &[Col] {
        &self.columns
    }

    pub(crate) fn derived(&self) -> &Derived {
        &self.derived
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    /// nvtop's ENC/DEC hiding: visible until 30 s after the last activity.
    pub fn encdec_visible_at(&self, now: Ts) -> bool {
        match self.encdec_active_at {
            Some(t) => now.since(t) < ENCDEC_HIDE_AFTER,
            None => true,
        }
    }

    /// The chart band's height for an inner height (§8.1) — the table tiers
    /// take what remains above `table_rows`, between 4 and 8 rows.
    pub fn band_rows(&self, tier: usize, inner_height: u16) -> u16 {
        if tier < TIER_PROCS {
            return inner_height
                .saturating_sub(HEADER_ROWS)
                .clamp(BAND_MIN, BAND_MAX);
        }
        inner_height
            .saturating_sub(HEADER_ROWS + 1 + self.options.table_rows)
            .clamp(BAND_MIN, BAND_MAX)
    }

    /// Body rows the table shows (§8.1 row budget): `min(table_rows,
    /// available)` on the grid, everything when zoomed.
    pub fn body_rows(&self, tier: usize, inner_height: u16, zoomed: bool) -> usize {
        let band = self.band_rows(tier, inner_height);
        let available = usize::from(inner_height.saturating_sub(HEADER_ROWS + band + 1));
        if zoomed {
            // The zoom-only `full` tier keeps one row for the Power sub-panel.
            let panel = usize::from(tier >= TIER_FULL && self.options.power_panel);
            available.saturating_sub(panel)
        } else {
            available.min(usize::from(self.options.table_rows))
        }
        .max(1)
    }

    /// Re-sort the joined rows in place (a key changed the sort).
    fn resort(&mut self) {
        table::sort_for(&mut self.derived.rows, self.sort, self.desc);
    }

    fn follow_selection(&mut self, rows: usize) {
        let Some(i) = self
            .selected
            .and_then(|pid| self.derived.rows.iter().position(|r| r.pid == pid))
        else {
            return;
        };
        if i < self.scroll {
            self.scroll = i;
        } else if i >= self.scroll + rows {
            self.scroll = i + 1 - rows;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let rows = &self.derived.rows;
        if rows.is_empty() {
            self.selected = None;
            return;
        }
        let cur = self
            .selected
            .and_then(|pid| rows.iter().position(|r| r.pid == pid));
        let next = match cur {
            None => match delta {
                1 => 0,
                -1 => rows.len() - 1,
                d if d > 0 => (d as usize).min(rows.len() - 1),
                _ => 0,
            },
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
        };
        self.selected = Some(rows[next].pid);
    }

    fn cycle_sort(&mut self, step: isize) {
        let cols = &self.columns;
        if cols.is_empty() {
            return;
        }
        let cur = cols
            .iter()
            .position(|c| *c == self.sort)
            .map(|i| i as isize)
            .unwrap_or(if step > 0 { -1 } else { 0 });
        let n = cols.len() as isize;
        let next = cols[((cur + step).rem_euclid(n)) as usize];
        self.sort = next;
        self.desc = table::default_dir(next);
        self.resort();
    }
}

impl Default for Gpu {
    fn default() -> Gpu {
        Gpu::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Gpu::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build: Box::new(build),
};

impl Component for Gpu {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("gpu")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    /// The table tiers ask for `Detail::Table` from *every* source in
    /// `sources ∪ optional_sources` — which raises the cpu scan too, so the
    /// joined columns exist even with no htop tile visible (§8).
    fn demand(&self, tier: usize) -> Detail {
        if tier >= TIER_PROCS {
            Detail::Table
        } else {
            Detail::Meters
        }
    }

    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let mut redraw = Redraw::No;
        // nvtop's ENC/DEC timer runs on the store's clock, never `Instant`.
        let enc = cx.store.last(&gpu::ENC_PCT.idx(0)).map(|(_, v)| v);
        let dec = cx.store.last(&gpu::DEC_PCT.idx(0)).map(|(_, v)| v);
        if enc.is_some() || dec.is_some() {
            let busy = enc.unwrap_or(0.0) > 0.0 || dec.unwrap_or(0.0) > 0.0;
            if busy || self.encdec_active_at.is_none() {
                self.encdec_active_at = Some(cx.now);
            }
        }
        let procs = cx.store.record(&gpu::PROCS.idx(0));
        let table = cx.store.record(&cpu::PROC_TABLE);
        let (procs_at, table_at) = (procs.map(|(t, _)| t), table.map(|(t, _)| t));
        if let Some((_, p)) = procs
            && (self.procs_seen != procs_at || self.table_seen != table_at)
        {
            self.procs_seen = procs_at;
            self.table_seen = table_at;
            let old_index = self
                .selected
                .and_then(|pid| self.derived.rows.iter().position(|r| r.pid == pid));
            self.derived
                .rebuild(p, 0, table.map(|(_, t)| t), self.sort, self.desc);
            if let Some(pid) = self.selected
                && !self.derived.rows.iter().any(|r| r.pid == pid)
            {
                self.selected = old_index
                    .and_then(|i| {
                        self.derived
                            .rows
                            .get(i.min(self.derived.rows.len().saturating_sub(1)))
                    })
                    .map(|r| r.pid);
            }
            redraw = Redraw::Yes;
        }
        redraw
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        let rows = self.body_rows(TIER_PROCS, cx.inner.height, false);
        let page = rows as isize;
        // The signal picker owns every key while it is open.
        if let Some(at) = self.signal_menu {
            match key.code {
                KeyCode::Esc => self.signal_menu = None,
                KeyCode::Up | KeyCode::Char('k') => self.signal_menu = Some(at.saturating_sub(1)),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.signal_menu = Some((at + 1).min(SIGNALS.len() - 1));
                }
                KeyCode::Enter => {
                    self.signal_menu = None;
                    let Some(row) = self
                        .selected
                        .and_then(|pid| self.derived.rows.iter().find(|r| r.pid == pid))
                    else {
                        return Outcome::Consumed;
                    };
                    let (name, number) = SIGNALS[at.min(SIGNALS.len() - 1)];
                    return Outcome::Command(Command::Run(
                        ActionId(0),
                        Box::new(ProcAction::Signal {
                            pids: vec![row.pid as u32],
                            signal: number,
                            signal_name: name.to_string(),
                            names: vec![
                                row.cmdline
                                    .as_ref()
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| format!("pid {}", row.pid)),
                            ],
                        }),
                    ));
                }
                _ => return Outcome::Ignored,
            }
            return Outcome::Consumed;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-page),
            KeyCode::PageDown => self.move_selection(page),
            KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::End => self.move_selection(isize::MAX / 2),
            KeyCode::Char('<') => self.cycle_sort(-1),
            KeyCode::Char('>') | KeyCode::F(6) => self.cycle_sort(1),
            KeyCode::Char('I') => {
                self.desc = !self.desc;
                self.resort();
            }
            KeyCode::Char('r') => self.reverse = !self.reverse,
            // nvtop's horizontal scroll and signal menu, in the zoomed
            // `full` tier only (arc 8a): the grid's `procs` table is a
            // dashboard, and a 4x2 tile has nowhere to scroll to.
            KeyCode::Char('h') if self.zoomed(cx) => {
                self.col_scroll = self.col_scroll.saturating_sub(4);
            }
            KeyCode::Char('l') if self.zoomed(cx) => {
                self.col_scroll = (self.col_scroll + 4).min(self.columns.len().saturating_sub(1));
            }
            KeyCode::F(9) if self.zoomed(cx) => {
                if self.selected.is_none() {
                    return Outcome::Consumed;
                }
                self.signal_menu = Some(0);
            }
            KeyCode::Char(c @ '1'..='6') => {
                let i = (c as u8 - b'1') as usize;
                self.series_on[i] = !self.series_on[i];
            }
            _ => return Outcome::Ignored,
        }
        self.follow_selection(rows);
        Outcome::Consumed
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_BADGE => &["%"],
            TIER_GAUGES => &["GPU", "VRAM"],
            TIER_HEADER | TIER_CHARTS => &["PCIe", "POW"],
            _ => &["PID", "GPU MEM"],
        }
    }
}
