//! htop's whole face: the zoom-only `full` tier (§8, arc 8a).
//!
//! What the grid tiers show is a dashboard — meters, cores, a top-N table.
//! This is the tool: two screens as tabs, the tree, incremental search, a
//! filter, tags, follow, both thread toggles, and the F-key bar that tells
//! a person what the function keys do. The three pickers (`F9`, `a`, `i`)
//! draw here too; the actions they build are the shell's to run.

use gridwatch_store::keys::cpu;
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{ColWidth, Column, Constraint, Dir, Line, Span, View};

use super::format::{kbytes, percentage, state, time_plus};
use super::{Htop, IO_CLASSES, Menu, SIGNALS, Screen, Typing};

/// The function keys, in htop's order. `F1`/`F2`/`F3`/`F4` are its help,
/// setup, search and filter; the ones this tile answers are marked.
const FKEYS: &[(&str, &str, bool)] = &[
    ("F5", "tree", true),
    ("F6", "sort", true),
    ("F7", "nice −", true),
    ("F8", "nice +", true),
    ("F9", "kill", true),
    ("F10", "quit", false),
];

fn bps(v: f32) -> String {
    if v <= 0.0 {
        return "—".into();
    }
    const UNITS: [&str; 4] = ["B", "K", "M", "G"];
    let mut x = f64::from(v);
    let mut i = 0;
    while x >= 1000.0 && i + 1 < UNITS.len() {
        x /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{x:.0}{}", UNITS[i])
    } else {
        format!("{x:.1}{}", UNITS[i])
    }
}

/// The tab row: which screen, and what is switched on.
fn tabs(h: &Htop) -> Line {
    let mut line: Line = Vec::new();
    for s in [Screen::Main, Screen::Io] {
        let on = h.screen() == s;
        line.push(Span::new(
            if on {
                Role::AccentPrimary
            } else {
                Role::TextMuted
            },
            format!(" {} ", s.name()),
        ));
    }
    line.push(Span::new(Role::TextGhost, "  Tab switches  "));
    let mut state: Vec<&str> = Vec::new();
    if h.tree() {
        state.push("tree");
    }
    if h.follow() {
        state.push("follow");
    }
    if h.show_userland() {
        state.push("threads");
    }
    if h.show_kernel() {
        state.push("kernel");
    }
    if !state.is_empty() {
        line.push(Span::new(Role::Ok, state.join(" · ")));
    }
    if !h.tags().is_empty() {
        line.push(Span::new(
            Role::AccentSecondary,
            format!("  {} tagged", h.tags().len()),
        ));
    }
    line
}

/// The line under the tabs: whatever is being typed, or what is in force.
pub(super) fn search_line(h: &Htop) -> Option<Line> {
    match (h.typing(), h.search(), h.filter()) {
        (Some(Typing::Search), Some(s), _) => Some(vec![
            Span::new(Role::AccentPrimary, "search: "),
            Span::bold(Role::Text, s.to_string()),
            Span::new(Role::TextGhost, "▏"),
        ]),
        (Some(Typing::Filter), _, Some(f)) => Some(vec![
            Span::new(Role::AccentSecondary, "filter: "),
            Span::bold(Role::Text, f.to_string()),
            Span::new(Role::TextGhost, "▏"),
        ]),
        (None, _, Some(f)) if !f.is_empty() => Some(vec![
            Span::new(Role::TextMuted, "filtered by "),
            Span::bold(Role::Text, f.to_string()),
            Span::new(Role::TextGhost, "  (\\ to change, Esc to clear)"),
        ]),
        (None, Some(s), _) if !s.is_empty() => Some(vec![
            Span::new(Role::TextMuted, "searched for "),
            Span::bold(Role::Text, s.to_string()),
        ]),
        _ => None,
    }
}

/// The F-key bar. It is a `View::Custom`-free row of spans through theme
/// roles: the key in reverse-ish accent, the word in muted text.
fn fkey_bar(h: &Htop) -> Line {
    let mut line: Line = Vec::new();
    for (key, what, ours) in FKEYS {
        let what = if *key == "F5" && h.tree() {
            "list"
        } else {
            what
        };
        line.push(Span::new(Role::TextGhost, format!("{key} ")));
        line.push(Span::new(
            if *ours { Role::Text } else { Role::TextMuted },
            format!("{what}  "),
        ));
    }
    line.push(Span::new(
        Role::TextGhost,
        "/ search · \\ filter · Space tag · U untag · F follow · K kernel · H threads · a affinity · i io",
    ));
    line
}

/// One picker, drawn where the table would be.
fn menu_view(h: &Htop, menu: &Menu) -> View {
    let targets = h.action_targets();
    let who = match targets.as_slice() {
        [(pid, name)] => format!("{name} ({pid})"),
        many => format!("{} tagged processes", many.len()),
    };
    let (title, items, at, chosen): (String, Vec<String>, usize, Vec<usize>) = match menu {
        Menu::Signal { at } => (
            format!("send a signal to {who}"),
            SIGNALS.iter().map(|(n, _)| (*n).to_string()).collect(),
            *at,
            Vec::new(),
        ),
        Menu::IoPrio { at } => (
            format!("I/O priority for {who}"),
            IO_CLASSES.iter().map(|(n, _)| (*n).to_string()).collect(),
            *at,
            Vec::new(),
        ),
        Menu::Affinity { at, chosen, cpus } => (
            format!("which CPUs may run {who}"),
            (0..*cpus).map(|c| format!("cpu {c}")).collect(),
            *at,
            chosen.iter().copied().collect(),
        ),
    };
    let mut lines: Vec<Line> = vec![
        vec![Span::bold(Role::AccentPrimary, title)],
        vec![Span::new(
            Role::TextGhost,
            match menu {
                Menu::Affinity { .. } => "↑/↓ move · Space choose · Enter apply · Esc cancel",
                _ => "↑/↓ move · Enter apply · Esc cancel",
            },
        )],
        Vec::new(),
    ];
    for (i, item) in items.iter().enumerate() {
        let cursor = i == at;
        let mut line: Line = vec![Span::new(
            if cursor {
                Role::AccentPrimary
            } else {
                Role::TextGhost
            },
            if cursor { "▸ " } else { "  " },
        )];
        if matches!(menu, Menu::Affinity { .. }) {
            line.push(Span::new(
                if chosen.contains(&i) {
                    Role::Ok
                } else {
                    Role::TextMuted
                },
                if chosen.contains(&i) { "[x] " } else { "[ ] " },
            ));
        }
        line.push(Span::new(
            if cursor { Role::Text } else { Role::TextMuted },
            item.clone(),
        ));
        lines.push(line);
    }
    View::Text(lines)
}

/// The Main screen's columns, or the I/O screen's.
fn columns(screen: Screen, width: u16) -> Vec<Column> {
    let mut cols = vec![Column {
        title: "PID".into(),
        width: ColWidth::Fixed(7),
        right: true,
    }];
    if width >= 90 {
        cols.push(Column {
            title: "USER".into(),
            width: ColWidth::Fixed(9),
            right: false,
        });
    }
    match screen {
        Screen::Main => {
            cols.push(Column {
                title: "PRI".into(),
                width: ColWidth::Fixed(4),
                right: true,
            });
            cols.push(Column {
                title: "NI".into(),
                width: ColWidth::Fixed(3),
                right: true,
            });
            cols.push(Column {
                title: "VIRT".into(),
                width: ColWidth::Fixed(6),
                right: true,
            });
            cols.push(Column {
                title: "RES".into(),
                width: ColWidth::Fixed(6),
                right: true,
            });
            cols.push(Column {
                title: "S".into(),
                width: ColWidth::Fixed(1),
                right: false,
            });
            cols.push(Column {
                title: "CPU%".into(),
                width: ColWidth::Fixed(5),
                right: true,
            });
            cols.push(Column {
                title: "MEM%".into(),
                width: ColWidth::Fixed(5),
                right: true,
            });
            cols.push(Column {
                title: "TIME+".into(),
                width: ColWidth::Fixed(9),
                right: true,
            });
        }
        Screen::Io => {
            cols.push(Column {
                title: "RD/s".into(),
                width: ColWidth::Fixed(7),
                right: true,
            });
            cols.push(Column {
                title: "WR/s".into(),
                width: ColWidth::Fixed(7),
                right: true,
            });
            cols.push(Column {
                title: "IO".into(),
                width: ColWidth::Fixed(4),
                right: false,
            });
        }
    }
    cols.push(Column {
        title: "Command".into(),
        width: ColWidth::Elastic,
        right: false,
    });
    cols
}

fn row_cells(
    h: &Htop,
    r: &cpu::ProcRow,
    depth: usize,
    screen: Screen,
    width: u16,
    needle: Option<&str>,
) -> Vec<Line> {
    let tagged = h.tags().contains(&r.pid);
    let name_role = if tagged {
        Role::AccentSecondary
    } else {
        Role::Text
    };
    let mut cells: Vec<Line> = vec![vec![Span::new(
        if tagged {
            Role::AccentSecondary
        } else {
            Role::TextMuted
        },
        r.pid.to_string(),
    )]];
    if width >= 90 {
        cells.push(vec![Span::new(Role::TextMuted, r.user.to_string())]);
    }
    match screen {
        Screen::Main => {
            cells.push(vec![Span::new(Role::TextMuted, r.pri.to_string())]);
            cells.push(vec![Span::new(Role::TextMuted, r.nice.to_string())]);
            cells.push(kbytes(Some(r.virt_kib)));
            cells.push(kbytes(Some(r.res_kib)));
            cells.push(vec![state(r.state)]);
            cells.push(vec![percentage(r.cpu_pct, 5)]);
            cells.push(vec![percentage(r.mem_pct, 5)]);
            cells.push(time_plus(r.time_cs));
        }
        Screen::Io => {
            cells.push(vec![Span::new(Role::Text, bps(r.read_bps))]);
            cells.push(vec![Span::new(Role::Text, bps(r.write_bps))]);
            // A row whose `io` file was not ours to read says so, rather
            // than showing two zeroes as if it were idle.
            cells.push(vec![Span::new(
                if r.io_readable {
                    Role::Ok
                } else {
                    Role::TextGhost
                },
                if r.io_readable { "ok" } else { "n/a" },
            )]);
        }
    }
    // The command, indented by its depth in the tree, and marked when it
    // matches the search.
    let indent = "  ".repeat(depth.min(8));
    let text = if r.cmdline.is_empty() {
        r.comm.to_string()
    } else {
        r.cmdline.to_string()
    };
    let hit =
        needle.is_some_and(|n| !n.is_empty() && text.to_lowercase().contains(&n.to_lowercase()));
    cells.push(vec![
        Span::new(Role::TextGhost, indent),
        Span::new(if hit { Role::AccentPrimary } else { name_role }, text),
    ]);
    cells
}

pub fn render(h: &Htop, cx: &RenderCx<'_>) -> View {
    let mut children: Vec<(Constraint, View)> =
        vec![(Constraint::Len(1), View::Text(vec![tabs(h)]))];
    if let Some(line) = search_line(h) {
        children.push((Constraint::Len(1), View::Text(vec![line])));
    }
    // A menu takes the body: it is a question, and the table behind it is
    // not what a person is answering.
    if let Some(menu) = h.menu() {
        children.push((Constraint::Fill(1), menu_view(h, menu)));
        children.push((Constraint::Len(1), View::Text(vec![fkey_bar(h)])));
        return View::Stack {
            dir: Dir::V,
            children,
        };
    }
    let rows = h.visible_rows();
    // One method owns this budget; `on_key` reads the same one, so `PgDn`
    // moves exactly what is drawn even with the search line open (D60).
    let body = h.full_body_rows(cx.inner.height);
    debug_assert_eq!(
        body,
        usize::from(cx.inner.height)
            .saturating_sub(children.len() + 2)
            .max(1)
    );
    let cursor = h
        .selected()
        .and_then(|pid| rows.iter().position(|r| r.pid == pid));
    let top = match cursor {
        Some(i) if i >= body => (i + 1 - body).min(rows.len().saturating_sub(body.min(rows.len()))),
        _ => 0,
    };
    let table = if rows.is_empty() {
        View::Text(vec![vec![Span::new(
            Role::TextGhost,
            match h.filter() {
                Some(f) if !f.is_empty() => format!("nothing matches “{f}”"),
                _ => "waiting for the process scan…".to_string(),
            },
        )]])
    } else {
        View::Table {
            columns: columns(h.screen(), cx.inner.width),
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    // The depth was computed in `tick`; `view` only reads
                    // it (§8.1: `view` never sorts).
                    let depth = h.depths().get(i).copied().unwrap_or(0);
                    row_cells(h, r, depth, h.screen(), cx.inner.width, h.search())
                })
                .collect(),
            selected: cursor,
            sort: None,
            scroll: top,
        }
    };
    children.push((Constraint::Fill(1), table));
    children.push((Constraint::Len(1), View::Text(vec![fkey_bar(h)])));
    View::Stack {
        dir: Dir::V,
        children,
    }
}
