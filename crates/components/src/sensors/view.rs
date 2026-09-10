//! The sensors tiers as view trees (§8): `hottest` (one reading with its
//! chip and a Temp chip), `strip` (one reading per chip, hottest first),
//! `table` (CHIP · SENSOR · VALUE · MAX · BAR, scrollable), `chart` (the
//! table plus a braille chart of the four hottest over ten minutes) and the
//! zoom-only `full` (every reading of every chip, the RAPL line, the PSI row
//! and the gpu row). Every string is a theme role.

use std::borrow::Cow;

use gridwatch_store::keys::sensors::RaplState;
use gridwatch_store::keys::{cpu, gpu, sensors};
use gridwatch_store::{Agg, SourceState};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{
    Bounds, ColWidth, Column, Constraint, Dir, Line, MarkerHint, Series, Span, View,
};

use super::{Reading, Sensors, TIER_CHART, TIER_HOTTEST, TIER_STRIP, TIER_TABLE};

pub fn render(s: &Sensors, cx: &RenderCx<'_>) -> View {
    match cx.tier {
        TIER_HOTTEST => hottest(s, cx),
        TIER_STRIP => strip(s, cx),
        TIER_TABLE => table_tier(s, cx, None, cx.inner.height),
        TIER_CHART => chart_tier(s, cx),
        _ => full(s, cx),
    }
}

/// The role a reading's number is drawn in.
fn role_of(r: &Reading) -> Role {
    if r.over_crit() {
        Role::Crit
    } else if r.over_max() || r.max.is_some_and(|m| r.value >= 0.9 * m) {
        Role::Warn
    } else {
        Role::Text
    }
}

fn degrees(v: f64) -> String {
    format!("{v:.0}°")
}

/// The status line an unavailable source earns; `None` while it is fine.
fn status_line(cx: &RenderCx<'_>) -> Option<Line> {
    let st = cx.store.status(sensors::SOURCE);
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
    let line = status_line(cx).unwrap_or_else(|| vec![Span::new(Role::TextMuted, "— no sensors")]);
    View::Text(vec![line])
}

/// `k10temp Tctl 59°` with a Temp gauge under it, `▲` when over max.
fn hottest(s: &Sensors, cx: &RenderCx<'_>) -> View {
    let Some(r) = s.model().hottest() else {
        return empty(cx);
    };
    // The reading is what the tier promises: it is placed first and the
    // chip and label take whatever room is left (review: at 17 cells the
    // number itself was being truncated away).
    let w = usize::from(cx.inner.width);
    let value = degrees(r.value);
    let mark = if r.over_max() { 2 } else { 0 };
    let spare = w.saturating_sub(value.chars().count() + mark + 1);
    let mut head: Line = Vec::new();
    if spare >= 4 {
        let label_room = spare.saturating_sub(r.chip.chars().count() + 1);
        let chip: String = r.chip.chars().take(spare.min(16)).collect();
        let mut prefix = chip;
        if label_room >= 4 {
            prefix.push(' ');
            prefix.extend(r.label.chars().take(label_room - 1));
        }
        prefix.push(' ');
        head.push(Span::new(Role::TextMuted, prefix));
    }
    head.push(Span::bold(role_of(r), value));
    if r.over_max() {
        head.push(Span::bold(Role::Crit, " ▲"));
    }
    // The gauge's text needs room beside its bar; below ~14 cells the bar
    // alone says it (the renderer draws nothing when the text crowds it).
    let gauge_text = (w >= 14).then(|| {
        Cow::Owned(if r.assumed() {
            format!("of ~{:.0}°", r.limit())
        } else {
            format!("max {:.0}°", r.limit())
        })
    });
    let gauge = View::Gauge {
        label: Cow::Borrowed(""),
        value: r.frac().min(1.0),
        gradient: GradientId::Temp,
        text: gauge_text,
    };
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![head])),
            (Constraint::Len(1), gauge),
        ],
    }
}

/// `nvme 51° · k10temp 59° · spd 44°` in one or two rows, hottest first.
fn strip(s: &Sensors, cx: &RenderCx<'_>) -> View {
    let per = s.model().per_chip();
    if per.is_empty() {
        return empty(cx);
    }
    let w = usize::from(cx.inner.width);
    let rows = usize::from(cx.inner.height).clamp(1, 2);
    let mut lines: Vec<Line> = vec![Vec::new()];
    let mut used = 0usize;
    for (i, r) in per.iter().take(6).enumerate() {
        let short: String = r.chip.chars().take(10).collect();
        let text = format!("{short} {}", degrees(r.value));
        let sep = if i == 0 { 0 } else { 3 };
        let need = text.chars().count() + sep + usize::from(r.over_max());
        if used + need > w {
            if lines.len() >= rows {
                break;
            }
            lines.push(Vec::new());
            used = 0;
        } else if i > 0 {
            lines
                .last_mut()
                .unwrap()
                .push(Span::new(Role::TextMuted, " · "));
            used += 3;
        }
        let line = lines.last_mut().unwrap();
        line.push(Span::new(Role::TextMuted, format!("{short} ")));
        line.push(Span::bold(role_of(r), degrees(r.value)));
        if r.over_max() {
            line.push(Span::bold(Role::Crit, "▲"));
        }
        used += text.chars().count() + usize::from(r.over_max());
    }
    let mut children: Vec<(Constraint, View)> =
        vec![(Constraint::Len(rows as u16), View::Text(lines))];
    if usize::from(cx.inner.height) > rows + 1
        && let Some(h) = s.model().hottest()
    {
        children.push((
            Constraint::Len(1),
            View::Gauge {
                label: Cow::Owned(format!("{} {}", h.chip, h.label)),
                value: h.frac().min(1.0),
                gradient: GradientId::Temp,
                text: Some(Cow::Owned(if h.assumed() {
                    format!("{} of ~{}", degrees(h.value), degrees(h.limit()))
                } else {
                    format!("{} / max {}", degrees(h.value), degrees(h.limit()))
                })),
            },
        ));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The table's rows. The brief's BAR column is a percentage of the chip's
/// own max here: a `View::Table` cell is a `Line`, and a component may not
/// write glyphs (§4.6) — the bar lives in `hottest`/`strip` as a `Gauge`
/// the renderer draws, and the table prints `72 %` in the reading's role.
fn table_rows(temps: &[Reading], with_bar: bool) -> Vec<Vec<Line>> {
    temps
        .iter()
        .map(|r| {
            let mut row = vec![
                vec![Span::new(Role::Text, r.chip.clone())],
                vec![Span::new(Role::TextMuted, r.label.clone())],
                vec![Span::bold(role_of(r), format!("{:.1}°C", r.value))],
                vec![Span::new(
                    Role::TextMuted,
                    if r.assumed() {
                        format!("~{:.0}°", r.limit())
                    } else {
                        format!("{:.0}°", r.limit())
                    },
                )],
            ];
            if with_bar {
                // The column the rows are ordered by, always shown — an
                // invisible sort key is a puzzle (review).
                row.push(vec![Span::new(
                    role_of(r),
                    format!("{:>3.0} %", r.heat() * 100.0),
                )]);
            }
            row
        })
        .collect()
}

/// The cells this table draws into: its fixed widths as declared, its elastic
/// column at the widest cell it holds, and one separator between each. The
/// elastic half comes from the renderer's own measurement so the number the
/// tier reserves and the number the renderer draws cannot drift (D65 §3).
fn table_natural_width(columns: &[Column], rows: &[Vec<Line>]) -> u16 {
    let natural = gridwatch_ui::renderer::table_natural_widths(columns, rows);
    let sum: u16 = columns
        .iter()
        .zip(&natural)
        .map(|(c, n)| match c.width {
            ColWidth::Fixed(w) => w,
            ColWidth::Elastic => *n,
        })
        .sum();
    sum + u16::try_from(columns.len().saturating_sub(1)).unwrap_or(0)
}

/// The cells the bar column needs beyond the table's own width before it is
/// worth drawing: a bar short enough to read as a chip is worse than none.
/// Arc 14's `MODEL` rule — a column appears when there is room for it.
const GAUGES_AT: u16 = 16;

/// The table's columns for a width. Split out from `table_view` so the tier
/// can ask what the table is *worth* — its natural width — before deciding
/// whether there is room beside it for the bars (§4.6, D65 §1: text gives the
/// rect its size).
fn table_columns(width: u16) -> Vec<Column> {
    let with_bar = width >= 48;
    // The chip is a name, not a paragraph: the elastic column is the
    // sensor label, so the numbers sit beside the chips.
    let mut columns = vec![
        Column {
            title: "chip".into(),
            width: ColWidth::Fixed(16),
            right: false,
        },
        Column {
            title: "sensor".into(),
            width: ColWidth::Elastic,
            right: false,
        },
        Column {
            title: "value".into(),
            width: ColWidth::Fixed(7),
            right: true,
        },
        Column {
            title: "limit".into(),
            width: ColWidth::Fixed(5),
            right: true,
        },
    ];
    if with_bar {
        columns.push(Column {
            title: "of lim".into(),
            width: ColWidth::Fixed(6),
            right: true,
        });
    }
    columns
}

/// The table's top row for a cursor and a body height. `scroll` is derived
/// from the cursor so the selection stays visible and the last page is never
/// scrolled past (review: 20 downs left one row on screen).
fn top_row(cursor: usize, len: usize, body: usize) -> usize {
    cursor
        .saturating_sub(body.saturating_sub(1))
        .min(len.saturating_sub(body.min(len)))
}

fn table_view(s: &Sensors, cx: &RenderCx<'_>, rows: Vec<Vec<Line>>, body: usize) -> View {
    let columns = table_columns(cx.inner.width);
    let cursor = s.scroll().min(rows.len().saturating_sub(1));
    let top = top_row(cursor, rows.len(), body);
    View::Table {
        columns,
        rows,
        selected: cx.captured.then_some(cursor),
        sort: None,
        scroll: top,
    }
}

/// One `Len(1)` bar per *shown* row under a `View::Empty` header spacer, so
/// the bars sit on the rows they belong to — the warn/crit bars §8 has
/// promised since arc 5b (D65 §7). The value is the same `heat` the table
/// sorts by and the `of lim` column prints; the bar is that number's picture,
/// not its replacement, and the percentage at the far end of a bar that can be
/// two hundred cells long is what says where the end is.
///
/// The per-row **mini sparklines** §8 also names are not here and cannot be: a
/// table cell is a `Vec<Span>` and a component may not write a glyph (§4.6),
/// so they need a third pane. `BACKLOG.md`.
fn gauge_pane(temps: &[Reading], top: usize, body: usize) -> View {
    let mut children: Vec<(Constraint, View)> = vec![(Constraint::Len(1), View::Empty)];
    for r in temps.iter().skip(top).take(body) {
        let heat = r.heat();
        children.push((
            Constraint::Len(1),
            View::Gauge {
                label: Cow::Borrowed(""),
                value: (heat as f32).clamp(0.0, 1.0),
                gradient: GradientId::Temp,
                text: Some(Cow::Owned(format!("{:>3.0} %", heat * 100.0))),
            },
        ));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The table, and beside it the bars when the width beyond the table's own
/// content allows. `rows_available` is the band the caller is giving this
/// table — not the tile's inner height, which is what the viewport used to be
/// computed from at the `chart` tier, where the table only gets three fifths
/// of it.
fn table_tier(s: &Sensors, cx: &RenderCx<'_>, footer: Option<Line>, rows_available: u16) -> View {
    let m = s.model();
    if m.temps.is_empty() {
        return empty(cx);
    }
    let rapl_hint = m
        .info
        .as_ref()
        .is_some_and(|i| i.rapl == RaplState::RootOnly);
    let footer_rows = u16::from(footer.is_some() || (rapl_hint && cx.inner.height >= 10));
    let body = usize::from(rows_available.saturating_sub(1 + footer_rows)).max(1);
    let rows = table_rows(&m.temps, cx.inner.width >= 48);
    let rows_len = rows.len();
    let natural = table_natural_width(&table_columns(cx.inner.width), &rows);
    let cursor = s.scroll().min(rows.len().saturating_sub(1));
    let top = top_row(cursor, rows.len(), body);
    let table = table_view(s, cx, rows, body);
    let with_gauges = cx.inner.width >= natural.saturating_add(GAUGES_AT);
    let pane = if with_gauges {
        View::Stack {
            dir: Dir::H,
            children: vec![
                // One cell more than the table draws into, so the bars do not
                // start against the `of lim` column they picture.
                (Constraint::Len(natural + 1), table),
                (Constraint::Fill(1), gauge_pane(&m.temps, top, body)),
            ],
        }
    } else {
        table
    };
    // The table is as many rows as the machine has readings and can never
    // use more, so it asks for them rather than for `Fill` — §4.6's text rule,
    // which arc 15 applied to net's interface band and left standing here, in
    // the tile it rebuilt: at 480×135 that was sixty-one blank rows, 45 % of
    // the terminal (arc 15 review, F3). Capped at two thirds of the body so a
    // short tile still leaves room for the footer.
    let table_h = u16::try_from(rows_len + 1)
        .unwrap_or(u16::MAX)
        .min(rows_available.saturating_sub(footer_rows).saturating_mul(2) / 3)
        .max(1);
    let mut children = vec![(Constraint::Len(table_h), pane)];
    if let Some(f) = footer {
        children.push((Constraint::Len(1), View::Text(vec![f])));
    } else if rapl_hint && cx.inner.height >= 10 {
        children.push((
            Constraint::Len(1),
            View::Text(vec![vec![Span::new(
                Role::TextMuted,
                "RAPL: needs a udev rule — see doctor",
            )]]),
        ));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The chart's window: the run's age, capped at five minutes (ten once the run
/// has passed ten). The cap alone is not enough — with a fixed five minutes a
/// one-minute-old run has no samples for four fifths of its buckets, and the
/// chart stops a fifth of the way across its rect. D62 §1's span rule is "the
/// run's age capped at the component's span", which is what htop, gpu and pins
/// already ask for.
fn chart_span(cx: &RenderCx<'_>) -> std::time::Duration {
    let cap = if cx.now.as_secs_f64() < 600.0 {
        300
    } else {
        600
    };
    std::time::Duration::from_nanos(cx.now.0)
        .min(std::time::Duration::from_secs(cap))
        .max(std::time::Duration::from_secs(1))
}

/// The window for the legend: seconds under a minute and a half, rounded
/// minutes after — a 40 s run says `40 s`, not `1 min` (arc 12 review).
fn chart_span_text(cx: &RenderCx<'_>) -> String {
    let s = chart_span(cx).as_secs();
    if s < 90 {
        format!("{s} s")
    } else {
        format!("{} min", (s + 30) / 60)
    }
}

/// The four hottest readings over the chart's window, as a braille chart.
fn chart_view(s: &Sensors, cx: &RenderCx<'_>) -> View {
    let m = s.model();
    let mut picks: Vec<&Reading> = m.temps.iter().collect();
    picks.sort_by(|a, b| super::hottest_first(a, b));
    picks.truncate(4);
    let span = chart_span(cx);
    let buckets = usize::from(cx.inner.width).max(2) * 2;
    let mut series = Vec::with_capacity(picks.len());
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    let mut buf: Vec<Option<f64>> = Vec::new();
    for r in &picks {
        let key = sensors::TEMP_C.named(&std::sync::Arc::from(r.key.as_str()));
        cx.store.resample(&key, span, buckets, Agg::Avg, &mut buf);
        let data: Vec<(f64, f64)> = buf
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (i as f64, v)))
            .collect();
        for (_, v) in &data {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        series.push(Series {
            label: Cow::Owned(r.key.clone()),
            gradient: GradientId::Temp,
            data,
        });
    }
    if lo == f64::MAX || hi == f64::MIN {
        lo = 20.0;
        hi = 100.0;
    }
    let (lo, hi) = (
        (lo - 5.0).floor().max(0.0),
        (hi + 5.0).ceil().max(lo + 10.0),
    );
    View::Chart {
        series,
        bounds: Bounds {
            x: (0.0, (buckets - 1) as f64),
            y: (lo, hi),
        },
        marker: MarkerHint::Auto,
    }
}

fn chart_legend(s: &Sensors, span: &str) -> Line {
    let m = s.model();
    let mut picks: Vec<&Reading> = m.temps.iter().collect();
    picks.sort_by(|a, b| super::hottest_first(a, b));
    let mut line: Line = vec![Span::new(Role::TextMuted, format!("chart · {span} · "))];
    for (i, r) in picks.iter().take(4).enumerate() {
        if i > 0 {
            line.push(Span::new(Role::TextMuted, " · "));
        }
        line.push(Span::new(Role::Text, r.key.clone()));
        line.push(Span::new(role_of(r), format!(" {}", degrees(r.value))));
    }
    line
}

fn chart_tier(s: &Sensors, cx: &RenderCx<'_>) -> View {
    if s.model().temps.is_empty() {
        return empty(cx);
    }
    let span = chart_span_text(cx);
    // The table's viewport is the band *this* tier gives it, three fifths of
    // the body, not the tile's inner height — the same rect the renderer will
    // hand it, taken from the renderer's own split so the two cannot drift
    // (§4.6, D65 §5: a scroll viewport is the band a table was given).
    let children = [Constraint::Fill(3), Constraint::Len(1), Constraint::Fill(2)];
    let bands = gridwatch_ui::layout::split(&children, cx.inner.height);
    View::Stack {
        dir: Dir::V,
        children: vec![
            (children[0], table_tier(s, cx, None, bands[0])),
            (children[1], View::Text(vec![chart_legend(s, &span)])),
            (children[2], chart_view(s, cx)),
        ],
    }
}

/// The RAPL line: `package 87 W` or the udev hint.
fn rapl_line(s: &Sensors) -> Line {
    let m = s.model();
    let pkg = m
        .others
        .iter()
        .find(|o| o.kind == "power" && o.key == "rapl:package-0");
    match (pkg, m.info.as_ref().map(|i| i.rapl)) {
        (Some(p), _) => vec![
            Span::new(Role::TextMuted, "RAPL package "),
            Span::bold(Role::Text, format!("{:.0} W", p.value)),
        ],
        (None, Some(RaplState::RootOnly)) => vec![Span::new(
            Role::Warn,
            "RAPL: needs a udev rule — see doctor",
        )],
        (None, Some(RaplState::Absent)) => {
            vec![Span::new(Role::TextMuted, "RAPL: absent on this machine")]
        }
        _ => vec![Span::new(Role::TextMuted, "RAPL: waiting for the source")],
    }
}

/// The PSI row from the cpu source's keys.
fn psi_line(cx: &RenderCx<'_>) -> Line {
    let v = |k: &gridwatch_store::Key<f64>| {
        cx.store
            .last(k)
            .map(|(_, v)| format!("{v:.2}"))
            .unwrap_or_else(|| "—".into())
    };
    vec![
        Span::new(Role::TextMuted, "PSI some avg10  cpu "),
        Span::new(Role::Text, v(&cpu::PSI_CPU)),
        Span::new(Role::TextMuted, " · mem "),
        Span::new(Role::Text, v(&cpu::PSI_MEM)),
        Span::new(Role::TextMuted, " · io "),
        Span::new(Role::Text, v(&cpu::PSI_IO)),
    ]
}

/// The gpu row from the gpu source's keys (`—` without the source).
fn gpu_line(cx: &RenderCx<'_>) -> Line {
    let last = |k: gridwatch_store::Key<f64>| cx.store.last(&k).map(|(_, v)| v);
    let temp = last(gpu::TEMP_C.idx(0));
    let fan = cx
        .store
        .last(&gpu::FAN_PCT.named(&gpu::fan_label(0, 0)))
        .map(|(_, v)| v);
    let power = last(gpu::POWER_W.idx(0));
    if temp.is_none() && fan.is_none() && power.is_none() {
        return vec![Span::new(Role::TextMuted, "gpu —  (no gpu source)")];
    }
    let f = |v: Option<f64>, unit: &str| {
        v.map(|v| format!("{v:.0}{unit}"))
            .unwrap_or_else(|| "—".into())
    };
    vec![
        Span::new(Role::TextMuted, "gpu "),
        Span::bold(Role::Text, f(temp, "°C")),
        Span::new(Role::TextMuted, " · fan "),
        Span::new(Role::Text, f(fan, "%")),
        Span::new(Role::TextMuted, " · "),
        Span::new(Role::Text, f(power, " W")),
        Span::new(Role::TextMuted, "  (from the gpu source)"),
    ]
}

fn full(s: &Sensors, cx: &RenderCx<'_>) -> View {
    let m = s.model();
    let mut others: Line = Vec::new();
    for (i, o) in m
        .others
        .iter()
        .filter(|o| o.key != "rapl:package-0")
        .enumerate()
    {
        if i > 0 {
            others.push(Span::new(Role::TextMuted, " · "));
        }
        let unit = match o.kind {
            "fan" => " rpm",
            "volt" => " V",
            _ => " W",
        };
        others.push(Span::new(Role::TextMuted, format!("{} ", o.key)));
        others.push(Span::new(Role::Text, format!("{:.1}{unit}", o.value)));
    }
    if others.is_empty() {
        others.push(Span::new(
            Role::TextMuted,
            "fans / volts / power: no hwmon chip here exports them",
        ));
    }
    // The band this pane is placed in, so the viewport is the rows the table
    // is actually given: with `body == rows.len()` the scroll can never leave
    // zero, and on a rect shorter than the reading list the cursor walks off
    // the bottom (§4.6, D65 §5).
    let rows = u16::try_from(m.temps.len().saturating_add(1)).unwrap_or(u16::MAX);
    let band = rows.min(cx.inner.height);
    let body = usize::from(band.saturating_sub(1)).max(1);
    let table = if m.temps.is_empty() {
        empty(cx)
    } else {
        table_view(s, cx, table_rows(&m.temps, true), body)
    };
    // `full` is cumulative over `table` and `chart` (§4.6: tiers are supersets),
    // so it carries their bars and their chart. It used to draw neither, which
    // made `z` on a wide terminal *poorer* than the tile it zoomed — twenty
    // drawn rows of a hundred and thirty-one, no bars and no chart, beside an
    // unzoomed tile with fifteen bars and a fifty-two-row chart (arc 15 review,
    // F1).
    let rows = table_rows(&m.temps, cx.inner.width >= 48);
    let natural = table_natural_width(&table_columns(cx.inner.width), &rows);
    let cursor = s.scroll().min(rows.len().saturating_sub(1));
    let top = top_row(cursor, rows.len(), body);
    let head = if !m.temps.is_empty() && cx.inner.width >= natural.saturating_add(GAUGES_AT) {
        View::Stack {
            dir: Dir::H,
            children: vec![
                (Constraint::Len(natural + 1), table),
                (Constraint::Fill(1), gauge_pane(&m.temps, top, body)),
            ],
        }
    } else {
        table
    };
    let mut children = vec![
        (Constraint::Len(band), head),
        (Constraint::Len(1), View::Text(vec![others])),
        (Constraint::Len(1), View::Text(vec![rapl_line(s)])),
        (Constraint::Len(1), View::Text(vec![psi_line(cx)])),
        (Constraint::Len(1), View::Text(vec![gpu_line(cx)])),
    ];
    // Whatever is left is the chart's, floored at the height that makes one
    // legible; below that the tile ends where its text ends.
    let used = band + 4;
    if !m.temps.is_empty() && cx.inner.height >= used.saturating_add(6) {
        children.push((
            Constraint::Len(1),
            View::Text(vec![chart_legend(s, &chart_span_text(cx))]),
        ));
        children.push((Constraint::Fill(1), chart_view(s, cx)));
    } else {
        children.push((Constraint::Fill(1), View::Empty));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}
