//! The gpu process table (§8.1): nvtop 3.2.0's columns and widths, the join
//! with `proc.table` by PID with a per-PID last-known cache, gridwatch's drop
//! order, sort and selection keyed by PID. `tick` derives; `view` lays out.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use gridwatch_store::keys::cpu::ProcTable;
use gridwatch_store::keys::gpu::{GpuProcKind, GpuProcs};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{ColWidth, Column, Line, SortDir, Span, View};

use super::format as fmt;

/// nvtop's process columns; `Dev` is auto-hidden with one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Pid,
    User,
    Dev,
    Type,
    Gpu,
    Enc,
    Dec,
    GpuMem,
    Cpu,
    HostMem,
    Command,
}

impl Col {
    pub fn from_id(id: &str) -> Option<Col> {
        Some(match id {
            "pid" => Col::Pid,
            "user" => Col::User,
            "dev" => Col::Dev,
            "type" => Col::Type,
            "gpu" => Col::Gpu,
            "enc" => Col::Enc,
            "dec" => Col::Dec,
            "gpu_mem" => Col::GpuMem,
            "cpu" => Col::Cpu,
            "host_mem" => Col::HostMem,
            "command" => Col::Command,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Col::Pid => "pid",
            Col::User => "user",
            Col::Dev => "dev",
            Col::Type => "type",
            Col::Gpu => "gpu",
            Col::Enc => "enc",
            Col::Dec => "dec",
            Col::GpuMem => "gpu_mem",
            Col::Cpu => "cpu",
            Col::HostMem => "host_mem",
            Col::Command => "command",
        }
    }

    /// nvtop's header text.
    pub fn title(self) -> &'static str {
        match self {
            Col::Pid => "PID",
            Col::User => "USER",
            Col::Dev => "DEV",
            Col::Type => "TYPE",
            Col::Gpu => "GPU",
            Col::Enc => "ENC",
            Col::Dec => "DEC",
            Col::GpuMem => "GPU MEM",
            Col::Cpu => "CPU",
            Col::HostMem => "HOST MEM",
            Col::Command => "Command",
        }
    }

    fn right(self) -> bool {
        !matches!(self, Col::User | Col::Type | Col::Command)
    }

    fn descending_first(self) -> bool {
        !matches!(self, Col::User | Col::Type | Col::Command | Col::Pid)
    }

    /// nvtop's `sizeof_process_field` (the separator is added by the table).
    fn width(self, user_w: u16) -> u16 {
        match self {
            Col::Pid => 7,
            Col::User => user_w,
            Col::Dev => 3,
            Col::Type => 8,
            Col::Gpu | Col::Enc | Col::Dec => 4,
            Col::GpuMem => 14,
            Col::Cpu => 6,
            Col::HostMem => 9,
            Col::Command => 0,
        }
    }
}

/// Gridwatch's drop order when `Command` would fall below `command_min`
/// (§8.1): `PID`, `GPU`, `GPU MEM` and `Command` always survive.
const DROP_ORDER: [Col; 6] = [
    Col::Enc,
    Col::Dec,
    Col::HostMem,
    Col::Type,
    Col::User,
    Col::Cpu,
];

/// One joined row: the gpu source's numbers plus what `proc.table` knows
/// about the PID, or the last thing it knew.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub pid: i32,
    pub dev: u16,
    pub kind: GpuProcKind,
    pub vram_b: Option<u64>,
    pub sm_pct: u32,
    pub enc_pct: u32,
    pub dec_pct: u32,
    pub fresh: bool,
    pub user: Option<Arc<str>>,
    pub cpu_pct: Option<f32>,
    pub res_kib: Option<u64>,
    pub cmdline: Option<Arc<str>>,
}

/// The rows the table draws, derived once per generation in `tick`.
#[derive(Debug, Default)]
pub struct Derived {
    pub rows: Vec<Row>,
    pub vram_total_b: u64,
    /// nvtop's USER width: max(4, longest visible name).
    pub user_w: u16,
    /// Per-PID last-known `user`/`cmdline` (nvtop's cache): a process the scan
    /// has not listed yet, or no longer lists, keeps what was seen.
    cache: BTreeMap<i32, (Arc<str>, Arc<str>)>,
}

impl Derived {
    /// Join `gpu.procs` with `proc.table` (absent → `—` columns), then sort.
    pub fn rebuild(
        &mut self,
        procs: &GpuProcs,
        dev: u16,
        table: Option<&ProcTable>,
        sort: Col,
        desc: bool,
    ) {
        self.vram_total_b = procs.vram_total_b;
        if let Some(t) = table {
            for r in &t.rows {
                if r.tgid == r.pid {
                    self.cache
                        .insert(r.pid, (r.user.clone(), r.cmdline.clone()));
                }
            }
        }
        self.rows = procs
            .rows
            .iter()
            .map(|g| {
                let live = table.and_then(|t| t.rows.iter().find(|r| r.pid == g.pid));
                let cached = self.cache.get(&g.pid);
                Row {
                    pid: g.pid,
                    dev,
                    kind: g.kind,
                    vram_b: g.vram_b,
                    sm_pct: g.sm_pct,
                    enc_pct: g.enc_pct,
                    dec_pct: g.dec_pct,
                    fresh: g.fresh,
                    user: cached.map(|(u, _)| u.clone()),
                    cpu_pct: live.map(|r| r.cpu_pct),
                    res_kib: live.map(|r| r.res_kib),
                    cmdline: cached.map(|(_, c)| c.clone()),
                }
            })
            .collect();
        // Forget PIDs that are gone from both sides so the cache cannot grow
        // without bound across a day of processes.
        let keep: std::collections::BTreeSet<i32> = self
            .rows
            .iter()
            .map(|r| r.pid)
            .chain(table.into_iter().flat_map(|t| t.rows.iter().map(|r| r.pid)))
            .collect();
        self.cache.retain(|pid, _| keep.contains(pid));
        self.user_w = self
            .rows
            .iter()
            .filter_map(|r| r.user.as_ref().map(|u| u.chars().count() as u16))
            .max()
            .unwrap_or(0)
            .max(4);
        sort_rows(&mut self.rows, sort, desc);
    }
}

/// Re-sort already-joined rows (a key press changed the sort).
pub fn sort_for(rows: &mut [Row], col: Col, desc: bool) {
    sort_rows(rows, col, desc);
}

fn sort_rows(rows: &mut [Row], col: Col, desc: bool) {
    rows.sort_by(|a, b| {
        let o = match col {
            Col::Pid | Col::Dev => a.pid.cmp(&b.pid),
            Col::User => a.user.cmp(&b.user),
            Col::Type => kind_rank(a.kind).cmp(&kind_rank(b.kind)),
            Col::Gpu => a.sm_pct.cmp(&b.sm_pct),
            Col::Enc => a.enc_pct.cmp(&b.enc_pct),
            Col::Dec => a.dec_pct.cmp(&b.dec_pct),
            Col::GpuMem => a.vram_b.cmp(&b.vram_b),
            Col::Cpu => a
                .cpu_pct
                .unwrap_or(-1.0)
                .total_cmp(&b.cpu_pct.unwrap_or(-1.0)),
            Col::HostMem => a.res_kib.cmp(&b.res_kib),
            Col::Command => a.cmdline.cmp(&b.cmdline),
        };
        let o = if desc { o.reverse() } else { o };
        // §8.1: a process that only holds a context (sm 0) sorts below an
        // active one at equal memory; then PID for a stable order.
        o.then_with(|| b.sm_pct.min(1).cmp(&a.sm_pct.min(1)))
            .then(a.pid.cmp(&b.pid))
    });
}

fn kind_rank(k: GpuProcKind) -> u8 {
    match k {
        GpuProcKind::Graphics => 0,
        GpuProcKind::Compute => 1,
        GpuProcKind::Both => 2,
    }
}

pub fn default_dir(col: Col) -> bool {
    col.descending_first()
}

/// Which columns survive at `width`: the enabled set in nvtop's order, `DEV`
/// only with several devices, minus the drop order's head until `Command`
/// keeps `command_min` cells.
pub fn fit_columns(
    enabled: &[Col],
    width: u16,
    command_min: u16,
    user_w: u16,
    devices: usize,
) -> Vec<Col> {
    let mut cols: Vec<Col> = enabled
        .iter()
        .copied()
        .filter(|c| *c != Col::Dev || devices > 1)
        .collect();
    if !cols.contains(&Col::Command) {
        cols.push(Col::Command);
    }
    let fixed = |cols: &[Col]| -> u16 {
        cols.iter().map(|c| c.width(user_w)).sum::<u16>() + cols.len().saturating_sub(1) as u16
    };
    for victim in DROP_ORDER {
        if width.saturating_sub(fixed(&cols)) >= command_min {
            break;
        }
        cols.retain(|c| *c != victim);
    }
    cols
}

fn kind_cell(k: GpuProcKind) -> Line {
    match k {
        // nvtop: Graphic yellow → Warn, Compute magenta → AccentTertiary.
        GpuProcKind::Graphics => vec![Span::new(Role::Warn, "Graphic")],
        GpuProcKind::Compute => vec![Span::new(Role::AccentTertiary, "Compute")],
        GpuProcKind::Both => vec![
            Span::new(Role::Text, "Both "),
            Span::new(Role::Warn, "G"),
            Span::new(Role::Text, "+"),
            Span::new(Role::AccentTertiary, "C"),
        ],
    }
}

fn cell(row: &Row, col: Col, total_b: u64) -> Line {
    let muted_if_stale = |s: String| -> Span {
        Span::new(
            if row.fresh {
                Role::Text
            } else {
                Role::TextMuted
            },
            s,
        )
    };
    match col {
        Col::Pid => vec![Span::new(Role::Text, format!("{:>7}", row.pid))],
        Col::User => vec![match &row.user {
            Some(u) => Span::new(Role::Text, u.to_string()),
            None => Span::new(Role::TextGhost, "—"),
        }],
        Col::Dev => vec![Span::new(Role::TextMuted, format!("{:>3}", row.dev))],
        Col::Type => kind_cell(row.kind),
        Col::Gpu => vec![muted_if_stale(fmt::pct(Some(f64::from(row.sm_pct))))],
        Col::Enc => vec![muted_if_stale(fmt::pct(Some(f64::from(row.enc_pct))))],
        Col::Dec => vec![muted_if_stale(fmt::pct(Some(f64::from(row.dec_pct))))],
        Col::GpuMem => vec![Span::new(Role::Text, fmt::gpu_mem(row.vram_b, total_b))],
        Col::Cpu => vec![Span::new(
            if row.cpu_pct.is_some() {
                Role::Text
            } else {
                Role::TextGhost
            },
            fmt::cpu(row.cpu_pct),
        )],
        Col::HostMem => vec![Span::new(
            if row.res_kib.is_some() {
                Role::Text
            } else {
                Role::TextGhost
            },
            fmt::host_mem(row.res_kib),
        )],
        Col::Command => vec![match &row.cmdline {
            Some(c) if !c.is_empty() => Span::new(Role::Text, c.to_string()),
            // Nothing was ever read for the PID (§8.1): `[pid]`, muted.
            _ => Span::new(Role::TextMuted, format!("[{}]", row.pid)),
        }],
    }
}

/// The table view for `body_rows` lines; `selected` is a PID.
#[allow(clippy::too_many_arguments)] // the tier's whole state, read-only
pub fn view(
    d: &Derived,
    width: u16,
    body_rows: usize,
    command_min: u16,
    sort: Col,
    desc: bool,
    selected: Option<i32>,
    scroll: usize,
    enabled: &[Col],
    devices: usize,
) -> View {
    let cols = fit_columns(enabled, width, command_min, d.user_w, devices);
    let columns: Vec<Column> = cols
        .iter()
        .map(|c| Column {
            title: c.title().into(),
            width: match c {
                Col::Command => ColWidth::Elastic,
                other => ColWidth::Fixed(other.width(d.user_w)),
            },
            right: c.right(),
        })
        .collect();
    let sel_idx = selected.and_then(|pid| d.rows.iter().position(|r| r.pid == pid));
    let max_scroll = d.rows.len().saturating_sub(body_rows);
    let mut scroll = scroll.min(max_scroll);
    if let Some(i) = sel_idx {
        if i < scroll {
            scroll = i;
        } else if body_rows > 0 && i >= scroll + body_rows {
            scroll = i + 1 - body_rows;
        }
    }
    let rows: Vec<Vec<Line>> = d
        .rows
        .iter()
        .skip(scroll)
        .take(body_rows)
        .map(|r| cols.iter().map(|c| cell(r, *c, d.vram_total_b)).collect())
        .collect();
    View::Table {
        columns,
        rows,
        selected: sel_idx.map(|i| i - scroll),
        sort: cols
            .iter()
            .position(|c| *c == sort)
            .map(|i| (i, if desc { SortDir::Desc } else { SortDir::Asc })),
        scroll: 0,
    }
}

/// Ordering helper exposed for tests of the tie rule.
pub fn compare_for_tests(a: &Row, b: &Row, col: Col, desc: bool) -> Ordering {
    let mut v = vec![a.clone(), b.clone()];
    sort_rows(&mut v, col, desc);
    if v[0] == *a {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}
