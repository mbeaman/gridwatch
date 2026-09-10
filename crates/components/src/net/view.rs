//! The net tiers as view trees (§8): `rates` (one rate pair with a link
//! dot), `sparks` (+ sparklines and the speed or SSID), `table` (every
//! shown interface with drops and errors, and the probe strip), `conns`
//! (+ the connection table) and the zoom-only `full` (+ the route, the
//! per-interface detail and the probe statistics).

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::net::{self, Link, LinkKind};
use gridwatch_store::{Agg, SourceState};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{
    Bounds, ColWidth, Column, Constraint, Dir, Line, MarkerHint, Series, Span, View,
};

use super::{Iface, Net, TIER_CONNS, TIER_RATES, TIER_SPARKS, TIER_TABLE};

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

/// The dot beside an interface: up, no carrier, down.
fn dot(i: &Iface) -> Span {
    let (role, glyph) = match i.link.as_ref() {
        Some(l) if l.up && l.carrier => (Role::Ok, "●"),
        Some(l) if l.up => (Role::Warn, "◍"),
        Some(_) => (Role::TextMuted, "○"),
        None => (Role::TextGhost, "·"),
    };
    Span::new(role, glyph)
}

fn status_line(cx: &RenderCx<'_>) -> Option<Line> {
    let st = cx.store.status(net::SOURCE);
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
        status_line(cx).unwrap_or_else(|| vec![Span::new(Role::TextMuted, "— no interfaces")]);
    View::Text(vec![line])
}

pub fn render(n: &Net, cx: &RenderCx<'_>) -> View {
    if n.model().ifaces.is_empty() {
        return empty(cx);
    }
    match cx.tier {
        TIER_RATES => rates(n, cx),
        TIER_SPARKS => sparks(n, cx),
        TIER_TABLE => table_tier(n, cx, false),
        TIER_CONNS => table_tier(n, cx, true),
        _ => full(n, cx),
    }
}

/// `↓ 1.2M ↑ 340k` for the interface the default route uses.
fn rates(n: &Net, cx: &RenderCx<'_>) -> View {
    let Some(i) = n.model().primary() else {
        return empty(cx);
    };
    let w = usize::from(cx.inner.width);
    let mut head: Line = vec![dot(i)];
    if w >= 14 {
        head.push(Span::new(Role::TextMuted, format!(" {}", i.name)));
    }
    let down: Line = vec![
        Span::new(Role::AccentPrimary, "↓ "),
        Span::bold(Role::Text, rate(i.rx_bps)),
    ];
    let up: Line = vec![
        Span::new(Role::AccentSecondary, "↑ "),
        Span::bold(Role::Text, rate(i.tx_bps)),
    ];
    let mut children = vec![(Constraint::Len(1), View::Text(vec![head]))];
    if cx.inner.height >= 3 {
        children.push((Constraint::Len(1), View::Text(vec![down])));
        children.push((Constraint::Len(1), View::Text(vec![up])));
    } else {
        let mut one = down;
        one.push(Span::new(Role::TextMuted, "  "));
        one.extend(up);
        children.push((Constraint::Len(1), View::Text(vec![one])));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The window a rate sparkline covers, at most.
const SPARK_SPAN: Duration = Duration::from_secs(120);

/// A sparkline of one key over the last minutes. The window is the run's age
/// capped at `SPARK_SPAN`, the way htop's, gpu's and pins' sparklines already
/// take it (D62 §1 — "the run's age capped at the component's span"): with a
/// fixed span, a run younger than two minutes has no samples for the oldest
/// buckets, so the line starts part-way across the rect and the left of the
/// tile is blank (the renderer holds nothing before the first sample).
fn spark(cx: &RenderCx<'_>, key: &gridwatch_store::Key<f64>, iface: &str) -> View {
    let mut buf = Vec::new();
    let buckets = usize::from(cx.inner.width).max(2);
    let span = Duration::from_nanos(cx.now.0)
        .min(SPARK_SPAN)
        .max(Duration::from_secs(1));
    cx.store.resample(
        &key.named(&std::sync::Arc::from(iface)),
        span,
        buckets,
        Agg::Max,
        &mut buf,
    );
    View::Sparkline {
        series: buf.iter().map(|v| v.map(|v| v as f32)).collect(),
        gradient: GradientId::NetRx,
        max: None,
    }
}

/// What a link says about itself in one phrase.
fn link_text(l: &Link) -> String {
    if let Some(w) = l.wifi.as_ref() {
        return format!("{} {} dBm", w.ssid, w.signal_dbm);
    }
    if l.speed_mbps > 0 {
        if l.speed_mbps >= 1000 {
            format!("{} Gb/s", l.speed_mbps / 1000)
        } else {
            format!("{} Mb/s", l.speed_mbps)
        }
    } else if l.kind == LinkKind::Wifi {
        "no radio link".into()
    } else {
        "speed unknown".into()
    }
}

fn sparks(n: &Net, cx: &RenderCx<'_>) -> View {
    let Some(i) = n.model().primary() else {
        return empty(cx);
    };
    let mut head: Line = vec![
        dot(i),
        Span::new(Role::TextMuted, format!(" {} ", i.name)),
        Span::new(Role::AccentPrimary, "↓ "),
        Span::bold(Role::Text, rate(i.rx_bps)),
        Span::new(Role::AccentSecondary, "  ↑ "),
        Span::bold(Role::Text, rate(i.tx_bps)),
    ];
    if let Some(l) = i.link.as_ref()
        && cx.inner.width >= 34
    {
        head.push(Span::new(Role::TextMuted, format!("  {}", link_text(l))));
    }
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![head])),
            (Constraint::Fill(1), spark(cx, &net::RX_BPS, &i.name)),
            (Constraint::Fill(1), spark(cx, &net::TX_BPS, &i.name)),
        ],
    }
}

/// `gw 1.4 ms · 1.1.1.1 12 ms · 0 % loss`, or why there is no ICMP.
fn probe_line(n: &Net) -> Option<Line> {
    let p = n.model().probes.as_ref()?;
    if p.targets.is_empty() {
        return None;
    }
    let mut line: Line = Vec::new();
    for (i, t) in p.targets.iter().enumerate() {
        if i > 0 {
            line.push(Span::new(Role::TextMuted, " · "));
        }
        line.push(Span::new(Role::TextMuted, format!("{} ", t.target)));
        let role = if t.loss_pct > 20.0 {
            Role::Crit
        } else if t.loss_pct > 0.0 || t.avg_ms > 100.0 {
            Role::Warn
        } else {
            Role::Text
        };
        line.push(Span::new(role, format!("{:.1} ms", t.avg_ms)));
        if t.loss_pct > 0.0 {
            line.push(Span::new(Role::Warn, format!(" {:.0}% loss", t.loss_pct)));
        }
    }
    if let Some(d) = p.degraded.as_ref() {
        line.push(Span::new(Role::Warn, format!("  ({d})")));
    } else if p.targets.iter().all(|t| t.kind == net::ProbeKind::Tcp) {
        line.push(Span::new(Role::TextMuted, "  (tcp)"));
    }
    Some(line)
}

fn iface_rows(n: &Net, with_errors: bool) -> Vec<Vec<Line>> {
    n.model()
        .ifaces
        .iter()
        .map(|i| {
            let mut row = vec![
                vec![dot(i), Span::new(Role::Text, format!(" {}", i.name))],
                vec![Span::new(
                    if i.up() { Role::Ok } else { Role::TextMuted },
                    i.state().to_string(),
                )],
                vec![Span::bold(Role::Text, rate(i.rx_bps))],
                vec![Span::bold(Role::Text, rate(i.tx_bps))],
            ];
            if with_errors {
                let drops = i.rx_drop + i.tx_drop;
                let errs = i.rx_err + i.tx_err;
                row.push(vec![Span::new(
                    if drops > 0.0 {
                        Role::Warn
                    } else {
                        Role::TextMuted
                    },
                    format!("{drops:.0}"),
                )]);
                row.push(vec![Span::new(
                    if errs > 0.0 {
                        Role::Crit
                    } else {
                        Role::TextMuted
                    },
                    format!("{errs:.0}"),
                )]);
            }
            row
        })
        .collect()
}

/// The interface table. It scrolls with the same cursor the connection
/// table uses: with `a` pressed, torch shows nine interfaces and the tier
/// gives this table a few rows, so `↑/↓` has to reach the ones below the
/// fold — the key was advertised and did nothing here (arc 7a review).
fn iface_table(n: &Net, cx: &RenderCx<'_>, body: usize) -> View {
    let with_errors = cx.inner.width >= 60;
    let mut columns = vec![
        Column {
            title: "iface".into(),
            width: ColWidth::Elastic,
            right: false,
        },
        Column {
            title: "state".into(),
            width: ColWidth::Fixed(10),
            right: false,
        },
        Column {
            title: "rx".into(),
            width: ColWidth::Fixed(8),
            right: true,
        },
        Column {
            title: "tx".into(),
            width: ColWidth::Fixed(8),
            right: true,
        },
    ];
    if with_errors {
        columns.push(Column {
            title: "drop".into(),
            width: ColWidth::Fixed(5),
            right: true,
        });
        columns.push(Column {
            title: "err".into(),
            width: ColWidth::Fixed(5),
            right: true,
        });
    }
    let rows = iface_rows(n, with_errors);
    let body = body.max(1);
    let cursor = n.scroll().min(rows.len().saturating_sub(1));
    let top = cursor
        .saturating_sub(body.saturating_sub(1))
        .min(rows.len().saturating_sub(body.min(rows.len())));
    View::Table {
        columns,
        selected: (cx.captured && rows.len() > body).then_some(cursor),
        rows,
        sort: None,
        scroll: top,
    }
}

/// The connection table, scrolled by the cursor. `body` is the rows the band
/// it is placed in actually has — it used to be the *tile's* inner height,
/// which at the `conns` tier is nearly twice the band, so a cursor sixteen
/// rows below the fold stayed off screen (§4.6, D65 §5).
fn conn_table(n: &Net, cx: &RenderCx<'_>, body: usize) -> View {
    let Some(c) = n.model().conns.as_ref() else {
        return View::Text(vec![vec![Span::new(
            Role::TextMuted,
            "connections: zoom or widen the tile (the scan runs at table detail)",
        )]]);
    };
    let rows: Vec<Vec<Line>> = c
        .rows
        .iter()
        .map(|r| {
            vec![
                vec![Span::new(Role::TextMuted, r.proto.name())],
                vec![Span::new(Role::Text, r.local.clone())],
                vec![Span::new(Role::Text, r.remote.clone())],
                vec![Span::new(
                    if r.state == "ESTAB" {
                        Role::Ok
                    } else {
                        Role::TextMuted
                    },
                    r.state.clone(),
                )],
                vec![Span::new(
                    if r.pid.is_some() {
                        Role::Text
                    } else {
                        Role::TextMuted
                    },
                    match (r.pid, r.process.as_str()) {
                        (Some(pid), "") => format!("pid {pid}"),
                        (Some(pid), p) => format!("{p} ({pid})"),
                        // Not ours to read: the uid is what is knowable.
                        (None, _) => format!("uid {}", r.uid),
                    },
                )],
            ]
        })
        .collect();
    let body = body.max(1);
    let cursor = n.scroll().min(rows.len().saturating_sub(1));
    let top = cursor
        .saturating_sub(body.saturating_sub(1))
        .min(rows.len().saturating_sub(body.min(rows.len())));
    View::Table {
        columns: vec![
            Column {
                title: "proto".into(),
                width: ColWidth::Fixed(5),
                right: false,
            },
            Column {
                title: "local".into(),
                width: ColWidth::Elastic,
                right: false,
            },
            Column {
                title: "remote".into(),
                width: ColWidth::Elastic,
                right: false,
            },
            Column {
                title: "state".into(),
                width: ColWidth::Fixed(9),
                right: false,
            },
            Column {
                title: "process".into(),
                width: ColWidth::Fixed(18),
                right: false,
            },
        ],
        rows,
        selected: cx.captured.then_some(cursor),
        sort: None,
        scroll: top,
    }
}

/// The window the mirrored chart covers, at most — ten minutes, the same
/// window the gpu and sensors charts use. The window actually drawn is the
/// run's age capped at this (D62 amendment 1): with a fixed span a run younger
/// than the cap has no samples for the oldest buckets and the line starts
/// part-way across the rect.
const CHART_SPAN: Duration = Duration::from_secs(600);

/// At most this many interfaces are charted: the busiest, by peak over the
/// window. A workstation has nine interfaces and eight of them are flat.
const CHART_IFACES: usize = 4;

/// One interface's two lines, and the peak that ranks it.
struct Charted {
    peak: f64,
    name: String,
    rx: Vec<(f64, f64)>,
    tx: Vec<(f64, f64)>,
}

/// The mirrored rx/tx chart the arc-7 spec named and D62 amendment 5 deleted
/// rather than built (D65 §5): one line pair per shown interface, rx above a
/// zero line and tx below it, over the run's age capped at `CHART_SPAN`.
///
/// **The zero line is the renderer's midpoint gridline** — `bounds.y` is
/// symmetric about zero, so the rule the renderer draws at the middle of the
/// range is exactly the axis these lines are mirrored about. That is why 15a
/// had to land first.
///
/// Interfaces that carried nothing over the window are left out rather than
/// drawn flat: eight idle lines on the zero row is eight names stacked on one
/// cell, and a legend nobody can read is worse than a shorter one.
fn mirror_chart(n: &Net, cx: &RenderCx<'_>, width: u16) -> View {
    let buckets = usize::from(width.max(2));
    let span = Duration::from_nanos(cx.now.0)
        .min(CHART_SPAN)
        .max(Duration::from_secs(1));
    let mut buf: Vec<Option<f64>> = Vec::new();
    let mut per_iface: Vec<Charted> = Vec::new();
    for i in &n.model().ifaces {
        let name: std::sync::Arc<str> = std::sync::Arc::from(i.name.as_str());
        let mut peak = 0.0f64;
        let take = |key: &gridwatch_store::Key<f64>, sign: f64, buf: &mut Vec<Option<f64>>| {
            cx.store
                .resample(&key.named(&name), span, buckets, Agg::Max, buf);
            buf.iter()
                .enumerate()
                .filter_map(|(x, v)| v.map(|v| (x as f64, sign * v.max(0.0))))
                .collect::<Vec<_>>()
        };
        let rx = take(&net::RX_BPS, 1.0, &mut buf);
        let tx = take(&net::TX_BPS, -1.0, &mut buf);
        for (_, v) in rx.iter().chain(&tx) {
            peak = peak.max(v.abs());
        }
        if peak > 0.0 {
            per_iface.push(Charted {
                peak,
                name: i.name.clone(),
                rx,
                tx,
            });
        }
    }
    if per_iface.is_empty() {
        return View::Text(vec![vec![Span::new(
            Role::TextGhost,
            "rx/tx: nothing has moved in this window",
        )]]);
    }
    per_iface.sort_by(|a, b| b.peak.total_cmp(&a.peak));
    per_iface.truncate(CHART_IFACES);
    let m = per_iface.iter().fold(0.0f64, |a, p| a.max(p.peak));
    let mut series = Vec::with_capacity(per_iface.len() * 2);
    for c in per_iface {
        series.push(Series {
            label: Cow::Owned(format!("↓{}", c.name)),
            gradient: GradientId::NetRx,
            data: c.rx,
        });
        series.push(Series {
            label: Cow::Owned(format!("↑{}", c.name)),
            gradient: GradientId::NetTx,
            data: c.tx,
        });
    }
    View::Chart {
        series,
        bounds: Bounds {
            x: (0.0, (buckets.saturating_sub(1)).max(1) as f64),
            y: (-m, m),
        },
        marker: MarkerHint::Braille,
    }
}

/// The footer: what the filter hides, and how to see it.
fn footer(n: &Net) -> Line {
    let mut line: Line = vec![Span::new(
        Role::TextMuted,
        format!("sort {} · a all · s sort", n.sort().name()),
    )];
    let hidden = n.model().hidden();
    if hidden > 0 && !n.show_all() {
        line.push(Span::new(Role::TextGhost, format!("  ({hidden} hidden)")));
    }
    line
}

/// The shortest band worth charting: four rows, which is where the renderer
/// starts drawing the midpoint gridline (D65 §4) — and that line **is** this
/// chart's zero. A three-row mirrored chart is two lines with nothing to say
/// which side of zero they are on, so the tier draws the connections instead.
const CHART_MIN_ROWS: u16 = 4;

fn table_tier(n: &Net, cx: &RenderCx<'_>, with_conns: bool) -> View {
    let body = cx.inner.height;
    // **The interface band is the rows it has**, capped at two fifths of the
    // body — not `Fill` (§4.6, D65 §5). A workstation has three interfaces by
    // default and nine with `a`, and can never use sixteen rows; `Fill`
    // belongs to whichever child can use every cell it is given, which here
    // is the chart.
    let iface_rows = u16::try_from(n.model().ifaces.len() + 1).unwrap_or(u16::MAX);
    let iface_h = iface_rows.min((body * 2 / 5).max(2));
    let probe = probe_line(n);
    // The constraints first, then the split they produce, so **each table's
    // scroll viewport is the band it was actually given** rather than the
    // tile's inner height. The renderer computes the same split from the same
    // function, so the two cannot disagree.
    let build = |with_chart: bool| -> Vec<Constraint> {
        let mut cs: Vec<Constraint> = vec![Constraint::Len(iface_h)];
        if probe.is_some() {
            cs.push(Constraint::Len(1));
        }
        if with_chart {
            cs.push(Constraint::Fill(1));
        }
        if with_conns {
            cs.push(Constraint::Fill(3));
        }
        cs.push(Constraint::Len(1));
        cs
    };
    // Whether the chart fits is decided by the band it would actually get,
    // not by a threshold on the tile: at the `conns` tier it shares the
    // remainder with the connection table one part to three.
    let chart_at = usize::from(probe.is_some()) + 1;
    let cs = build(true);
    let with_chart = gridwatch_ui::layout::split(&cs, body)[chart_at] >= CHART_MIN_ROWS;
    let cs = if with_chart { cs } else { build(false) };
    let bands = gridwatch_ui::layout::split(&cs, body);
    let mut children: Vec<(Constraint, View)> = Vec::with_capacity(cs.len());
    let mut at = 0usize;
    let push = |v: View, children: &mut Vec<(Constraint, View)>, at: &mut usize| {
        children.push((cs[*at], v));
        *at += 1;
    };
    let iface_body = usize::from(bands[0].saturating_sub(1));
    push(iface_table(n, cx, iface_body), &mut children, &mut at);
    if let Some(p) = probe {
        push(View::Text(vec![p]), &mut children, &mut at);
    }
    if with_chart {
        push(mirror_chart(n, cx, cx.inner.width), &mut children, &mut at);
    }
    if with_conns {
        let conn_body = usize::from(bands[at].saturating_sub(1));
        push(conn_table(n, cx, conn_body), &mut children, &mut at);
    }
    push(View::Text(vec![footer(n)]), &mut children, &mut at);
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The route pane: interface, gateway, source address, DNS, public IP.
fn route_lines(n: &Net) -> Vec<Line> {
    let Some(r) = n.model().route.as_ref() else {
        return vec![vec![Span::new(Role::TextMuted, "route —")]];
    };
    let mut out = vec![vec![
        Span::new(Role::TextMuted, "route "),
        Span::bold(Role::Text, r.default_iface.clone()),
        Span::new(Role::TextMuted, " via "),
        Span::new(Role::Text, r.gateway.clone()),
    ]];
    if !r.prefsrc.is_empty() {
        out[0].push(Span::new(Role::TextMuted, " from "));
        out[0].push(Span::new(Role::Text, r.prefsrc.clone()));
    }
    out.push(vec![
        Span::new(Role::TextMuted, "dns   "),
        Span::new(
            Role::Text,
            if r.dns.is_empty() {
                "—".to_string()
            } else {
                r.dns.join(", ")
            },
        ),
    ]);
    out.push(match r.public_ip.as_ref() {
        Some(ip) => vec![
            Span::new(Role::TextMuted, "public "),
            Span::new(Role::Text, ip.clone()),
        ],
        None => vec![Span::new(
            Role::TextGhost,
            "public ip: off ([sources.net] public_ip = true asks the internet)",
        )],
    });
    out
}

/// Per-interface detail: mac, mtu, addresses, carrier flaps.
fn detail_lines(n: &Net) -> Vec<Line> {
    n.model()
        .ifaces
        .iter()
        .filter_map(|i| i.link.as_ref())
        .map(|l| {
            let mut line: Line = vec![
                Span::bold(Role::Text, l.iface.clone()),
                Span::new(Role::TextMuted, format!("  {} · ", l.kind.name())),
                Span::new(Role::Text, link_text(l)),
                Span::new(Role::TextMuted, format!("  mtu {}", l.mtu)),
            ];
            if !l.mac.is_empty() {
                line.push(Span::new(Role::TextMuted, format!("  {}", l.mac)));
            }
            if !l.addrs.is_empty() {
                line.push(Span::new(Role::Text, format!("  {}", l.addrs.join(" "))));
            }
            if l.carrier_changes > 0 {
                line.push(Span::new(
                    Role::TextMuted,
                    format!("  {} flaps", l.carrier_changes),
                ));
            }
            line
        })
        .collect()
}

fn full(n: &Net, cx: &RenderCx<'_>) -> View {
    let mut probe_rows: Vec<Line> = Vec::new();
    if let Some(p) = n.model().probes.as_ref() {
        for t in &p.targets {
            probe_rows.push(vec![
                Span::bold(Role::Text, format!("{:<10}", t.target)),
                Span::new(Role::TextMuted, format!("{} ", t.addr)),
                Span::new(
                    Role::Text,
                    format!(
                        "min {:.1} · avg {:.1} · max {:.1} · mdev {:.1} · jitter {:.1} ms",
                        t.min_ms, t.avg_ms, t.max_ms, t.mdev_ms, t.jitter_ms
                    ),
                ),
                Span::new(
                    if t.loss_pct > 0.0 {
                        Role::Warn
                    } else {
                        Role::TextMuted
                    },
                    format!("  {:.0}% loss of {}", t.loss_pct, t.sent),
                ),
            ]);
        }
    }
    let ifaces = u16::try_from(n.model().ifaces.len() + 1).unwrap_or(u16::MAX);
    let detail = detail_lines(n);
    // **Both** viewports come from the band each table was given (§4.6, D65
    // §5), and this is the tier where it matters most: `full` is the zoomed
    // connection browser, so its `Fill` band is most of the body while the
    // panes above it are fixed. A guessed fraction of the tile's height told
    // the table it had a third of the rows it does, and the scroll it derived
    // then left thirty blank rows under the last page.
    let cs = [
        Constraint::Len(ifaces.min(cx.inner.height)),
        Constraint::Len(3),
        Constraint::Len(u16::try_from(detail.len()).unwrap_or(0)),
        Constraint::Len(u16::try_from(probe_rows.len()).unwrap_or(0)),
        Constraint::Fill(1),
        Constraint::Len(1),
    ];
    let bands = gridwatch_ui::layout::split(&cs, cx.inner.height);
    View::Stack {
        dir: Dir::V,
        children: vec![
            (cs[0], iface_table(n, cx, n.model().ifaces.len())),
            (cs[1], View::Text(route_lines(n))),
            (cs[2], View::Text(detail)),
            (cs[3], View::Text(probe_rows)),
            (
                cs[4],
                conn_table(n, cx, usize::from(bands[4].saturating_sub(1))),
            ),
            (cs[5], View::Text(vec![footer(n)])),
        ],
    }
}
