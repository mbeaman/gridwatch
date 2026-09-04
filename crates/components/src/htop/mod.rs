//! The htop component (§8): htop 3.4.1's dashboard face as a view tree —
//! four cumulative tiers from an 8×3 chip to the 32-core CCD blocks. Every
//! number comes from the cpu source through the store; the component never
//! reads a file, names a colour or picks a glyph (§4.6).
//!
//! Arc 1b shipped `tiny` → `cores`; arc 2a adds the top-N process table
//! (`table`, min 56×18) over the pid-level scan behind `Detail::Table`
//! (§8.1). htop's whole Main screen (`full`, zoom-only) is arc 8.

pub mod format;
mod full;
pub mod table;
mod view;

use std::borrow::Cow;

use gridwatch_store::keys::cpu;
use gridwatch_store::{ActionId, Detail, KeyCode, KeyEvent};
use gridwatch_ui::actions::ProcAction;
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, InputCx, KeyHint,
    Manifest, Outcome, Redraw, RenderCx, Size, TickCx, Tier,
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
    // htop's whole face (arc 8a): the Main and I/O screens, search, filter,
    // the tree, tags, follow, both thread toggles and the F-key bar.
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "Main and I/O screens",
            "search, filter and tree",
            "tags, follow and the thread toggles",
            "the F-key bar and the process actions",
        ],
        zoom_only: true,
    },
];

pub const TIER_TINY: usize = 0;
pub const TIER_BIG_NUMBER: usize = 1;
pub const TIER_METERS: usize = 2;
pub const TIER_CORES: usize = 3;
pub const TIER_TABLE: usize = 4;
pub const TIER_FULL: usize = 5;

/// Does a row match what a person typed? Case-insensitive over the command
/// and the pid, which is what htop's search does.
pub fn matches_row(r: &cpu::ProcRow, needle: &str) -> bool {
    let n = needle.to_lowercase();
    r.cmdline.to_lowercase().contains(&n)
        || r.comm.to_lowercase().contains(&n)
        || r.pid.to_string().contains(&n)
}

/// Depth-first by parent pid, keeping each level in the current sort order
/// (htop's tree view is a re-ordering, not a different set). `visible` is
/// indices into `rows`; the result is the same indices, reordered.
pub fn tree_order(rows: &[cpu::ProcRow], visible: Vec<usize>) -> Vec<usize> {
    use std::collections::BTreeMap;
    let present: std::collections::BTreeSet<i32> = visible.iter().map(|i| rows[*i].pid).collect();
    let mut children: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for &i in &visible {
        let r = &rows[i];
        // A row whose parent is not in the set is a root here, which keeps
        // a filtered tree readable instead of empty.
        if present.contains(&r.ppid) && r.ppid != r.pid {
            children.entry(r.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }
    let mut out = Vec::with_capacity(visible.len());
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    let mut guard = visible.len() * 2 + 8;
    while let Some(i) = stack.pop() {
        out.push(i);
        guard -= 1;
        if guard == 0 {
            break; // a cycle in ppid: stop rather than spin
        }
        if let Some(kids) = children.get(&rows[i].pid) {
            for k in kids.iter().rev() {
                stack.push(*k);
            }
        }
    }
    out
}

/// Every visible row's depth, in one pass over the set rather than one map
/// per row (arc 8a review: this was O(n²) inside `view`).
pub fn tree_depths(rows: &[cpu::ProcRow], visible: &[usize]) -> Vec<usize> {
    let parent_of: std::collections::BTreeMap<i32, i32> = visible
        .iter()
        .map(|i| (rows[*i].pid, rows[*i].ppid))
        .collect();
    visible
        .iter()
        .map(|i| {
            let mut depth = 0;
            let mut pid = rows[*i].pid;
            let mut guard = 64;
            while let Some(&parent) = parent_of.get(&pid) {
                if parent == pid || guard == 0 || !parent_of.contains_key(&parent) {
                    break;
                }
                depth += 1;
                pid = parent;
                guard -= 1;
            }
            depth
        })
        .collect()
}

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
    /// Everything below is the zoom-only `full` tier's (arc 8a). None of it
    /// changes what the grid tiers draw.
    screen: Screen,
    /// `/`: the cursor jumps to a match; the rows are not filtered.
    search: Option<String>,
    /// `\`: rows that do not match are hidden.
    filter: Option<String>,
    /// Which of the two is taking keystrokes, if either.
    typing: Option<Typing>,
    tags: std::collections::BTreeSet<i32>,
    /// `F`: the cursor sticks to its pid across sorts and rebuilds.
    follow: bool,
    /// `t`, `K`, `H`: the runtime halves of three options.
    tree: bool,
    show_kernel: bool,
    show_userland: bool,
    /// An open menu (`F9`, `a`, `i`) and where its cursor is.
    menu: Option<Menu>,
    /// Indices into `derived.rows`, in draw order, and each one's depth in
    /// the tree — both computed in `tick`, never in `view`.
    visible: Vec<usize>,
    depths: Vec<usize>,
}

/// htop's screens, as tabs. Its user-configurable screen set is a config
/// surface nobody has asked for; these two are the ones that matter here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Main,
    Io,
}

impl Screen {
    pub fn name(self) -> &'static str {
        match self {
            Screen::Main => "Main",
            Screen::Io => "I/O",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    Search,
    Filter,
}

/// The three pickers, each a list the cursor walks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Menu {
    /// `F9`: which signal.
    Signal { at: usize },
    /// `a`: which CPUs. `chosen` is a set of cpu indices.
    Affinity {
        at: usize,
        chosen: std::collections::BTreeSet<usize>,
        cpus: usize,
    },
    /// `i`: which I/O class.
    IoPrio { at: usize },
}

/// The signals the `F9` menu offers (`components::signals`), shared with
/// the gpu tile.
pub use crate::signals::SIGNALS;

/// The I/O classes the `i` menu offers.
pub const IO_CLASSES: &[(&str, gridwatch_ui::actions::IoClass)] = &[
    ("best-effort", gridwatch_ui::actions::IoClass::BestEffort),
    ("idle", gridwatch_ui::actions::IoClass::Idle),
    ("realtime", gridwatch_ui::actions::IoClass::RealTime),
];

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
            screen: Screen::Main,
            search: None,
            filter: None,
            typing: None,
            tags: Default::default(),
            follow: false,
            tree: options.tree,
            show_kernel: !options.hide_kernel_threads,
            show_userland: !options.hide_userland_threads,
            menu: None,
            visible: Vec::new(),
            depths: Vec::new(),
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

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn search(&self) -> Option<&str> {
        self.search.as_deref()
    }

    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub fn typing(&self) -> Option<Typing> {
        self.typing
    }

    pub fn tags(&self) -> &std::collections::BTreeSet<i32> {
        &self.tags
    }

    pub fn follow(&self) -> bool {
        self.follow
    }

    pub fn tree(&self) -> bool {
        self.tree
    }

    pub fn show_userland(&self) -> bool {
        self.show_userland
    }

    pub fn show_kernel(&self) -> bool {
        self.show_kernel
    }

    /// Re-derive the rows from the store, whatever the table stamp says —
    /// what a filter change needs, since the derived set has already had
    /// the old filter applied to it.
    fn rederive(&mut self, cx: &InputCx<'_>) {
        let Some((at, table)) = cx.store.record(&cpu::PROC_TABLE) else {
            return;
        };
        self.table_seen = Some(at);
        self.derived
            .rebuild(table, &self.options, self.threads(), self.sort, self.desc);
        self.rebuild_visible();
        // A followed process that exits releases follow on the next scan,
        // not on the next keystroke (arc 8a review).
        self.keep_followed();
    }

    /// What the row filter should hide right now.
    fn threads(&self) -> table::Threads {
        table::Threads {
            hide_kernel: !self.show_kernel,
            hide_userland: !self.show_userland,
        }
    }

    pub fn menu(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }

    /// The rows the `full` tier draws — the filter applied and the tree
    /// order if it is on — as indices into `derived.rows`, computed in
    /// `tick` and whenever a key changes them.
    ///
    /// It used to be computed in `view`, where the per-row depth rebuilt a
    /// map over every row: 638 rows in tree mode cost ~407 000 map inserts
    /// a frame, and §8.1 says plainly that `view` never sorts (arc 8a
    /// review, D58 amendment 11).
    fn rebuild_visible(&mut self) {
        let mut idx: Vec<usize> = self
            .derived
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| match self.filter.as_deref() {
                Some(f) if !f.is_empty() => matches_row(r, f),
                _ => true,
            })
            .map(|(i, _)| i)
            .collect();
        if self.tree {
            idx = tree_order(&self.derived.rows, idx);
            self.depths = tree_depths(&self.derived.rows, &idx);
        } else {
            self.depths = vec![0; idx.len()];
        }
        self.visible = idx;
    }

    /// The rows the `full` tier draws. The grid tiers use `rows()`.
    pub fn visible_rows(&self) -> Vec<&cpu::ProcRow> {
        self.visible
            .iter()
            .filter_map(|i| self.derived.rows.get(*i))
            .collect()
    }

    /// How deep each visible row sits in the tree, in the same order.
    pub fn depths(&self) -> &[usize] {
        &self.depths
    }

    /// The processes an action applies to: every tagged row, or the one
    /// under the cursor. htop works this way and it is why `Space` exists.
    pub fn action_targets(&self) -> Vec<(i32, String)> {
        let rows = self.visible_rows();
        if !self.tags.is_empty() {
            return rows
                .iter()
                .filter(|r| self.tags.contains(&r.pid))
                .map(|r| (r.pid, r.comm.to_string()))
                .collect();
        }
        self.selected
            .and_then(|pid| rows.iter().find(|r| r.pid == pid).copied())
            .map(|r| vec![(r.pid, r.comm.to_string())])
            .unwrap_or_default()
    }

    /// The row the cursor is on, in the `full` tier's row set.
    /// The `full` tier's keys. `Ignored` falls through to the shared table
    /// keys (selection, sort), which every tier has.
    fn full_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        // A menu owns every key while it is open.
        if self.menu.is_some() {
            return self.menu_key(key, cx);
        }
        // So does the search or filter line.
        if let Some(mode) = self.typing {
            return self.typing_key(key, mode);
        }
        match key.code {
            // A filter in force says "Esc to clear" on screen; before
            // this, Esc fell through and released the capture instead
            // (arc 8a review, D58 amendment 12).
            KeyCode::Esc if self.filter.as_deref().is_some_and(|f| !f.is_empty()) => {
                self.filter = None;
                self.rebuild_visible();
            }
            // htop's F5 is the tree, and the F-key bar says so; `t` is the
            // same switch for a keyboard without function keys.
            KeyCode::F(5) => {
                self.tree = !self.tree;
                self.rebuild_visible();
            }
            KeyCode::Char('/') => {
                self.typing = Some(Typing::Search);
                self.search = Some(String::new());
            }
            KeyCode::Char('\\') => {
                self.typing = Some(Typing::Filter);
                self.filter = Some(String::new());
            }
            KeyCode::Char('t') => {
                self.tree = !self.tree;
                self.rebuild_visible();
            }
            KeyCode::Char(' ') => {
                if let Some(pid) = self.selected
                    && !self.tags.insert(pid)
                {
                    self.tags.remove(&pid);
                }
                // htop moves down after tagging, so a run of rows can be
                // tagged with one finger.
                self.move_selection(1);
            }
            KeyCode::Char('U') => self.tags.clear(),
            KeyCode::Char('F') => self.follow = !self.follow,
            // `resort` works on the rows already derived, which is no use
            // for a filter that must *add* rows back: forget the table
            // stamp instead, and the next tick re-derives from the store
            // (arc 8a review, D58 amendment 10).
            KeyCode::Char('K') => {
                self.show_kernel = !self.show_kernel;
                self.rederive(cx);
            }
            KeyCode::Char('H') => {
                self.show_userland = !self.show_userland;
                self.rederive(cx);
            }
            KeyCode::Tab => {
                self.screen = match self.screen {
                    Screen::Main => Screen::Io,
                    Screen::Io => Screen::Main,
                };
            }
            KeyCode::F(9) => {
                if self.action_targets().is_empty() {
                    return Outcome::Consumed;
                }
                self.menu = Some(Menu::Signal { at: 0 });
            }
            KeyCode::Char('a') => {
                let Some((pid, _)) = self.action_targets().first().cloned() else {
                    return Outcome::Consumed;
                };
                let _ = pid;
                self.menu = Some(Menu::Affinity {
                    at: 0,
                    chosen: Default::default(),
                    cpus: self.cpu_count(cx),
                });
            }
            KeyCode::Char('i') => {
                if self.action_targets().is_empty() {
                    return Outcome::Consumed;
                }
                self.menu = Some(Menu::IoPrio { at: 0 });
            }
            // htop's F7/F8. One step, and no question: the opposite key
            // undoes it.
            KeyCode::F(7) | KeyCode::F(8) => {
                let targets = self.action_targets();
                if targets.is_empty() {
                    return Outcome::Consumed;
                }
                let delta = if key.code == KeyCode::F(7) { -1 } else { 1 };
                return Outcome::Command(Command::Run(
                    ActionId(0),
                    Box::new(ProcAction::Renice {
                        pids: targets.iter().map(|(p, _)| *p as u32).collect(),
                        delta,
                    }),
                ));
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    /// How many CPUs the affinity picker offers, from what the store knows.
    fn cpu_count(&self, cx: &InputCx<'_>) -> usize {
        cx.store
            .labels(cpu::CORE_PCT.id.name)
            .filter(|l| matches!(l, gridwatch_store::Label::Index(_)))
            .count()
            .max(1)
    }

    fn typing_key(&mut self, key: KeyEvent, mode: Typing) -> Outcome {
        let buf = match mode {
            Typing::Search => &mut self.search,
            Typing::Filter => &mut self.filter,
        };
        match key.code {
            KeyCode::Esc => {
                *buf = None;
                self.typing = None;
            }
            KeyCode::Enter => self.typing = None,
            KeyCode::Backspace => {
                if let Some(s) = buf.as_mut() {
                    s.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = buf.as_mut() {
                    s.push(c);
                }
            }
            _ => return Outcome::Ignored,
        }
        // The filter decides which rows exist, so the set is rebuilt here
        // rather than in `view`.
        if mode == Typing::Filter {
            self.rebuild_visible();
        }
        // Search moves the cursor to the first match; filter changes the
        // row set, so the cursor is clamped by `visible_rows`.
        if mode == Typing::Search
            && let Some(needle) = self.search.clone()
            && !needle.is_empty()
            && let Some(hit) = self
                .visible_rows()
                .iter()
                .find(|r| matches_row(r, &needle))
                .map(|r| r.pid)
        {
            self.selected = Some(hit);
        }
        Outcome::Consumed
    }

    fn menu_key(&mut self, key: KeyEvent, _cx: &InputCx<'_>) -> Outcome {
        let targets = self.action_targets();
        let Some(menu) = self.menu.clone() else {
            return Outcome::Ignored;
        };
        let len = match &menu {
            Menu::Signal { .. } => SIGNALS.len(),
            Menu::IoPrio { .. } => IO_CLASSES.len(),
            Menu::Affinity { cpus, .. } => *cpus,
        };
        match key.code {
            KeyCode::Esc => {
                self.menu = None;
                return Outcome::Consumed;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(m) = self.menu.as_mut() {
                    let at = match m {
                        Menu::Signal { at } | Menu::IoPrio { at } | Menu::Affinity { at, .. } => at,
                    };
                    *at = at.saturating_sub(1);
                }
                return Outcome::Consumed;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = self.menu.as_mut() {
                    let at = match m {
                        Menu::Signal { at } | Menu::IoPrio { at } | Menu::Affinity { at, .. } => at,
                    };
                    *at = (*at + 1).min(len.saturating_sub(1));
                }
                return Outcome::Consumed;
            }
            KeyCode::Char(' ') => {
                // The affinity picker's only multi-select.
                if let Some(Menu::Affinity { at, chosen, .. }) = self.menu.as_mut()
                    && !chosen.insert(*at)
                {
                    chosen.remove(at);
                }
                return Outcome::Consumed;
            }
            KeyCode::Enter => {}
            // A menu owns every key while it is open, as the docstring
            // says: without this, PageDown moved the selection *under* an
            // open picker and changed what it would act on (arc 8a
            // review, D58 amendment 12).
            _ => return Outcome::Consumed,
        }
        // Enter: build the action and hand it to the shell, which asks the
        // question and runs it (§4.6).
        self.menu = None;
        if targets.is_empty() {
            return Outcome::Consumed;
        }
        let pids: Vec<u32> = targets.iter().map(|(p, _)| *p as u32).collect();
        let names: Vec<String> = targets.iter().map(|(_, n)| n.clone()).collect();
        let action: Box<dyn gridwatch_ui::component::Action> = match menu {
            Menu::Signal { at } => {
                let (name, number) = SIGNALS[at.min(SIGNALS.len() - 1)];
                Box::new(ProcAction::Signal {
                    pids,
                    signal: number,
                    signal_name: name.to_string(),
                    names,
                })
            }
            Menu::IoPrio { at } => {
                let (_, class) = IO_CLASSES[at.min(IO_CLASSES.len() - 1)];
                Box::new(ProcAction::IoPrio {
                    pid: pids[0],
                    class,
                    level: 4,
                })
            }
            Menu::Affinity { chosen, cpus, at } => {
                let mut cpu_list: Vec<usize> = chosen.into_iter().collect();
                if cpu_list.is_empty() {
                    cpu_list.push(at.min(cpus.saturating_sub(1)));
                }
                Box::new(ProcAction::Affinity {
                    pid: pids[0],
                    cpus: cpu_list,
                })
            }
        };
        Outcome::Command(Command::Run(ActionId(0), action))
    }

    /// `F` follow: the cursor already sticks to its pid, because the
    /// selection *is* a pid. What follow adds is that a process which
    /// leaves the visible set takes the cursor with it rather than
    /// stranding it on whatever moved into that row.
    fn keep_followed(&mut self) {
        if !self.follow {
            return;
        }
        let Some(pid) = self.selected else { return };
        if self.visible_rows().iter().any(|r| r.pid == pid) {
            return;
        }
        // It is gone: stop following rather than jumping somewhere else,
        // and say so through the tab row's state list.
        self.follow = false;
    }

    /// The body rows a table in `inner` shows on the grid (§8.1's budget).
    fn page_rows(&self, inner_height: u16) -> usize {
        usize::from(inner_height.saturating_sub(ROWS_ABOVE_TABLE + 1))
            .min(usize::from(self.options.table_rows))
            .max(1)
    }

    /// Whether the `full` tier is drawing its search/filter line, which costs
    /// a row. Both `full::render` and `full_body_rows` ask this rather than
    /// each deciding for itself.
    pub fn has_search_line(&self) -> bool {
        match (self.typing(), self.search(), self.filter()) {
            (Some(Typing::Search), Some(_), _) | (Some(Typing::Filter), _, Some(_)) => true,
            (None, _, Some(f)) => !f.is_empty(),
            (None, Some(s), _) => !s.is_empty(),
            _ => false,
        }
    }

    /// The rows the **zoomed** `full` table actually draws: the tabs row, the
    /// search line when one is open, the column header and the F-key bar come
    /// off `inner`. `full::render` derives its own `top` from this same
    /// number, so a page that computed the budget independently would drift by
    /// a row the moment the search line opened (D60).
    pub(crate) fn full_body_rows(&self, inner_height: u16) -> usize {
        let chrome = 3 + usize::from(self.has_search_line());
        usize::from(inner_height).saturating_sub(chrome).max(1)
    }

    /// The page a `PgUp`/`PgDn` moves, which is what is on screen — the grid's
    /// top-N budget in a grid tile, the drawn body in the zoomed `full` tier.
    /// Asks the shell which tier is being drawn rather than comparing sizes:
    /// a 6x3 tile on a large screen clears `full`'s minimum (D58 amendment 7).
    fn keys_page_rows(&self, cx: &InputCx<'_>) -> usize {
        if cx.tier >= TIER_FULL {
            self.full_body_rows(cx.inner.height)
        } else {
            self.page_rows(cx.inner.height)
        }
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
            .rebuild(&table, &self.options, self.threads(), self.sort, self.desc);
        self.rebuild_visible();
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
    build: Box::new(build),
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
        // `Detail::Columns` is htop's gated files — `/proc/<pid>/io` and
        // the `task/` walk — and it costs a read per process. Only the
        // zoomed `full` tier asks, and only when a person switched one of
        // them on (D58 seam 3; the first user of `Columns`).
        if tier >= TIER_FULL && (self.show_userland || self.screen == Screen::Io) {
            Detail::Columns
        } else if tier >= TIER_TABLE {
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
            .rebuild(table, &self.options, self.threads(), self.sort, self.desc);
        self.rebuild_visible();
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
        let rows = self.keys_page_rows(cx);
        let page = rows as isize;
        self.keep_followed();
        // The zoom-only tier's keys, which the grid tiers never see: a
        // 4x2 htop tile is a dashboard, not a process manager (§4.6).
        // This asks the shell which tier is being *drawn* — a 6x3 tile on
        // a large screen is 122x31, which clears the `full` tier's
        // minimum, so a size comparison had these keys (renice included)
        // answering on the grid where none of their chrome was drawn
        // (arc 8a review, D58 amendment 7).
        if cx.tier >= TIER_FULL {
            match self.full_key(key, cx) {
                Outcome::Ignored => {}
                other => return other,
            }
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
            TIER_TABLE => &["CCD", "kthr", "PID", "CPU%", "Command"],
            // The `full` tier is the tool, not the dashboard: the meters
            // are gone and the tabs, the table and the F-key bar are what
            // every store shows (the columns differ by screen, so `PID`
            // and `Command` are the two that always stand).
            _ => &["Main", "I/O", "PID", "Command", "F9"],
        }
    }
}
