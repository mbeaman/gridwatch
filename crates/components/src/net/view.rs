//! The net tiers as view trees (§8): `rates` (one rate pair with a link
//! dot), `sparks` (+ sparklines and the speed or SSID), `table` (every
//! shown interface with drops and errors, and the probe strip), `conns`
//! (+ the connection table) and the zoom-only `full` (+ the route, the
//! per-interface detail and the probe statistics).

use std::time::Duration;

use gridwatch_store::keys::net::{self, Link, LinkKind};
use gridwatch_store::{Agg, SourceState};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{ColWidth, Column, Constraint, Dir, Line, Span, View};

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

/// A sparkline of one key over the last minutes.
fn spark(cx: &RenderCx<'_>, key: &gridwatch_store::Key<f64>, iface: &str) -> View {
    let mut buf = Vec::new();
    let buckets = usize::from(cx.inner.width).max(2);
    cx.store.resample(
        &key.named(&std::sync::Arc::from(iface)),
        Duration::from_secs(120),
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

/// The connection table, scrolled by the cursor.
fn conn_table(n: &Net, cx: &RenderCx<'_>) -> View {
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
    let body = usize::from(cx.inner.height).saturating_sub(2).max(1);
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

fn table_tier(n: &Net, cx: &RenderCx<'_>, with_conns: bool) -> View {
    // The iface table takes two fifths of the body at `table`, a fifth
    // once the connection table is under it; minus the header row.
    let body = usize::from(cx.inner.height);
    let share = if with_conns { body / 5 } else { body * 2 / 5 };
    let mut children = vec![(
        Constraint::Fill(2),
        iface_table(n, cx, share.saturating_sub(1)),
    )];
    if let Some(p) = probe_line(n) {
        children.push((Constraint::Len(1), View::Text(vec![p])));
    }
    if with_conns {
        children.push((Constraint::Fill(3), conn_table(n, cx)));
    }
    children.push((Constraint::Len(1), View::Text(vec![footer(n)])));
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
    View::Stack {
        dir: Dir::V,
        children: vec![
            (
                Constraint::Len(ifaces.min(cx.inner.height)),
                iface_table(n, cx, n.model().ifaces.len()),
            ),
            (Constraint::Len(3), View::Text(route_lines(n))),
            (
                Constraint::Len(u16::try_from(detail.len()).unwrap_or(0)),
                View::Text(detail),
            ),
            (
                Constraint::Len(u16::try_from(probe_rows.len()).unwrap_or(0)),
                View::Text(probe_rows),
            ),
            (Constraint::Fill(1), conn_table(n, cx)),
            (Constraint::Len(1), View::Text(vec![footer(n)])),
        ],
    }
}
