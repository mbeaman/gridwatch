//! The audio tiers as view trees (§8): `vu` (a stereo gauge pair), `mini`
//! (thin mono bars over the VU), `scope` (the oscilloscope over the VU),
//! `spectrum` (mirrored stereo bars with peak caps, the sink and levels
//! header, the Hz axis) and the zoom-only `full` (spectrum + scope + VU +
//! LUFS + chips). Every string is a theme role; the sink picker is a table.

use std::borrow::Cow;

use gridwatch_store::keys::audio::FLOOR_DB;
use gridwatch_store::{SourceState, keys::audio};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{ColWidth, Column, Constraint, Dir, Line, Span, View};

use super::{Audio, BarCount, Mode, TIER_MINI, TIER_SCOPE, TIER_SPECTRUM, TIER_VU};

pub fn render(a: &Audio, cx: &RenderCx<'_>) -> View {
    if let Some(p) = a.picker() {
        return picker(p, cx);
    }
    match cx.tier {
        TIER_VU => vu_tier(a, cx),
        TIER_MINI => mini(a, cx),
        TIER_SCOPE => scope_tier(a, cx),
        TIER_SPECTRUM => spectrum(a, cx, false),
        _ => full(a, cx),
    }
}

fn db_text(db: f64) -> String {
    if db <= FLOOR_DB + 0.5 {
        "−∞".into()
    } else {
        format!("{db:.0} dB").replace('-', "−")
    }
}

/// The sink's short name: its description, or the tail of its node name.
pub fn sink_short(a: &Audio, max: usize) -> Cow<'static, str> {
    let Some(s) = a.sink() else {
        return Cow::Borrowed("no audio");
    };
    let name = if s.description.is_empty() {
        s.name.rsplit('.').next().unwrap_or(&s.name).to_string()
    } else {
        s.description.clone()
    };
    let mut out: String = name.chars().take(max).collect();
    if out.is_empty() {
        out = "sink".into();
    }
    Cow::Owned(out)
}

/// The status line a degraded or unavailable source earns.
fn status_line(cx: &RenderCx<'_>) -> Option<Line> {
    let st = cx.store.status(audio::SOURCE);
    match st.state {
        SourceState::Unavailable | SourceState::Degraded => {
            let mut text = st.reason.as_deref().unwrap_or("unavailable").to_string();
            if let Some(h) = st.hint.as_deref() {
                text.push_str(" — ");
                text.push_str(h);
            }
            let role = if st.state == SourceState::Unavailable {
                Role::Crit
            } else {
                Role::Warn
            };
            Some(vec![Span::new(role, text)])
        }
        _ => None,
    }
}

fn gauge(label: &'static str, level: f32, text: String) -> View {
    View::Gauge {
        label: Cow::Borrowed(label),
        value: level,
        gradient: GradientId::Audio,
        text: Some(Cow::Owned(text)),
    }
}

/// The stereo VU pair: `L` and `R` gauges with the RMS and the held peak.
fn vu_pair(a: &Audio, wide: bool) -> Vec<(Constraint, View)> {
    let has = a.sink().is_some() || !a.silent();
    let mut out = Vec::with_capacity(2);
    for (ch, label) in [(0usize, "L"), (1, "R")] {
        let v = &a.vu[ch];
        let text = if !has && v.rms_db <= FLOOR_DB {
            "—".to_string()
        } else if wide {
            format!("{} · pk {}", db_text(v.rms_db), db_text(v.peak_db))
        } else {
            db_text(v.rms_db)
        };
        out.push((Constraint::Len(1), gauge(label, v.level(), text)));
    }
    out
}

fn header(a: &Audio, cx: &RenderCx<'_>, levels: bool) -> Line {
    if let Some(l) = status_line(cx) {
        return l;
    }
    let w = usize::from(cx.inner.width);
    let mut line: Line = vec![Span::bold(Role::Title, sink_short(a, w.min(28)))];
    if a.silent() && a.sink().is_some() {
        line.push(Span::new(Role::TextMuted, " · silent"));
    }
    if levels && w >= 40 && a.sink().is_some() {
        line.push(Span::new(
            Role::TextMuted,
            format!(
                "  L {} · R {}",
                db_text(a.vu[0].rms_db),
                db_text(a.vu[1].rms_db)
            ),
        ));
    }
    line
}

fn vu_tier(a: &Audio, cx: &RenderCx<'_>) -> View {
    let wide = cx.inner.width >= 24;
    let mut children = Vec::with_capacity(3);
    if cx.inner.height >= 3 {
        children.push((Constraint::Len(1), View::Text(vec![header(a, cx, false)])));
    }
    children.extend(vu_pair(a, wide));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// 8–10 thin mono bars (the max of both channels) over the VU.
fn mini(a: &Audio, cx: &RenderCx<'_>) -> View {
    let n = usize::from(cx.inner.width).clamp(1, 10);
    let (l, _) = a.resampled(0, n);
    let (r, _) = a.resampled(1, n);
    let values: Vec<f32> = l.iter().zip(&r).map(|(x, y)| x.max(*y)).collect();
    let vu_rows = if cx.inner.height >= 5 { 2 } else { 1 };
    let mut children = vec![(
        Constraint::Fill(1),
        View::Bars {
            values,
            gradient: GradientId::Audio,
            labels: None,
            peaks: None,
        },
    )];
    let pair = vu_pair(a, cx.inner.width >= 24);
    children.extend(pair.into_iter().take(vu_rows));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

fn scope_tier(a: &Audio, cx: &RenderCx<'_>) -> View {
    let mut line = header(a, cx, false);
    line.push(Span::new(Role::TextMuted, " · scope"));
    let mut children = vec![
        (Constraint::Len(1), View::Text(vec![line])),
        (
            Constraint::Fill(1),
            super::scope::chart(&a.scope, cx.inner.width),
        ),
    ];
    children.extend(vu_pair(a, cx.inner.width >= 24));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// Mirrored stereo bars: ⌊(w+1)/3⌋ thick (two cells + a gap) bars, the left
/// channel reversed on the left half so the bass meets in the middle.
pub fn mirrored(a: &Audio, width: u16) -> (Vec<f32>, Vec<f32>) {
    let w = usize::from(width);
    let total = match a.options().bars {
        BarCount::Fixed(n) => usize::from(n).clamp(2, w.max(2)),
        BarCount::Auto => (w + 1) / 3,
    }
    .max(2);
    let per = total / 2;
    let (l, lp) = a.resampled(0, per);
    let (r, rp) = a.resampled(1, per);
    let thick = matches!(a.options().bars, BarCount::Auto);
    let mut values = Vec::with_capacity(w);
    let mut peaks = Vec::with_capacity(w);
    let mut push = |v: f32, p: f32| {
        values.push(v);
        peaks.push(p);
        if thick {
            values.push(v);
            peaks.push(p);
            values.push(0.0);
            peaks.push(0.0);
        }
    };
    for i in (0..per).rev() {
        push(l[i], lp[i]);
    }
    for i in 0..per {
        push(r[i], rp[i]);
    }
    if thick && values.len() > w {
        values.truncate(w);
        peaks.truncate(w);
    }
    (values, peaks)
}

/// `30 · 1k · 16k` under the bars: the window's edges and its centre.
fn axis(a: &Audio, width: u16) -> Line {
    let w = a.window();
    let hz = |k: usize| -> String {
        let f = w.hz(k);
        if f >= 1_000.0 {
            format!("{:.0}k", f / 1_000.0)
        } else {
            format!("{f:.0}")
        }
    };
    let (lo, hi) = (hz(w.lo), hz(w.hi));
    let mid = hz(w.lo + w.len() / 2);
    // Mirrored: the bass meets in the middle, the treble at both edges.
    let text = if width >= 40 {
        format!("{hi} · {mid} · {lo} Hz · {mid} · {hi}")
    } else {
        format!("{hi} · {lo} Hz · {hi}")
    };
    vec![Span::new(Role::TextMuted, text)]
}

fn spectrum(a: &Audio, cx: &RenderCx<'_>, in_full: bool) -> View {
    let (values, peaks) = mirrored(a, cx.inner.width);
    let bars = View::Bars {
        values,
        gradient: GradientId::Audio,
        labels: None,
        peaks: Some(peaks),
    };
    let body = match a.mode() {
        Mode::Bars => bars,
        Mode::Scope => super::scope::chart(&a.scope, cx.inner.width),
        Mode::Both => View::Stack {
            dir: Dir::V,
            children: vec![
                (Constraint::Fill(2), bars),
                (
                    Constraint::Fill(1),
                    super::scope::chart(&a.scope, cx.inner.width),
                ),
            ],
        },
    };
    let mut children = Vec::with_capacity(4);
    if !in_full {
        children.push((Constraint::Len(1), View::Text(vec![header(a, cx, true)])));
    }
    children.push((Constraint::Fill(1), body));
    children.push((
        Constraint::Len(1),
        View::Text(vec![axis(a, cx.inner.width)]),
    ));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

fn full(a: &Audio, cx: &RenderCx<'_>) -> View {
    let lufs = |k: &gridwatch_store::Key<f64>| -> String {
        cx.store
            .last(k)
            .map(|(_, v)| format!("{v:.1}").replace('-', "−"))
            .unwrap_or_else(|| "—".into())
    };
    let chips: Line = vec![
        Span::new(Role::TextMuted, "LUFS "),
        Span::new(Role::Text, lufs(&audio::LUFS_M)),
        Span::new(Role::TextMuted, " M / "),
        Span::new(Role::Text, lufs(&audio::LUFS_S)),
        Span::new(Role::TextMuted, " S   ·   preset "),
        Span::new(Role::AccentSecondary, a.preset().name()),
        Span::new(Role::TextMuted, "   ·   mode "),
        Span::new(Role::AccentSecondary, a.mode().name()),
        Span::new(Role::TextMuted, "   ·   sink "),
        Span::new(Role::AccentPrimary, sink_short(a, 40)),
        Span::new(
            Role::TextMuted,
            a.sink()
                .map(|s| format!(" (serial {}, {} Hz, {})", s.serial, s.rate, s.state))
                .unwrap_or_default(),
        ),
    ];
    let mut children = vec![
        (Constraint::Len(1), View::Text(vec![header(a, cx, true)])),
        (Constraint::Fill(3), spectrum(a, cx, true)),
        (
            Constraint::Fill(1),
            super::scope::chart(&a.scope, cx.inner.width),
        ),
    ];
    children.extend(vu_pair(a, true));
    children.push((Constraint::Len(1), View::Text(vec![chips])));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The sink picker: `SINK · STATE · DEFAULT`, the selection highlighted.
fn picker(p: &super::Picker, cx: &RenderCx<'_>) -> View {
    let columns = vec![
        Column {
            title: "sink".into(),
            width: ColWidth::Elastic,
            right: false,
        },
        Column {
            title: "state".into(),
            width: ColWidth::Fixed(9),
            right: false,
        },
        Column {
            title: "default".into(),
            width: ColWidth::Fixed(7),
            right: false,
        },
    ];
    let rows: Vec<Vec<Line>> = p
        .sinks
        .iter()
        .map(|s| {
            let name = if s.description.is_empty() {
                s.name.clone()
            } else {
                format!("{} ({})", s.description, s.name)
            };
            vec![
                vec![Span::new(Role::Text, name)],
                vec![Span::new(
                    if s.state == "running" {
                        Role::Ok
                    } else {
                        Role::TextMuted
                    },
                    s.state.clone(),
                )],
                vec![Span::new(
                    Role::AccentPrimary,
                    if s.is_default { "✓" } else { "" },
                )],
            ]
        })
        .collect();
    let hint: Line = vec![Span::new(
        Role::TextMuted,
        if p.sinks.is_empty() {
            "pick a sink — waiting for pw-dump…  Esc closes"
        } else {
            "pick a sink — ↑/↓ move · Enter choose · Esc closes"
        },
    )];
    let _ = cx;
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![hint])),
            (
                Constraint::Fill(1),
                View::Table {
                    columns,
                    rows,
                    selected: (!p.sinks.is_empty()).then_some(p.selected),
                    sort: None,
                    scroll: 0,
                },
            ),
        ],
    }
}
