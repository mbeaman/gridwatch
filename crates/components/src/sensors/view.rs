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
        TIER_TABLE => table_tier(s, cx, None),
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

fn table_view(s: &Sensors, cx: &RenderCx<'_>, rows: Vec<Vec<Line>>) -> View {
    let with_bar = cx.inner.width >= 48;
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
    // `scroll` is the table's top row: derive it from the cursor so the
    // selection stays visible and the last page is never scrolled past
    // (review: 20 downs left one row on screen).
    let body = usize::from(cx.inner.height).saturating_sub(2).max(1);
    let cursor = s.scroll().min(rows.len().saturating_sub(1));
    let top = cursor
        .saturating_sub(body.saturating_sub(1))
        .min(rows.len().saturating_sub(body.min(rows.len())));
    View::Table {
        columns,
        rows,
        selected: cx.captured.then_some(cursor),
        sort: None,
        scroll: top,
    }
}

fn table_tier(s: &Sensors, cx: &RenderCx<'_>, footer: Option<Line>) -> View {
    let m = s.model();
    if m.temps.is_empty() {
        return empty(cx);
    }
    let rows = table_rows(&m.temps, cx.inner.width >= 48);
    let mut children = vec![(Constraint::Fill(1), table_view(s, cx, rows))];
    let rapl_hint = m
        .info
        .as_ref()
        .is_some_and(|i| i.rapl == RaplState::RootOnly);
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

/// The four hottest readings over ten minutes (five when the run is
/// younger), as a braille chart.
fn chart_view(s: &Sensors, cx: &RenderCx<'_>) -> View {
    let m = s.model();
    let mut picks: Vec<&Reading> = m.temps.iter().collect();
    picks.sort_by(|a, b| super::hottest_first(a, b));
    picks.truncate(4);
    let age = cx.now.as_secs_f64();
    let span = if age < 600.0 {
        std::time::Duration::from_secs(300)
    } else {
        std::time::Duration::from_secs(600)
    };
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

fn chart_legend(s: &Sensors, span_min: u64) -> Line {
    let m = s.model();
    let mut picks: Vec<&Reading> = m.temps.iter().collect();
    picks.sort_by(|a, b| super::hottest_first(a, b));
    let mut line: Line = vec![Span::new(
        Role::TextMuted,
        format!("chart · {span_min} min · "),
    )];
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
    let span_min = if cx.now.as_secs_f64() < 600.0 { 5 } else { 10 };
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Fill(3), table_tier(s, cx, None)),
            (
                Constraint::Len(1),
                View::Text(vec![chart_legend(s, span_min)]),
            ),
            (Constraint::Fill(2), chart_view(s, cx)),
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
    let table = if m.temps.is_empty() {
        empty(cx)
    } else {
        table_view(s, cx, table_rows(&m.temps, true))
    };
    let rows = u16::try_from(m.temps.len().saturating_add(1)).unwrap_or(u16::MAX);
    View::Stack {
        dir: Dir::V,
        children: vec![
            // The table takes its rows, not the whole tile: the RAPL, PSI
            // and gpu lines belong under the data (review).
            (Constraint::Len(rows.min(cx.inner.height)), table),
            (Constraint::Len(1), View::Text(vec![others])),
            (Constraint::Len(1), View::Text(vec![rapl_line(s)])),
            (Constraint::Len(1), View::Text(vec![psi_line(cx)])),
            (Constraint::Len(1), View::Text(vec![gpu_line(cx)])),
            (Constraint::Fill(1), View::Empty),
        ],
    }
}
