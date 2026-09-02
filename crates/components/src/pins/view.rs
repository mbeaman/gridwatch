//! The pins tier builders (§8, brief arc 3 seam 4): tui.rs's panels as a
//! view tree — the watts badge, six eighth-block bars with the 9.2 A limit
//! line, the balance gauge, the watts sparkline, the alert log, and the
//! device header read from the gpu source's keys. Colours are roles chosen by
//! meaning: `Crit` over the overload amperage, `Warn` over the alarm band.

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::{gpu, pins};
use gridwatch_store::{Agg, Severity, Store, Transition};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{Bounds, Constraint, Dir, MarkerHint, Series, Span, View};

use super::limit::PinBars;
use super::model::{BalanceClass, Model, balance_class};
use super::{Pins, TIER_BADGE, TIER_BARS, TIER_FULL, TIER_MINI, TIER_TREND};

/// Six roles as *series identity* for the six pins (tui.rs's PIN_COLORS);
/// components never name a colour.
pub const PIN_ROLES: [Role; 6] = [
    Role::AccentPrimary,
    Role::AccentSecondary,
    Role::AccentTertiary,
    Role::Info,
    Role::Ok,
    Role::Warn,
];

fn muted(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::TextMuted, s)
}

fn ghost(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::TextGhost, s)
}

fn value(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::Text, s)
}

/// The amps band (tui.rs `pin_color`): red over the limit, yellow over the
/// alarm band, dark at zero or stale, else green.
fn amps_role(a: Option<f64>, m: &Model, stale: bool) -> Role {
    match a {
        None => Role::TextGhost,
        Some(_) if stale => Role::TextMuted,
        Some(a) if a > m.overload_a() => Role::Crit,
        Some(a) if a > m.warn_a() => Role::Warn,
        Some(a) if a <= 0.0 => Role::TextMuted,
        Some(_) => Role::Ok,
    }
}

fn balance_role(c: BalanceClass) -> Role {
    match c {
        BalanceClass::Idle => Role::TextMuted,
        BalanceClass::Normal => Role::Ok,
        BalanceClass::Warn => Role::Warn,
        BalanceClass::Alarm => Role::Crit,
        BalanceClass::Unknown => Role::TextGhost,
    }
}

fn watts(w: Option<f64>) -> String {
    match w {
        Some(w) => format!("{} W", w.round() as i64),
        None => "— W".into(),
    }
}

fn amps(a: Option<f64>) -> String {
    match a {
        Some(a) => format!("{a:.1}"),
        None => "—".into(),
    }
}

fn balance_text(b: Option<f64>) -> String {
    match b {
        Some(b) if b.is_finite() => format!("{b:.2}×"),
        _ => "—".into(),
    }
}

/// Is the tile's picture stale: telemetry lost, or the last *reading* older
/// than 3 × the source interval? A frozen (`p`) tile is PAUSED, not stale.
fn stale(p: &Pins, cx: &RenderCx<'_>) -> bool {
    if p.frozen() {
        return false;
    }
    let m = p.model();
    if m.telemetry_lost() {
        return true;
    }
    let interval_ms = m.info.as_ref().map(|i| i.interval_ms).unwrap_or(500);
    match m.last_reading {
        Some(at) => cx.now.since(at) > Duration::from_millis(u64::from(interval_ms) * 3),
        None => false,
    }
}

/// The marker after a total: ` PAUSED` while frozen, ` STALE` when stale.
fn marker(p: &Pins, cx: &RenderCx<'_>) -> Option<Span> {
    if p.frozen() {
        Some(Span::bold(Role::Warn, " PAUSED"))
    } else if stale(p, cx) {
        Some(muted(" STALE"))
    } else {
        None
    }
}

/// A source that is not `Ok` says why in the tile (review: an unavailable
/// source showed only dashes).
fn source_status(cx: &RenderCx<'_>) -> Option<View> {
    let st = cx.store.status(pins::SOURCE);
    if matches!(
        st.state,
        gridwatch_store::SourceState::Ok | gridwatch_store::SourceState::Starting
    ) {
        return None;
    }
    let mut line = vec![Span::new(
        Role::Warn,
        format!("pins: {}", st.reason.as_deref().unwrap_or("unavailable")),
    )];
    if let Some(h) = &st.hint {
        line.push(ghost(format!(" — {h}")));
    }
    Some(View::Text(vec![line]))
}

/// The worst active condition's glyph and role.
fn alert_glyph(m: &Model, cx: &RenderCx<'_>) -> Option<Span> {
    let worst = cx
        .store
        .alerts()
        .active()
        .filter(|(id, _)| id.0.starts_with("pins/"))
        .map(|(_, a)| a.severity)
        .max();
    // The theme owns the severity glyphs (review).
    let glyph = |s: Severity| cx.theme.severity(s).1;
    match worst {
        Some(Severity::Crit) => Some(Span::bold(Role::Crit, glyph(Severity::Crit))),
        Some(Severity::Warn) => Some(Span::bold(Role::Warn, glyph(Severity::Warn))),
        Some(Severity::Info) => Some(Span::new(Role::Info, glyph(Severity::Info))),
        None if m.telemetry_lost() => Some(Span::new(Role::Info, "?")),
        None if m.at.is_some() => Some(ghost("·")),
        None => None,
    }
}

// ---------------------------------------------------------------- badge

fn badge(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let st = stale(p, cx);
    let w = cx.inner.width;
    if m.at.is_none()
        && let Some(status) = source_status(cx)
    {
        return status;
    }
    let mut line1 = vec![];
    if w >= 10 {
        line1.push(muted("Σ "));
    }
    line1.push(Span::bold(
        if st { Role::TextMuted } else { Role::Text },
        watts(m.total_w),
    ));
    let class = balance_class(m.balance, m.total_a, m.min_load_a(), m.imbalance_ratio());
    let mut line2 = vec![Span::new(balance_role(class), balance_text(m.balance))];
    if let Some(g) = alert_glyph(m, cx) {
        line2.push(ghost(" "));
        line2.push(g);
    }
    if let Some(mk) = marker(p, cx) {
        line2.push(mk);
    }
    let mut children = vec![
        (Constraint::Len(1), View::Text(vec![line1])),
        (Constraint::Len(1), View::Text(vec![line2])),
    ];
    if cx.inner.height >= 3 {
        children.push((Constraint::Fill(1), watts_spark(p, cx, cx.inner.width)));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- bars

/// Six bars scaled to `AMPS_CEILING` with session-peak caps and the limit
/// line painted over the empty cells (`View::Custom`, §4.6).
fn overlaid_bars(p: &Pins, _cx: &RenderCx<'_>, with_labels: bool) -> View {
    let m = p.model();
    let values: Vec<f32> = m
        .amps
        .iter()
        .map(|a| (a.unwrap_or(0.0) / pins::AMPS_CEILING) as f32)
        .collect();
    let peaks: Vec<f32> = m
        .peaks
        .iter()
        .map(|a| (a / pins::AMPS_CEILING) as f32)
        .collect();
    View::Custom {
        paint: Box::new(PinBars {
            values,
            peaks,
            labels: with_labels.then(|| {
                (1..=6)
                    .map(|i| Cow::Owned(i.to_string()))
                    .collect::<Vec<Cow<'static, str>>>()
            }),
            limit_frac: (m.overload_a() / pins::AMPS_CEILING) as f32,
        }),
        describe: format!(
            "six pin bars over {:.0} A with the limit line at {:.1} A",
            pins::AMPS_CEILING,
            m.overload_a()
        )
        .into(),
    }
}

fn mini_bars(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let st = stale(p, cx);
    let mut top = vec![];
    if let Some(g) = alert_glyph(m, cx) {
        top.push(g);
        top.push(ghost(" "));
    }
    top.push(muted("Σ "));
    top.push(Span::bold(
        if st { Role::TextMuted } else { Role::Text },
        watts(m.total_w),
    ));
    if let Some(mk) = marker(p, cx) {
        top.push(mk);
    }
    let mut children = vec![(Constraint::Len(1), View::Text(vec![top]))];
    if let Some(status) = source_status(cx) {
        children.push((Constraint::Len(1), status));
    }
    children.push((Constraint::Fill(1), overlaid_bars(p, cx, false)));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// `p1 1.7 p2 1.6 …`; below 46 cells the space after the label goes
/// (`p1:1.7`) so the row fits the `bars` tier's 40-wide minimum (review).
fn pin_values_line(m: &Model, st: bool, width: u16) -> View {
    let compact = width < 46;
    let mut line = Vec::new();
    for (i, a) in m.amps.iter().enumerate() {
        if i > 0 {
            line.push(ghost(" "));
        }
        line.push(muted(if compact {
            format!("{}:", i + 1)
        } else {
            format!("p{} ", i + 1)
        }));
        line.push(Span::new(amps_role(*a, m, st), amps(*a)));
    }
    View::Text(vec![line])
}

/// `9.2 A · 111 W · peak 620 W · pins 12.06–12.08 V`, led by `PAUSED ·` or
/// `STALE ·` (tui.rs's prefix; at the end it fell off a 40-wide tile).
fn totals_line(m: &Model, st: bool, frozen: bool) -> View {
    let mut line = Vec::new();
    if frozen {
        line.push(Span::bold(Role::Warn, "PAUSED · "));
    } else if st {
        line.push(Span::bold(Role::Warn, "STALE · "));
    }
    line.extend([
        Span::new(
            if st { Role::TextMuted } else { Role::Text },
            match m.total_a {
                Some(a) => format!("{a:.1} A"),
                None => "— A".into(),
            },
        ),
        ghost(" · "),
        value(watts(m.total_w)),
    ]);
    if m.peak_w > 0.0 {
        line.push(ghost(" · "));
        line.push(muted(format!("peak {} W", m.peak_w.round() as i64)));
    }
    if let Some((lo, hi)) = m.volt_range() {
        line.push(ghost(" · "));
        line.push(muted(format!("pins {lo:.2}–{hi:.2} V")));
    }
    View::Text(vec![line])
}

fn balance_gauge(m: &Model) -> View {
    let class = balance_class(m.balance, m.total_a, m.min_load_a(), m.imbalance_ratio());
    let ratio = m.imbalance_ratio();
    let frac = match (class, m.balance) {
        (BalanceClass::Idle, _) | (BalanceClass::Unknown, _) => 0.0,
        (_, Some(b)) => ((b - 1.0) / (ratio - 1.0)).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    };
    View::Gauge {
        label: "balance".into(),
        value: frac,
        gradient: GradientId::Temp,
        text: Some(format!("{} {}", balance_text(m.balance), class.label()).into()),
    }
}

fn bars(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let st = stale(p, cx);
    let mut children = Vec::new();
    if let Some(status) = source_status(cx) {
        children.push((Constraint::Len(1), status));
    }
    children.extend([
        (Constraint::Fill(1), overlaid_bars(p, cx, true)),
        (Constraint::Len(1), pin_values_line(m, st, cx.inner.width)),
        (Constraint::Len(1), balance_gauge(m)),
        (Constraint::Len(1), totals_line(m, st, p.frozen())),
    ]);
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- trend

fn history_span(p: &Pins, cx: &RenderCx<'_>) -> Duration {
    let interval_ms = p
        .model()
        .info
        .as_ref()
        .map(|i| i.interval_ms)
        .unwrap_or(500);
    let span = Duration::from_millis(u64::from(interval_ms) * u64::from(p.options().history));
    Duration::from_nanos(cx.now.0)
        .min(span)
        .max(Duration::from_secs(1))
}

fn watts_spark(p: &Pins, cx: &RenderCx<'_>, width: u16) -> View {
    let mut out = Vec::new();
    let span = history_span(p, cx);
    // No more buckets than samples: a young run draws a short continuous
    // line at the right rather than a comb of blanks (review).
    let interval_ms = p
        .model()
        .info
        .as_ref()
        .map(|i| u64::from(i.interval_ms))
        .unwrap_or(500)
        .max(1);
    let samples = (span.as_millis() as u64 / interval_ms).max(1);
    let buckets = (u64::from(width.max(1))).min(samples) as usize;
    cx.store
        .resample(&pins::TOTAL_W, span, buckets, Agg::Avg, &mut out);
    View::Sparkline {
        series: out.into_iter().map(|v| v.map(|v| v as f32)).collect(),
        gradient: GradientId::Power,
        max: None,
    }
}

/// The active-alert row (tui.rs `draw_alarm`): Crit conditions joined with
/// ` + `; only advisories → the Warn wording; nothing when clear.
fn alarm_row(cx: &RenderCx<'_>) -> Option<View> {
    let mut crit = Vec::new();
    let mut warn = Vec::new();
    for (id, a) in cx.store.alerts().active() {
        if !id.0.starts_with("pins/") {
            continue;
        }
        match a.severity {
            Severity::Crit => crit.push(a.title.to_string()),
            Severity::Warn => warn.push(a.title.to_string()),
            Severity::Info => {}
        }
    }
    if !crit.is_empty() {
        return Some(View::Text(vec![vec![Span::bold(
            Role::Crit,
            format!("⚠ ALERT: {} ⚠", crit.join(" + ")),
        )]]));
    }
    if !warn.is_empty() {
        return Some(View::Text(vec![vec![Span::bold(
            Role::Warn,
            warn.join(" + "),
        )]]));
    }
    None
}

/// The alert log: the store's event ring filtered to this source, newest at
/// the bottom (tui.rs), scrolled by the component.
fn log(p: &Pins, cx: &RenderCx<'_>, rows: usize) -> View {
    let events: Vec<_> = cx
        .store
        .alerts()
        .events()
        .filter(|e| e.source == pins::SOURCE)
        .collect();
    let mut lines: Vec<Vec<Span>> = Vec::new();
    lines.push(vec![Span::bold(Role::TextMuted, "log")]);
    if events.is_empty() {
        lines.push(vec![ghost("no alerts this session")]);
    } else {
        let n = events.len();
        // The header and the hint line live inside `rows`.
        let hint = usize::from(n > rows.saturating_sub(1).max(1));
        let body = rows.saturating_sub(1 + hint).max(1);
        let end = n.saturating_sub(p.log_scroll().min(n.saturating_sub(1)));
        let start = end.saturating_sub(body);
        for e in &events[start..end] {
            let role = match (e.transition, e.severity) {
                (Transition::Resolved, _) => Role::Ok,
                (_, Severity::Crit) => Role::Crit,
                (_, Severity::Warn) => Role::Warn,
                (_, Severity::Info) => Role::Info,
            };
            let when = format!("{:>6.1}s ", e.at.as_secs_f64());
            let head = crate::alerts::headline(&e.title, &e.detail);
            let what = match e.transition {
                Transition::Raised => format!("RAISED {head}"),
                Transition::Repeated => format!("ACTIVE {head}"),
                Transition::Resolved => format!("RESOLVED {} {}", e.title, e.detail),
            };
            lines.push(vec![ghost(when), Span::new(role, what)]);
        }
        if start > 0 || end < n {
            lines.push(vec![ghost(format!("({}–{} of {n}; ↑/↓)", start + 1, end))]);
        }
    }
    View::Text(lines)
}

fn trend(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let st = stale(p, cx);
    let h = cx.inner.height;
    let alarm = alarm_row(cx);
    let status = source_status(cx);
    // bars 6 + values + balance + totals + sparkline 3 + alarm/status rows + log ≥ 3.
    let fixed = 6 + 3 + 3 + u16::from(alarm.is_some()) + u16::from(status.is_some());
    let log_rows = h.saturating_sub(fixed).max(3);
    let mut children = Vec::new();
    if let Some(s) = status {
        children.push((Constraint::Len(1), s));
    }
    children.extend([
        (Constraint::Len(6), overlaid_bars(p, cx, true)),
        (Constraint::Len(1), pin_values_line(m, st, cx.inner.width)),
        (Constraint::Len(1), balance_gauge(m)),
        (Constraint::Len(1), totals_line(m, st, p.frozen())),
        (Constraint::Len(3), watts_spark(p, cx, cx.inner.width)),
    ]);
    if let Some(row) = alarm {
        children.push((Constraint::Len(1), row));
    }
    children.push((Constraint::Fill(1), log(p, cx, usize::from(log_rows))));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- full

fn scalar(store: &Store, key: &gridwatch_store::Key<f64>) -> Option<f64> {
    store.last(key).map(|(_, v)| v)
}

/// tui.rs's device header from the gpu source (never sysfs or nvidia-smi).
fn device_header(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let store = cx.store;
    let st = stale(p, cx);
    let info = store.record(&gpu::INFO.idx(0)).map(|(_, i)| i.clone());
    let mut l1 = vec![];
    let model = m
        .info
        .as_ref()
        .and_then(|i| i.model.clone())
        .or_else(|| info.as_ref().map(|i| i.name.clone()))
        .unwrap_or_else(|| "—".into());
    l1.push(Span::bold(Role::Title, model));
    if let Some(i) = &m.info {
        l1.push(ghost(" · "));
        l1.push(muted(if i.pci.is_empty() {
            "—".to_string()
        } else {
            i.pci.clone()
        }));
    }
    let generation = scalar(store, &gpu::PCIE_GEN.idx(0));
    let width = scalar(store, &gpu::PCIE_WIDTH.idx(0));
    if let (Some(g), Some(w)) = (generation, width) {
        l1.push(ghost(" · "));
        // Below Gen5×16 → Warn, as tui.rs does against the card's max link;
        // the gpu source does not publish the max yet (PARITY records the
        // "this card" rule; BACKLOG has the general one).
        let below = info.is_some() && (g < 5.0 || w < 16.0);
        l1.push(Span::new(
            if below { Role::Warn } else { Role::Text },
            format!(
                "PCIe Gen{}×{}{}",
                g as u8,
                w as u8,
                if below { " ↓" } else { "" }
            ),
        ));
    }
    if let Some(i) = &m.info {
        l1.push(ghost(" · "));
        l1.push(muted(match (i.mode, i.bus) {
            (pins::PinsMode::I2c, Some(b)) => format!("i2c-{b} @ {:#04x}", i.addr),
            (pins::PinsMode::Exporter, _) => format!("exporter {}", i.pci),
            _ => i.mode.label().to_string(),
        }));
        if m.state
            .as_ref()
            .is_some_and(|s| !s.service_active.is_empty())
        {
            l1.push(ghost(" · "));
            l1.push(Span::new(Role::Info, "svc"));
        }
    }
    if p.frozen() {
        l1.push(ghost(" · "));
        l1.push(Span::bold(Role::Warn, "PAUSED"));
    }
    // Line 2: the gpu source's numbers with tui.rs's colour bands as roles.
    let util = scalar(store, &gpu::UTIL_PCT.idx(0));
    let pw = scalar(store, &gpu::POWER_W.idx(0));
    let limit = scalar(store, &gpu::POWER_LIMIT_W.idx(0));
    let temp = scalar(store, &gpu::TEMP_C.idx(0));
    let fan = store
        .last(&gpu::FAN_PCT.named(&gpu::fan_label(0, 0)))
        .map(|(_, v)| v);
    let pct = |v: Option<f64>| {
        v.map(|v| format!("{}%", v.round() as i64))
            .unwrap_or("—".into())
    };
    let pw_role = match (pw, limit) {
        (Some(p), Some(l)) if l > 0.0 && p / l >= 0.97 => Role::Crit,
        (Some(p), Some(l)) if l > 0.0 && p / l >= 0.85 => Role::Warn,
        (Some(_), _) => Role::Ok,
        _ => Role::TextGhost,
    };
    let temp_role = match temp {
        Some(t) if t >= 85.0 => Role::Crit,
        Some(t) if t >= 75.0 => Role::Warn,
        Some(_) => Role::Ok,
        None => Role::TextGhost,
    };
    let l2 = vec![
        muted("GPU "),
        value(pct(util)),
        ghost(" · "),
        muted("PWR "),
        Span::new(
            pw_role,
            match (pw, limit) {
                (Some(p), Some(l)) => format!("{}/{}W", p.round() as i64, l.round() as i64),
                (Some(p), None) => format!("{}W", p.round() as i64),
                _ => "—".into(),
            },
        ),
        ghost(" · "),
        Span::new(
            temp_role,
            temp.map(|t| format!("{}°C", t.round() as i64))
                .unwrap_or("—".into()),
        ),
        ghost(" · "),
        muted("fan "),
        value(pct(fan)),
    ];
    // Line 3: the connector line, `STALE ·` prefixed when the picture is old.
    let mut l3 = vec![];
    if st {
        l3.push(Span::bold(Role::Warn, "STALE · "));
    }
    l3.push(muted("connector "));
    l3.push(value(match m.total_a {
        Some(a) => format!("{a:.1} A"),
        None => "— A".into(),
    }));
    l3.push(ghost(" · "));
    l3.push(value(watts(m.total_w)));
    l3.push(ghost(" · "));
    l3.push(muted("balance "));
    let class = balance_class(m.balance, m.total_a, m.min_load_a(), m.imbalance_ratio());
    l3.push(Span::new(balance_role(class), balance_text(m.balance)));
    if let Some((lo, hi)) = m.volt_range() {
        l3.push(ghost(" · "));
        l3.push(muted(format!("pins {lo:.2}–{hi:.2} V")));
    }
    l3.push(ghost(" · "));
    l3.push(muted(format!("samples {}", m.samples)));
    View::Text(vec![l1, l2, l3])
}

/// The six-pin braille trend (tui.rs `draw_trend`): y max = max(limit, peak).
fn pin_trend(p: &Pins, cx: &RenderCx<'_>, width: u16) -> View {
    let m = p.model();
    let span = history_span(p, cx);
    let buckets = usize::from(width.max(1));
    let mut series = Vec::with_capacity(6);
    let mut out = Vec::new();
    let mut y_max: f64 = m.overload_a();
    for pin in 1..=pins::PIN_COUNT {
        cx.store
            .resample(&pins::AMPS.idx(pin), span, buckets, Agg::Avg, &mut out);
        let data: Vec<(f64, f64)> = out
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (i as f64, v)))
            .collect();
        for (_, v) in &data {
            y_max = y_max.max(*v);
        }
        series.push(Series {
            label: Cow::Owned(format!("p{pin}")),
            gradient: GradientId::Power,
            data,
        });
    }
    let _ = PIN_ROLES; // series identity is the legend's job (below)
    if series.iter().all(|s| s.data.is_empty()) {
        return View::Text(vec![vec![ghost("trend: waiting for samples…")]]);
    }
    let legend: Vec<Span> = (0..6)
        .flat_map(|i| {
            [
                Span::new(PIN_ROLES[i], format!("p{}", i + 1)),
                ghost(if i < 5 { " " } else { "" }),
            ]
        })
        .collect();
    View::Stack {
        dir: Dir::V,
        children: vec![
            (
                Constraint::Len(1),
                View::Text(vec![{
                    let mut l = vec![muted(format!(
                        "trend · {:.0} A ceiling · {}s",
                        y_max.ceil(),
                        span.as_secs()
                    ))];
                    l.push(ghost("  "));
                    l.extend(legend);
                    l
                }]),
            ),
            (
                Constraint::Fill(1),
                View::Chart {
                    series,
                    bounds: Bounds {
                        x: (0.0, (buckets.saturating_sub(1)).max(1) as f64),
                        y: (0.0, y_max.ceil().max(1.0)),
                    },
                    marker: MarkerHint::Auto,
                },
            ),
        ],
    }
}

fn full(p: &Pins, cx: &RenderCx<'_>) -> View {
    let m = p.model();
    let st = stale(p, cx);
    let w = cx.inner.width;
    let left_w = w * 58 / 100;
    let left = View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Fill(45), overlaid_bars(p, cx, true)),
            (Constraint::Len(1), pin_values_line(m, st, left_w)),
            (Constraint::Fill(55), pin_trend(p, cx, left_w)),
        ],
    };
    let right = View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), totals_line(m, st, p.frozen())),
            (Constraint::Len(1), balance_gauge(m)),
            (
                Constraint::Len(3),
                watts_spark(p, cx, w.saturating_sub(left_w + 1)),
            ),
            (
                Constraint::Fill(1),
                log(p, cx, usize::from(cx.inner.height.saturating_sub(9))),
            ),
        ],
    };
    let mut children = vec![(Constraint::Len(3), device_header(p, cx))];
    if let Some(s) = source_status(cx) {
        children.push((Constraint::Len(1), s));
    }
    if let Some(row) = alarm_row(cx) {
        children.push((Constraint::Len(1), row));
    }
    children.push((
        Constraint::Fill(1),
        View::Stack {
            dir: Dir::H,
            children: vec![
                (Constraint::Len(left_w), left),
                (Constraint::Len(1), View::Empty),
                (Constraint::Fill(1), right),
            ],
        },
    ));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

pub fn render(p: &Pins, cx: &RenderCx<'_>) -> View {
    match cx.tier {
        TIER_BADGE => badge(p, cx),
        TIER_MINI => mini_bars(p, cx),
        TIER_BARS => bars(p, cx),
        TIER_TREND => trend(p, cx),
        TIER_FULL => full(p, cx),
        _ => full(p, cx),
    }
}
