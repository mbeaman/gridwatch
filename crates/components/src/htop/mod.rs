//! The htop component (§8): htop 3.4.1's dashboard face as a view tree —
//! four cumulative tiers from an 8×3 chip to the 32-core CCD blocks. Every
//! number comes from the cpu source through the store; the component never
//! reads a file, names a colour or picks a glyph (§4.6).
//!
//! Arc 1b shipped `tiny` → `cores`; arc 2a adds the top-N process table
//! (`table`, min 56×18) over the pid-level scan behind `Detail::Table`
//! (§8.1). htop's whole Main screen (`full`, zoom-only) is arc 8.

pub mod format;
pub mod table;
mod view;

use std::borrow::Cow;

use gridwatch_store::keys::cpu;
use gridwatch_store::{Detail, KeyCode, KeyEvent};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, InputCx, KeyHint, Manifest,
    Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

use table::{Col, Derived};

pub static MANIFEST: Manifest = Manifest {
    kind: "htop",
    name: "CPU",
    summary: "htop's meters, per-core CCD blocks, memory, load and pressure",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 6, h: 3 },
    requires: &[gridwatch_store::Capability::Procfs],
    optional: &[
        gridwatch_store::Capability::Cpufreq,
        gridwatch_store::Capability::Hwmon,
    ],
    sources: &[cpu::SOURCE],
    optional_sources: &[],
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
    ],
    example_options: "options = { table_rows = 10, sort = \"cpu\" }",
};

/// Rows in brackets are what the tier occupies (§8): `tiny` [3],
/// `big-number` [4], `meters` [6], `cores` [12].
static TIERS: &[Tier] = &[
    Tier {
        name: "tiny",
        min: Size::new(8, 3),
        adds: &["total CPU%", "sparkline"],
        zoom_only: false,
    },
    Tier {
        name: "big-number",
        min: Size::new(12, 4),
        adds: &["big digits"],
        zoom_only: false,
    },
    Tier {
        name: "meters",
        min: Size::new(30, 6),
        // The tasks line is shortened clause by clause to fit; load and uptime
        // are appended per clause as the tile widens, and the pressure row
        // when a spare line remains.
        adds: &["cpu/mem/swap meters", "pids · tasks · load · uptime", "PSI"],
        zoom_only: false,
    },
    Tier {
        name: "cores",
        min: Size::new(56, 12),
        adds: &["per-core bars in CCD blocks", "MHz", "Tccd"],
        zoom_only: false,
    },
    // 12 rows of `cores` + a header + htop's five-row floor (§8.1).
    Tier {
        name: "table",
        min: Size::new(56, 18),
        adds: &["top-N process table", "kthr in the task line"],
        zoom_only: false,
    },
];

pub const TIER_TINY: usize = 0;
pub const TIER_BIG_NUMBER: usize = 1;
pub const TIER_METERS: usize = 2;
pub const TIER_CORES: usize = 3;
pub const TIER_TABLE: usize = 4;

/// Rows the tiers below the table occupy inside it (§8.1 `rows_above`).
pub const ROWS_ABOVE_TABLE: u16 = 12;

/// View-only instance options (§9): every one of htop's grid-relevant settings
/// parses today; the ones that only matter to the process table are inert until
/// arc 2 ships the `table` tier. Source cadence lives in `[sources.cpu]` and
/// must never appear here — `option_names_are_disjoint` enforces it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub hide_kernel_threads: bool,
    pub hide_userland_threads: bool,
    pub sort: String,
    pub tree: bool,
    pub table_rows: u16,
    pub columns: Vec<String>,
    pub command_min: u16,
    pub highlight_base_name: bool,
    pub highlight_changes: bool,
    /// Seconds (§8 writes it `5s`; the unit is fixed, the value is a count).
    pub highlight_changes_delay: u16,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            // htop's default.
            hide_kernel_threads: true,
            // A deliberate deviation from htop (whose default is false): the
            // grid table must not be ten rows of one game's threads, and it
            // keeps the scan pid-level (§8).
            hide_userland_threads: true,
            sort: "cpu".into(),
            tree: false,
            table_rows: 10,
            columns: DEFAULT_COLUMNS.iter().map(|c| c.to_string()).collect(),
            command_min: 20,
            highlight_base_name: false,
            highlight_changes: false,
            highlight_changes_delay: 5,
        }
    }
}

/// The grid's default column set (§8.1); `USER`, `VIRT`, `PRI` and `NI` are off
/// on the grid for space (D27) and available through `columns`.
pub const DEFAULT_COLUMNS: &[&str] = &[
    "pid", "res", "shr", "state", "cpu", "mem", "time", "command",
];

/// Every sort identifier htop accepts for these columns (§8).
pub const SORT_KEYS: &[&str] = &[
    "pid", "user", "pri", "nice", "virt", "res", "shr", "state", "cpu", "mem", "time", "command",
];

/// The option names this component owns — the disjointness half of §9's rule.
pub const OPTION_NAMES: &[&str] = &[
    "hide_kernel_threads",
    "hide_userland_threads",
    "sort",
    "tree",
    "table_rows",
    "columns",
    "command_min",
    "highlight_base_name",
    "highlight_changes",
    "highlight_changes_delay",
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
        Ok(Options {
            // htop never shows fewer than five rows of table (§8).
            table_rows: self.table_rows.max(5),
            ..self
        })
    }
}

pub struct Htop {
    options: Options,
    /// The enabled columns in htop's order, from `options.columns`.
    columns: Vec<Col>,
    derived: Derived,
    /// The `proc.table` timestamp the rows were derived from: the cpu
    /// generation moves every meters tick, the table only every scan.
    table_seen: Option<gridwatch_store::Ts>,
    sort: Col,
    desc: bool,
    /// Selection keyed by PID so a re-sort never moves the cursor (§8.1).
    selected: Option<i32>,
    /// First visible row, kept so the cursor moves within the page and the
    /// page scrolls only at its edges (htop's behaviour).
    scroll: usize,
}

impl Htop {
    pub fn new(options: Options) -> Htop {
        let sort = Col::from_id(&options.sort).unwrap_or(Col::Cpu);
        let columns = options
            .columns
            .iter()
            .filter_map(|c| Col::from_id(c))
            .collect();
        Htop {
            desc: table::default_dir(sort),
            sort,
            columns,
            derived: Derived::default(),
            table_seen: None,
            selected: None,
            scroll: 0,
            options,
        }
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    /// The sorted, filtered rows as of the last `tick` (tests).
    pub fn rows(&self) -> &[cpu::ProcRow] {
        &self.derived.rows
    }

    pub fn sort(&self) -> (Col, bool) {
        (self.sort, self.desc)
    }

    pub fn selected(&self) -> Option<i32> {
        self.selected
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The body rows a table in `inner` shows on the grid (§8.1's budget);
    /// the zoomed body is larger, which only makes this a conservative page.
    fn page_rows(&self, inner_height: u16) -> usize {
        usize::from(inner_height.saturating_sub(ROWS_ABOVE_TABLE + 1))
            .min(usize::from(self.options.table_rows))
            .max(1)
    }

    /// Keep the selected row inside `[scroll, scroll + rows)`.
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

    pub(crate) fn columns(&self) -> &[Col] {
        &self.columns
    }

    pub(crate) fn derived(&self) -> &Derived {
        &self.derived
    }

    fn resort(&mut self) {
        let rows = std::mem::take(&mut self.derived.rows);
        let table = cpu::ProcTable {
            rows,
            pid_digits: self.derived.pid_digits,
        };
        self.derived
            .rebuild(&table, &self.options, self.sort, self.desc);
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
        // Nothing selected yet (htop always has a row; the grid waits for the
        // first key): a single step lands on the edge it came from, a page or
        // an end key does what it says from row 0.
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
        // A `sort` outside the enabled set (the default set has no `user`)
        // starts the cycle from before the first column.
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

impl Default for Htop {
    fn default() -> Htop {
        Htop::new(Options::default())
    }
}

impl Htop {
    /// Parse and validate an instance's `options` table — the one path `build`
    /// takes, exposed so a test can inspect the *validated* options rather than
    /// re-implementing the parse.
    pub fn from_table(options: &toml::Table) -> Result<Htop, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Htop::new(parsed.validate()?))
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Htop::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

impl Component for Htop {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("cpu")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    /// Only the table tier turns on the pid-level scan (§4.3, §8.1); every
    /// tier below it is meters-only.
    fn demand(&self, tier: usize) -> Detail {
        if tier >= TIER_TABLE {
            Detail::Table
        } else {
            Detail::Meters
        }
    }

    /// The sorted, filtered rows are derived once per cpu generation (§8.1:
    /// "the htop component sorts and filters in `tick`; `view` never sorts").
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let Some((at, table)) = cx.store.record(&cpu::PROC_TABLE) else {
            return Redraw::No;
        };
        if self.table_seen == Some(at) {
            return Redraw::No;
        }
        self.table_seen = Some(at);
        let old_index = self
            .selected
            .and_then(|pid| self.derived.rows.iter().position(|r| r.pid == pid));
        self.derived
            .rebuild(table, &self.options, self.sort, self.desc);
        if let Some(pid) = self.selected
            && !self.derived.rows.iter().any(|r| r.pid == pid)
        {
            // The selected process vanished: stay on the same row, as htop
            // does, rather than losing the cursor.
            self.selected = old_index
                .and_then(|i| {
                    self.derived
                        .rows
                        .get(i.min(self.derived.rows.len().saturating_sub(1)))
                })
                .map(|r| r.pid);
        }
        Redraw::Yes
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        let rows = self.page_rows(cx.inner.height);
        let page = rows as isize;
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
            // `tiny` prints "CPU" only when 8+ wide; the number always ends in %.
            TIER_TINY => &["%"],
            // Big digits are block glyphs and the '%' is dropped below 16 wide:
            // non-blank is the only honest textual claim.
            TIER_BIG_NUMBER => &[],
            // The task line's wording depends on whether the scan has run
            // (`pids`/`tasks` before, htop's `thr`/`kthr` after), so the
            // meters claim is the three bars, which every store shows.
            TIER_METERS => &["CPU", "MEM", "SWP"],
            TIER_CORES => &["CPU", "MEM", "SWP", "CCD", "PSI"],
            _ => &["CCD", "kthr", "PID", "CPU%", "Command"],
        }
    }
}
