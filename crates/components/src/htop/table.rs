//! The htop process table (§8.1): one column model, htop's printed widths,
//! gridwatch's drop order, sort and selection keyed by PID. `tick` derives the
//! sorted rows once per source generation; `view` only lays them out.

use std::cmp::Ordering;

use gridwatch_store::keys::cpu::{ProcRow, ProcTable};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{ColWidth, Column, Line, SortDir, Span, View};

use super::Options;
use super::format as fmt;

/// Every column htop's Main screen has that the grid can show today (§8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Pid,
    User,
    Pri,
    Nice,
    Virt,
    Res,
    Shr,
    State,
    Cpu,
    Mem,
    Time,
    Command,
}

impl Col {
    /// The identifiers `columns` and `sort` accept (`htop::SORT_KEYS`).
    pub fn from_id(id: &str) -> Option<Col> {
        Some(match id {
            "pid" => Col::Pid,
            "user" => Col::User,
            "pri" => Col::Pri,
            "nice" => Col::Nice,
            "virt" => Col::Virt,
            "res" => Col::Res,
            "shr" => Col::Shr,
            "state" => Col::State,
            "cpu" => Col::Cpu,
            "mem" => Col::Mem,
            "time" => Col::Time,
            "command" => Col::Command,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Col::Pid => "pid",
            Col::User => "user",
            Col::Pri => "pri",
            Col::Nice => "nice",
            Col::Virt => "virt",
            Col::Res => "res",
            Col::Shr => "shr",
            Col::State => "state",
            Col::Cpu => "cpu",
            Col::Mem => "mem",
            Col::Time => "time",
            Col::Command => "command",
        }
    }

    /// htop's header text.
    pub fn title(self) -> &'static str {
        match self {
            Col::Pid => "PID",
            Col::User => "USER",
            Col::Pri => "PRI",
            Col::Nice => "NI",
            Col::Virt => "VIRT",
            Col::Res => "RES",
            Col::Shr => "SHR",
            Col::State => "S",
            Col::Cpu => "CPU%",
            Col::Mem => "MEM%",
            Col::Time => "TIME+",
            Col::Command => "Command",
        }
    }

    fn right(self) -> bool {
        !matches!(self, Col::User | Col::Command)
    }

    /// Numeric columns sort descending first (htop's default for them).
    fn descending_first(self) -> bool {
        !matches!(self, Col::User | Col::State | Col::Command)
    }

    /// Printed width without the separator (§8.1's table minus one).
    fn width(self, pid_digits: u8, cpu_w: u16) -> u16 {
        match self {
            Col::Pid => u16::from(pid_digits),
            Col::User => 10,
            Col::Pri | Col::Nice => 3,
            Col::Virt | Col::Res | Col::Shr => 5,
            Col::State => 1,
            Col::Cpu => cpu_w,
            Col::Mem => 4,
            Col::Time => 8,
            Col::Command => 0,
        }
    }
}

/// Gridwatch's drop order when `Command` would fall below `command_min`
/// (§8.1): htop never drops, it scrolls; the grid has no room to.
const DROP_ORDER: [Col; 11] = [
    Col::Virt,
    Col::Shr,
    Col::Pri,
    Col::Nice,
    Col::Time,
    Col::User,
    Col::Res,
    Col::Pid,
    Col::State,
    Col::Mem,
    Col::Cpu,
];

/// The rows the table draws, derived once per generation in `tick`.
#[derive(Debug, Default)]
pub struct Derived {
    pub rows: Vec<ProcRow>,
    pub pid_digits: u8,
    /// Auto-width CPU% (§8.1): 4, growing to `ceil(log10(max + 0.1)) + 2`.
    /// Recomputed per table like htop, whose `Row_resetFieldWidths` runs at
    /// the top of every scan cycle — a burst widens the column while it lasts.
    pub cpu_w: u16,
}

impl Derived {
    pub fn rebuild(&mut self, table: &ProcTable, o: &Options, sort: Col, desc: bool) {
        self.pid_digits = table.pid_digits.clamp(5, 19);
        self.rows = table
            .rows
            .iter()
            .filter(|r| !(o.hide_kernel_threads && r.kthread))
            .filter(|r| !(o.hide_userland_threads && r.tgid != r.pid))
            .cloned()
            .collect();
        let max_cpu = self.rows.iter().map(|r| r.cpu_pct).fold(0.0f32, f32::max);
        let want = if max_cpu < 99.9 {
            4
        } else {
            (f64::from(max_cpu + 0.1).log10().ceil() as u16) + 2
        };
        self.cpu_w = want.max(4);
        sort_rows(&mut self.rows, sort, desc);
    }
}

fn sort_rows(rows: &mut [ProcRow], col: Col, desc: bool) {
    let cmp = |a: &ProcRow, b: &ProcRow| -> Ordering {
        let o = match col {
            Col::Pid => a.pid.cmp(&b.pid),
            Col::User => a.user.cmp(&b.user),
            Col::Pri => a.pri.cmp(&b.pri),
            Col::Nice => a.nice.cmp(&b.nice),
            Col::Virt => a.virt_kib.cmp(&b.virt_kib),
            Col::Res => a.res_kib.cmp(&b.res_kib),
            Col::Shr => a.shr_kib.cmp(&b.shr_kib),
            Col::State => a.state.cmp(&b.state),
            Col::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            Col::Mem => a.mem_pct.total_cmp(&b.mem_pct),
            Col::Time => a.time_cs.cmp(&b.time_cs),
            Col::Command => a.cmdline.cmp(&b.cmdline),
        };
        let o = if desc { o.reverse() } else { o };
        // Ties by PID so a re-sort is stable across generations.
        o.then(a.pid.cmp(&b.pid))
    };
    rows.sort_by(cmp);
}

/// The sort direction a column starts in.
pub fn default_dir(col: Col) -> bool {
    col.descending_first()
}

/// Which columns survive at `width`: the enabled set in htop's order, minus
/// the drop order's head until `Command` keeps `command_min` cells.
pub fn fit_columns(
    enabled: &[Col],
    width: u16,
    command_min: u16,
    pid_digits: u8,
    cpu_w: u16,
) -> Vec<Col> {
    let mut cols: Vec<Col> = enabled.to_vec();
    if !cols.contains(&Col::Command) {
        cols.push(Col::Command);
    }
    let fixed = |cols: &[Col]| -> u16 {
        cols.iter().map(|c| c.width(pid_digits, cpu_w)).sum::<u16>()
            + cols.len().saturating_sub(1) as u16
    };
    for victim in DROP_ORDER {
        if width.saturating_sub(fixed(&cols)) >= command_min {
            break;
        }
        cols.retain(|c| *c != victim);
    }
    cols
}

fn cell(row: &ProcRow, col: Col, cpu_w: u16, pid_digits: u8) -> Line {
    match col {
        Col::Pid => vec![Span::new(
            Role::Text,
            format!("{:>w$}", row.pid, w = usize::from(pid_digits)),
        )],
        Col::User => vec![Span::new(
            if row.uid == 0 {
                Role::TextMuted
            } else {
                Role::Text
            },
            row.user.chars().take(10).collect::<String>(),
        )],
        Col::Pri => vec![Span::new(
            Role::Text,
            if row.pri <= -100 {
                " RT".to_string()
            } else {
                format!("{:3}", row.pri)
            },
        )],
        Col::Nice => vec![Span::new(
            match row.nice.cmp(&0) {
                Ordering::Less => Role::Crit,
                Ordering::Greater => Role::Ok,
                Ordering::Equal => Role::TextMuted,
            },
            format!("{:3}", row.nice),
        )],
        Col::Virt => fmt::kbytes(Some(row.virt_kib)),
        Col::Res => fmt::kbytes(Some(row.res_kib)),
        Col::Shr => fmt::kbytes(Some(row.shr_kib)),
        Col::State => vec![fmt::state(row.state)],
        Col::Cpu => vec![fmt::percentage(row.cpu_pct, usize::from(cpu_w))],
        Col::Mem => vec![fmt::percentage(row.mem_pct, 4)],
        Col::Time => fmt::time_plus(row.time_cs),
        Col::Command => vec![Span::new(
            if row.kthread || row.tgid != row.pid {
                Role::AccentTertiary
            } else {
                Role::Text
            },
            row.cmdline.to_string(),
        )],
    }
}

/// The table view for `rows` visible lines. `selected` is a PID; `scroll`
/// is the component's first visible row, clamped here so the selection is
/// always on screen and the page never scrolls past its end.
#[allow(clippy::too_many_arguments)] // the tier's whole state, read-only
pub fn view(
    d: &Derived,
    o: &Options,
    width: u16,
    body_rows: usize,
    sort: Col,
    desc: bool,
    selected: Option<i32>,
    scroll: usize,
    enabled: &[Col],
) -> View {
    let cols = fit_columns(enabled, width, o.command_min, d.pid_digits, d.cpu_w);
    let columns: Vec<Column> = cols
        .iter()
        .map(|c| Column {
            title: c.title().into(),
            width: match c {
                Col::Command => ColWidth::Elastic,
                other => ColWidth::Fixed(other.width(d.pid_digits, d.cpu_w)),
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
        .map(|r| {
            cols.iter()
                .map(|c| cell(r, *c, d.cpu_w, d.pid_digits))
                .collect()
        })
        .collect();
    View::Table {
        columns,
        rows,
        selected: sel_idx.map(|i| i - scroll),
        sort: cols
            .iter()
            .position(|c| *c == sort)
            .map(|i| (i, if desc { SortDir::Desc } else { SortDir::Asc })),
        // Rows are pre-sliced, so the renderer's own scroll stays at 0 and the
        // selected index is relative to what is shown.
        scroll: 0,
    }
}
