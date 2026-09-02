//! The htop component (§8): htop 3.4.1's dashboard face as a view tree —
//! four cumulative tiers from an 8×3 chip to the 32-core CCD blocks. Every
//! number comes from the cpu source through the store; the component never
//! reads a file, names a colour or picks a glyph (§4.6).
//!
//! Arc 1b ships `tiny` → `cores`; the top-N process table (`table`) and htop's
//! whole Main screen (`full`, zoom-only) arrive in arcs 2 and 8 with the
//! pid-level scan behind `Detail::Table` (§8.1).

mod format;
mod view;

use std::borrow::Cow;

use gridwatch_store::Detail;
use gridwatch_store::keys::cpu;
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, KeyHint, Manifest, Redraw,
    RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

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
    keys: &[KeyHint {
        key: "z",
        does: "zoom (the sortable process table arrives in arc 2)",
    }],
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
        // The tasks line always fits; load and uptime are appended per clause
        // as the tile widens, and the pressure row when a spare line remains.
        adds: &["cpu/mem/swap meters", "pids · tasks · load · uptime", "PSI"],
        zoom_only: false,
    },
    Tier {
        name: "cores",
        min: Size::new(56, 12),
        adds: &["per-core bars in CCD blocks", "MHz", "Tccd"],
        zoom_only: false,
    },
];

pub const TIER_TINY: usize = 0;
pub const TIER_BIG_NUMBER: usize = 1;
pub const TIER_METERS: usize = 2;
pub const TIER_CORES: usize = 3;

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
}

impl Htop {
    pub fn new(options: Options) -> Htop {
        Htop { options }
    }

    pub fn options(&self) -> &Options {
        &self.options
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

    /// Every 1b tier is meters-only: the pid-level scan is `Detail::Table` and
    /// belongs to the `table` tier in arc 2 (§4.3, §8.1).
    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn tick(&mut self, _cx: &TickCx<'_>) -> Redraw {
        // Nothing is derived per generation: the view is pure over the store,
        // and the shell already redraws when the cpu source's generation moves.
        Redraw::No
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
            TIER_METERS => &["CPU", "MEM", "SWP", "pids"],
            _ => &["CPU", "MEM", "SWP", "CCD", "PSI"],
        }
    }
}
