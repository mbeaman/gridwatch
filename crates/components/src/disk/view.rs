//! The disk tiers as view trees (§8): `rates` (the summed pair and a busy
//! chip), `sparks` (+ read and write sparklines), `table` (+ a row per drive,
//! widening through the column-drop order), `chart` (+ a braille chart of the
//! selected series and the service times) and the zoom-only `full` (+ the
//! per-drive pane and the sentence saying what `busy` cannot tell you).

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::disk;
use gridwatch_store::{Agg, SourceState};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{
    Bounds, ColWidth, Column, Constraint, Dir, Line, MarkerHint, Series, Span, View,
};

use super::{Disk, Drive, SeriesKind, TIER_CHART, TIER_RATES, TIER_SPARKS, TIER_TABLE};

/// Bytes per second, in the units a person reads.
pub fn rate(bps: f64) -> String {
    const UNITS: [&str; 5] = ["B", "k", "M", "G", "T"];
    let mut v = bps.max(0.0);
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", UNITS[i])
    } else if v < 10.0 {
        format!("{v:.1}{}", UNITS[i])
    } else {
        format!("{v:.0}{}", UNITS[i])
    }
}

/// A capacity, in the units a drive is sold in.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// One table column. The set is a **pure function of the width** so the drop
/// order can be tested directly rather than by reading pixels back out of a
/// rendered buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Device,
    Read,
    Write,
    Busy,
    Temp,
    ReadsPs,
    WritesPs,
    Queue,
    Model,
}

impl Col {
    pub fn title(self) -> &'static str {
        match self {
            Col::Device => "DEVICE",
            Col::Read => "READ",
            Col::Write => "WRITE",
            Col::Busy => "BUSY",
            Col::Temp => "°C",
            Col::ReadsPs => "R/S",
            Col::WritesPs => "W/S",
            Col::Queue => "Q",
            Col::Model => "MODEL",
        }
    }

    fn spec(self) -> Column {
        let (w, right) = match self {
            // 11, not 9: the cell is a status dot plus a space before the
            // name, and a partition name is two characters longer than its
            // drive's (`nvme0n1p1` against `nvme0n1`). At 9 every partition
            // clipped back to exactly its parent's name and the MODEL column
            // repeated the parent's model, so `partitions = true` drew twelve
            // rows with three distinct names (arc 14 review).
            Col::Device => (ColWidth::Fixed(11), false),
            Col::Read | Col::Write => (ColWidth::Fixed(7), true),
            Col::Busy => (ColWidth::Fixed(5), true),
            Col::Temp => (ColWidth::Fixed(4), true),
            Col::ReadsPs | Col::WritesPs | Col::Queue => (ColWidth::Fixed(6), true),
            Col::Model => (ColWidth::Elastic, false),
        };
        Column {
            title: Cow::Borrowed(self.title()),
            width: w,
            right,
        }
    }
}

/// The widths at which each column joins. Read the other way — the width
/// running out — this is §8's drop order: `Q`, `W/S`, `R/S`, `°C`, `BUSY`,
/// with the elastic `MODEL` the first thing to go. `DEVICE READ WRITE`
/// always survive, so the guaranteed set at the tier's own 36-wide minimum
/// is `DEVICE READ WRITE BUSY`.
const ADDED_AT: [(u16, Col); 5] = [
    (41, Col::Temp),
    (48, Col::ReadsPs),
    (55, Col::WritesPs),
    (61, Col::Queue),
    (72, Col::Model),
];

pub fn columns_for(width: u16) -> Vec<Col> {
    let mut out = vec![Col::Device, Col::Read, Col::Write, Col::Busy];
    // `MODEL` is last in the row but joins last too, so a plain filter over
    // the thresholds gives both the order and the set.
    for (at, col) in ADDED_AT {
        if width >= at {
            out.push(col);
        }
    }
    out
}

/// The role a busy percentage is drawn in. Deliberately conservative: on an
/// NVMe queue this number saturates long before the drive does, so it never
/// reaches `Crit` on its own (D64 §6).
fn busy_role(pct: f64) -> Role {
    if pct >= 90.0 {
        Role::Warn
    } else if pct >= 40.0 {
        Role::AccentPrimary
    } else {
        Role::Text
    }
}

fn temp_role(d: &Drive) -> Role {
    match (d.temp_c, d.crit_c) {
        (Some(t), Some(c)) if t >= c => Role::Crit,
        (Some(t), Some(c)) if t >= c - 10.0 => Role::Warn,
        (Some(_), _) => Role::Text,
        (None, _) => Role::TextMuted,
    }
}

/// The dot beside a drive: how hard it is working, at a glance.
fn dot(d: &Drive) -> Span {
    let (role, glyph) = if d.busy_pct >= 90.0 {
        (Role::Warn, "●")
    } else if d.total() > 0.0 || d.busy_pct > 0.0 {
        (Role::Ok, "●")
    } else {
        (Role::TextMuted, "○")
    };
    Span::new(role, glyph)
}

fn status_line(cx: &RenderCx<'_>) -> Option<Line> {
    let st = cx.store.status(disk::SOURCE);
    match st.state {
        SourceState::Unavailable | SourceState::Degraded => {
            let mut text = st.reason.as_deref().unwrap_or("unavailable").to_string();
            if let Some(h) = st.hint.as_deref() {
                text.push_str(" — ");
                text.push_str(h);
            }
            Some(vec![Span::new(Role::Warn, text)])
        }
        _ => None,
    }
}

fn empty(cx: &RenderCx<'_>) -> View {
    let line =
        status_line(cx).unwrap_or_else(|| vec![Span::new(Role::TextMuted, "— no block devices")]);
    View::Text(vec![line])
}

pub fn render(d: &Disk, cx: &RenderCx<'_>) -> View {
    if d.model().drives.is_empty() {
        return empty(cx);
    }
    match cx.tier {
        TIER_RATES => rates(d, cx),
        TIER_SPARKS => sparks(d, cx),
        TIER_TABLE => table_tier(d, cx),
        TIER_CHART => chart_tier(d, cx),
        _ => full(d, cx),
    }
}

/// `rd 1.5G` / `wr 340M` summed over the drives the tile shows, with the
/// busiest drive's `BUSY` as a chip. There is no published total (D64 §2):
/// this is the sum of what is on screen, and the footer says over how many
/// drives wherever there is room for it.
fn rates(d: &Disk, cx: &RenderCx<'_>) -> View {
    let (rd, wr) = d.model().totals();
    let busiest = d.model().busiest();
    let w = usize::from(cx.inner.width);
    let mut head: Line = Vec::new();
    if let Some(b) = busiest {
        head.push(dot(b));
        head.push(Span::new(
            busy_role(b.busy_pct),
            format!(" {:.0}%", b.busy_pct),
        ));
        if w >= 14 {
            head.push(Span::new(Role::TextMuted, format!(" {}", b.name)));
        }
    }
    let read: Line = vec![
        Span::new(Role::AccentPrimary, "rd "),
        Span::bold(Role::Text, rate(rd)),
    ];
    let write: Line = vec![
        Span::new(Role::AccentSecondary, "wr "),
        Span::bold(Role::Text, rate(wr)),
    ];
    let mut children = vec![(Constraint::Len(1), View::Text(vec![head]))];
    if cx.inner.height >= 3 {
        children.push((Constraint::Len(1), View::Text(vec![read])));
        children.push((Constraint::Len(1), View::Text(vec![write])));
    } else {
        let mut one = read;
        one.push(Span::new(Role::TextMuted, "  "));
        one.extend(write);
        children.push((Constraint::Len(1), View::Text(vec![one])));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The window a rate sparkline covers, at most. The span actually asked for
/// is the run's age capped at this, never a fixed span — with a fixed one a
/// run younger than the window has no samples for its oldest buckets and the
/// line starts part-way across the rect (D62 amendment 1).
const SPARK_SPAN: Duration = Duration::from_secs(120);
/// The same, for the braille chart.
const CHART_SPAN: Duration = Duration::from_secs(300);

fn window(cx: &RenderCx<'_>, cap: Duration) -> Duration {
    Duration::from_nanos(cx.now.0)
        .min(cap)
        .max(Duration::from_secs(1))
}

fn spark(
    cx: &RenderCx<'_>,
    key: &gridwatch_store::Key<f64>,
    dev: &str,
    gradient: GradientId,
) -> View {
    let mut buf = Vec::new();
    let buckets = usize::from(cx.inner.width).max(2);
    cx.store.resample(
        &key.named(&std::sync::Arc::from(dev)),
        window(cx, SPARK_SPAN),
        buckets,
        Agg::Max,
        &mut buf,
    );
    View::Sparkline {
        series: buf.iter().map(|v| v.map(|v| v as f32)).collect(),
        gradient,
        max: None,
    }
}

fn sparks(d: &Disk, cx: &RenderCx<'_>) -> View {
    let Some(b) = d.model().busiest() else {
        return empty(cx);
    };
    let mut head: Line = vec![
        dot(b),
        Span::new(Role::TextMuted, format!(" {} ", b.name)),
        Span::new(Role::AccentPrimary, "rd "),
        Span::bold(Role::Text, rate(b.read_bps)),
        Span::new(Role::AccentSecondary, "  wr "),
        Span::bold(Role::Text, rate(b.write_bps)),
    ];
    if cx.inner.width >= 34 {
        head.push(Span::new(
            busy_role(b.busy_pct),
            format!("  {:.0}%", b.busy_pct),
        ));
        head.push(Span::new(
            Role::TextMuted,
            format!(" busy · q {:.1}", b.queue),
        ));
    }
    if cx.inner.width >= 48
        && let Some(t) = b.temp_c
    {
        head.push(Span::new(temp_role(b), format!("  {t:.0}°")));
    }
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![head])),
            (
                Constraint::Fill(1),
                spark(cx, &disk::READ_BPS, &b.name, GradientId::NetRx),
            ),
            (
                Constraint::Fill(1),
                spark(cx, &disk::WRITE_BPS, &b.name, GradientId::NetTx),
            ),
        ],
    }
}

/// One row per drive, in the column set the width allows.
fn rows(d: &Disk, cols: &[Col]) -> Vec<Vec<Line>> {
    d.model()
        .drives
        .iter()
        .map(|dr| {
            cols.iter()
                .map(|c| match c {
                    Col::Device => vec![dot(dr), Span::new(Role::Text, format!(" {}", dr.name))],
                    Col::Read => vec![Span::bold(Role::Text, rate(dr.read_bps))],
                    Col::Write => vec![Span::bold(Role::Text, rate(dr.write_bps))],
                    Col::Busy => vec![Span::new(
                        busy_role(dr.busy_pct),
                        format!("{:.0}%", dr.busy_pct),
                    )],
                    Col::Temp => vec![Span::new(
                        temp_role(dr),
                        match dr.temp_c {
                            Some(t) => format!("{t:.0}°"),
                            None => "—".to_string(),
                        },
                    )],
                    Col::ReadsPs => vec![Span::new(Role::TextMuted, format!("{:.0}", dr.reads_ps))],
                    Col::WritesPs => {
                        vec![Span::new(Role::TextMuted, format!("{:.0}", dr.writes_ps))]
                    }
                    Col::Queue => vec![Span::new(Role::Text, format!("{:.1}", dr.queue))],
                    Col::Model => vec![Span::new(
                        Role::TextMuted,
                        dr.info
                            .as_ref()
                            .map(|i| i.model.clone())
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| "—".to_string()),
                    )],
                })
                .collect()
        })
        .collect()
}

fn drive_table(d: &Disk, cx: &RenderCx<'_>, body: usize) -> View {
    let cols = columns_for(cx.inner.width);
    let rows = rows(d, &cols);
    let body = body.max(1);
    let cursor = d.scroll().min(rows.len().saturating_sub(1));
    let top = cursor
        .saturating_sub(body.saturating_sub(1))
        .min(rows.len().saturating_sub(body.min(rows.len())));
    View::Table {
        columns: cols.iter().map(|c| c.spec()).collect(),
        selected: (cx.captured && rows.len() > body).then_some(cursor),
        rows,
        sort: None,
        scroll: top,
    }
}

/// The footer: what the tile is showing, and the keys that change it.
fn footer(d: &Disk, below: usize) -> Line {
    let n = d.model().drives.len();
    let mut line: Line = vec![Span::new(
        Role::TextMuted,
        format!(
            "{n} device{} · sort {} · s sort",
            if n == 1 { "" } else { "s" },
            d.sort().name()
        ),
    )];
    let hidden = d.model().hidden();
    if hidden > 0 {
        line.push(Span::new(Role::TextGhost, format!("  ({hidden} hidden)")));
    }
    if below > 0 {
        line.push(Span::new(
            Role::TextGhost,
            format!(
                "  ({below} more pane{} below — ↑/↓)",
                if below == 1 { "" } else { "s" }
            ),
        ));
    }
    line
}

fn table_tier(d: &Disk, cx: &RenderCx<'_>) -> View {
    let body = usize::from(cx.inner.height).saturating_sub(2);
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Fill(1), drive_table(d, cx, body)),
            (Constraint::Len(1), View::Text(vec![footer(d, 0)])),
        ],
    }
}

/// The service-time line: the last read and write awaits per drive, while
/// they are fresh, and `—` when the drive has completed nothing recently.
/// The hold is three times the source's **observed** cadence (§11), not a
/// comparison against the newest sample — a drive doing one or two I/Os a
/// second completes nothing on most ticks and would otherwise strobe.
fn await_line(d: &Disk, cx: &RenderCx<'_>) -> Line {
    let hold = d.await_hold();
    let mut line: Line = vec![Span::new(Role::TextMuted, "await ")];
    for (i, dr) in d.model().drives.iter().enumerate() {
        if i > 0 {
            line.push(Span::new(Role::TextGhost, " · "));
        }
        let one = |v: Option<f64>| match v {
            Some(v) => format!("{v:.2} ms"),
            None => "—".to_string(),
        };
        line.push(Span::new(Role::Text, format!("{} ", dr.name)));
        line.push(Span::new(
            Role::AccentPrimary,
            format!("r {}", one(dr.await_ms(SeriesKind::Read, cx.now, hold))),
        ));
        line.push(Span::new(
            Role::AccentSecondary,
            format!("  w {}", one(dr.await_ms(SeriesKind::Write, cx.now, hold))),
        ));
    }
    line
}

/// The legend: which series the chart is drawing, over what window, and the
/// keys that change it.
fn legend(d: &Disk, cx: &RenderCx<'_>) -> Line {
    let secs = window(cx, CHART_SPAN).as_secs();
    let span = if secs < 90 {
        format!("{secs} s")
    } else {
        format!("{} min", (secs + 30) / 60)
    };
    let mut line: Line = vec![
        Span::new(Role::TextMuted, "series "),
        Span::bold(Role::Text, d.series().name()),
        Span::new(Role::TextMuted, format!(" over {span}  ")),
    ];
    for (i, s) in SeriesKind::ALL.iter().enumerate() {
        line.push(Span::new(
            if *s == d.series() {
                Role::AccentPrimary
            } else {
                Role::TextGhost
            },
            format!("{} {} ", i + 1, s.name()),
        ));
    }
    line
}

fn chart_view(d: &Disk, cx: &RenderCx<'_>) -> View {
    let kind = d.series();
    let key = kind.key();
    let span = window(cx, CHART_SPAN);
    let buckets = usize::from(cx.inner.width).max(2) * 2;
    let gradient = match kind {
        SeriesKind::Read => GradientId::NetRx,
        SeriesKind::Write => GradientId::NetTx,
        SeriesKind::Busy => GradientId::Load,
        SeriesKind::Queue => GradientId::Mem,
    };
    let mut series = Vec::with_capacity(d.model().drives.len());
    let mut hi = f64::MIN;
    let mut buf: Vec<Option<f64>> = Vec::new();
    for dr in &d.model().drives {
        cx.store.resample(
            &key.named(&std::sync::Arc::from(dr.name.as_str())),
            span,
            buckets,
            Agg::Avg,
            &mut buf,
        );
        let data: Vec<(f64, f64)> = buf
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (i as f64, v)))
            .collect();
        for (_, v) in &data {
            hi = hi.max(*v);
        }
        series.push(Series {
            label: Cow::Owned(dr.name.clone()),
            gradient,
            data,
        });
    }
    // The floor is zero: a rate chart that hid the bottom of its range would
    // make an idle drive look busy.
    let hi = if hi == f64::MIN || hi <= 0.0 {
        match kind {
            SeriesKind::Busy => 100.0,
            _ => 1.0,
        }
    } else {
        hi * 1.1
    };
    View::Chart {
        series,
        bounds: Bounds {
            x: (0.0, (buckets.saturating_sub(1)).max(1) as f64),
            y: (0.0, hi),
        },
        marker: MarkerHint::Braille,
    }
}

fn chart_tier(d: &Disk, cx: &RenderCx<'_>) -> View {
    // The table takes the rows it has (a header plus a drive each), capped
    // at a third of the body; everything left over is the chart's, rather
    // than blank space between the two.
    let body = usize::from(cx.inner.height);
    let rows = (d.model().drives.len() + 1).min((body / 3).max(2));
    View::Stack {
        dir: Dir::V,
        children: vec![
            (
                Constraint::Len(u16::try_from(rows).unwrap_or(u16::MAX)),
                drive_table(d, cx, rows.saturating_sub(1)),
            ),
            (Constraint::Len(1), View::Text(vec![legend(d, cx)])),
            (Constraint::Fill(1), chart_view(d, cx)),
            (Constraint::Len(1), View::Text(vec![await_line(d, cx)])),
            (Constraint::Len(1), View::Text(vec![footer(d, 0)])),
        ],
    }
}

/// What `busy` cannot tell you, on the tile rather than only in a decision
/// file (D64 §6).
const CAVEAT: &str = "BUSY is the share of time the queue was non-empty — not how full it was. \
     One I/O outstanding on a 1023-deep NVMe queue reads 100 %. Q is the mean number in flight.";

/// The per-drive pane: what the drive is, where its temperature comes from,
/// and this interval's numbers written out.
fn pane(d: &Disk, dr: &Drive, cx: &RenderCx<'_>) -> Vec<Line> {
    let hold = d.await_hold();
    let ms = |v: Option<f64>| match v {
        Some(v) => format!("{v:.2} ms"),
        None => "—".to_string(),
    };
    let info = dr.info.as_ref();
    let mut head: Line = vec![dot(dr), Span::bold(Role::Text, format!(" {}", dr.name))];
    if let Some(i) = info {
        head.push(Span::new(
            Role::TextMuted,
            format!(
                "  {} · {} · {}",
                i.kind.name(),
                size(i.size_b),
                if i.rotational { "rotational" } else { "ssd" }
            ),
        ));
        if i.removable {
            head.push(Span::new(Role::Info, " · removable"));
        }
    }
    let mut out = vec![head];
    out.push(vec![
        Span::new(Role::TextMuted, "  model      "),
        Span::new(
            Role::Text,
            info.map(|i| i.model.clone())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "—".to_string()),
        ),
    ]);
    // The controller, and the hwmon node the temperature was joined from —
    // by device, never by index (D64 §7). Two namespaces of one controller
    // legitimately share a reading, so the line says whose it is.
    let mut ctrl: Line = vec![
        Span::new(Role::TextMuted, "  controller "),
        Span::new(
            Role::Text,
            info.map(|i| i.device.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".to_string()),
        ),
    ];
    match (dr.temp_c, dr.chip.as_ref()) {
        (Some(t), Some(chip)) => {
            ctrl.push(Span::new(Role::TextMuted, format!(" · hwmon {chip} ")));
            ctrl.push(Span::new(temp_role(dr), format!("{t:.1} °C")));
            if let Some(c) = dr.crit_c {
                ctrl.push(Span::new(Role::TextMuted, format!(" (crit {c:.1})")));
            }
            ctrl.push(Span::new(Role::TextGhost, " — the controller's"));
        }
        _ => {
            ctrl.push(Span::new(Role::TextMuted, " · temperature — "));
            ctrl.push(Span::new(Role::TextGhost, dr.no_temp.why()));
        }
    }
    out.push(ctrl);
    out.push(vec![
        Span::new(Role::TextMuted, "  queue      "),
        Span::new(busy_role(dr.busy_pct), format!("busy {:.0}%", dr.busy_pct)),
        Span::new(
            Role::Text,
            format!(
                " · q {:.1} of {}",
                dr.queue,
                info.map(|i| i.nr_requests).unwrap_or(0)
            ),
        ),
        Span::new(
            Role::TextMuted,
            format!(
                " · r {} · w {}",
                ms(dr.await_ms(SeriesKind::Read, cx.now, hold)),
                ms(dr.await_ms(SeriesKind::Write, cx.now, hold))
            ),
        ),
    ]);
    out.push(vec![
        Span::new(Role::TextMuted, "  scheduler  "),
        Span::new(
            Role::Text,
            info.map(|i| i.scheduler.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".to_string()),
        ),
        Span::new(
            Role::TextMuted,
            format!(
                " · r/s {:.0} · w/s {:.0} · discard {}/s",
                dr.reads_ps,
                dr.writes_ps,
                dr.discard_bps.map(rate).unwrap_or_else(|| "—".into())
            ),
        ),
    ]);
    if let Some(i) = info
        && !i.partitions.is_empty()
    {
        out.push(vec![
            Span::new(Role::TextMuted, "  partitions "),
            Span::new(Role::Text, i.partitions.join(" ")),
        ]);
    }
    out
}

fn full(d: &Disk, cx: &RenderCx<'_>) -> View {
    let head = u16::try_from(d.model().drives.len() + 1).unwrap_or(u16::MAX);
    // Everything above the panes takes a known number of rows, so the panes
    // get exactly what is left and a 100x24 tile draws whole drives rather
    // than half of one. `↑/↓` moves the window, so a tile too short for
    // every drive can still reach them all.
    let fixed = head.saturating_add(5);
    let room = usize::from(cx.inner.height.saturating_sub(fixed));
    let first = d.scroll().min(d.model().drives.len().saturating_sub(1));
    let mut panes: Vec<Line> = Vec::new();
    let mut drawn = 0usize;
    for dr in d.model().drives.iter().skip(first) {
        let p = pane(d, dr, cx);
        if !panes.is_empty() && panes.len() + p.len() > room {
            break;
        }
        panes.extend(p);
        panes.push(Vec::new());
        drawn += 1;
    }
    let below = d.model().drives.len().saturating_sub(first + drawn);
    let panes_h = u16::try_from(panes.len()).unwrap_or(u16::MAX);
    View::Stack {
        dir: Dir::V,
        children: vec![
            (
                Constraint::Len(head.min(cx.inner.height)),
                drive_table(d, cx, d.model().drives.len()),
            ),
            (Constraint::Len(1), View::Text(vec![legend(d, cx)])),
            (Constraint::Len(1), View::Text(vec![await_line(d, cx)])),
            (Constraint::Len(panes_h), View::Text(panes)),
            (Constraint::Fill(1), chart_view(d, cx)),
            (
                Constraint::Len(1),
                View::Text(vec![vec![Span::new(Role::TextGhost, CAVEAT)]]),
            ),
            (Constraint::Len(1), View::Text(vec![footer(d, below)])),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §8's drop order, as a table rather than as pixels. The six widths are
    /// the brief's own.
    #[test]
    fn the_column_set_is_a_function_of_the_width() {
        let names =
            |w: u16| -> Vec<&'static str> { columns_for(w).into_iter().map(Col::title).collect() };
        assert_eq!(names(36), ["DEVICE", "READ", "WRITE", "BUSY"]);
        assert_eq!(names(41), ["DEVICE", "READ", "WRITE", "BUSY", "°C"]);
        assert_eq!(names(48), ["DEVICE", "READ", "WRITE", "BUSY", "°C", "R/S"]);
        assert_eq!(
            names(55),
            ["DEVICE", "READ", "WRITE", "BUSY", "°C", "R/S", "W/S"]
        );
        assert_eq!(
            names(61),
            ["DEVICE", "READ", "WRITE", "BUSY", "°C", "R/S", "W/S", "Q"]
        );
        assert_eq!(
            names(80),
            [
                "DEVICE", "READ", "WRITE", "BUSY", "°C", "R/S", "W/S", "Q", "MODEL"
            ]
        );
        // The four that always survive, at any width the tier can be drawn
        // at — and the order the set is dropped in, read backwards.
        for w in 0..=200u16 {
            let c = columns_for(w);
            assert_eq!(&c[..4], [Col::Device, Col::Read, Col::Write, Col::Busy]);
            assert!(c.windows(2).all(|p| p[0] != p[1]));
        }
        // Every column set fits its own threshold: fixed widths plus one
        // separator each, so a header is never truncated.
        for (at, _) in ADDED_AT {
            let need: u16 = columns_for(at)
                .iter()
                .map(|c| match c.spec().width {
                    ColWidth::Fixed(w) => w,
                    ColWidth::Elastic => 0,
                })
                .sum::<u16>()
                + columns_for(at).len() as u16
                - 1;
            assert!(need <= at, "the set at {at} needs {need} columns");
        }
    }

    #[test]
    fn rates_and_sizes_read_the_way_a_person_writes_them() {
        assert_eq!(rate(0.0), "0B");
        assert_eq!(rate(900.0), "900B");
        assert_eq!(rate(1_500_000.0), "1.5M");
        assert_eq!(rate(1_500_000_000.0), "1.5G");
        assert_eq!(rate(12_000_000.0), "12M");
        assert_eq!(size(7_814_037_168 * 512), "4.0 TB");
        assert_eq!(size(0), "0 B");
    }

    /// The band a busy percentage is drawn in never reaches `Crit`: on an
    /// NVMe queue this number saturates long before the drive does.
    #[test]
    fn busy_is_never_drawn_as_critical() {
        for pct in [0.0, 39.9, 40.0, 89.9, 90.0, 100.0] {
            assert_ne!(busy_role(pct), Role::Crit, "{pct}");
        }
        assert_eq!(busy_role(0.0), Role::Text);
        assert_eq!(busy_role(50.0), Role::AccentPrimary);
        assert_eq!(busy_role(100.0), Role::Warn);
    }

    #[test]
    fn a_temperature_is_critical_only_against_the_chips_own_limit() {
        let d = |t: Option<f64>, c: Option<f64>| Drive {
            temp_c: t,
            crit_c: c,
            ..Drive::default()
        };
        assert_eq!(temp_role(&d(Some(85.0), Some(84.85))), Role::Crit);
        assert_eq!(temp_role(&d(Some(80.0), Some(84.85))), Role::Warn);
        assert_eq!(temp_role(&d(Some(48.0), Some(84.85))), Role::Text);
        assert_eq!(temp_role(&d(Some(48.0), None)), Role::Text);
        assert_eq!(temp_role(&d(None, None)), Role::TextMuted);
        assert_eq!(super::super::NoTemp::default(), super::super::NoTemp::None);
    }
}
