//! The `alerts` tile (§8 free extras, brief arc 3 seam 5): the store's active
//! alerts worst-first and the event ring newest-first. Always built, like
//! `clock` and `sources`; `A` opens the same view as an overlay.

use std::borrow::Cow;

use gridwatch_store::{Detail, KeyCode, KeyEvent, Severity, Store, Transition, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, InputCx, KeyHint, Manifest,
    Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{Constraint, Dir, Line, Span, View};

pub static MANIFEST: Manifest = Manifest {
    kind: "alerts",
    name: "Alerts",
    summary: "active alerts worst-first and the event log",
    contract: 1,
    footprints: &[
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 1 },
        Footprint { w: 4, h: 2 },
    ],
    default_footprint: Footprint { w: 4, h: 1 },
    requires: &[],
    optional: &[],
    sources: &[],
    optional_sources: &[],
    chrome: Chrome::Themed,
    keys: &[KeyHint {
        key: "↑/↓ PgUp/PgDn Home",
        does: "scroll the log",
    }],
    example_options: "",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "list",
        min: Size::new(8, 3),
        adds: &["active alerts, worst first"],
        zoom_only: false,
    },
    Tier {
        name: "log",
        min: Size::new(40, 8),
        adds: &["the event ring, newest first"],
        zoom_only: false,
    },
];

pub const TIER_LIST: usize = 0;
pub const TIER_LOG: usize = 1;

#[derive(Default)]
pub struct Alerts {
    scroll: usize,
}

fn build(_cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Alerts::default()))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

fn role(s: Severity) -> Role {
    match s {
        Severity::Crit => Role::Crit,
        Severity::Warn => Role::Warn,
        Severity::Info => Role::Info,
    }
}

/// `title: detail` unless the detail already starts with the title (astral-watch's
/// `OVERLOAD pins 1+2 >9.2A` carries its label) — no `OVERLOAD: OVERLOAD …`.
pub fn headline(title: &str, detail: &str) -> String {
    if detail.starts_with(title) || title.is_empty() {
        detail.to_string()
    } else if detail.is_empty() {
        title.to_string()
    } else {
        format!("{title}: {detail}")
    }
}

fn age(now: Ts, since: Ts) -> String {
    let s = now.since(since).as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// The active list, worst first: `‼ OVERLOAD · pins/overload · 5m02s`.
pub fn active_lines(store: &Store, now: Ts, width: u16) -> Vec<Line> {
    let mut active: Vec<_> = store.alerts().active().collect();
    active.sort_by(|a, b| b.1.severity.cmp(&a.1.severity).then(a.0.cmp(b.0)));
    if active.is_empty() {
        return vec![vec![Span::new(Role::TextGhost, "no active alerts")]];
    }
    active
        .into_iter()
        .map(|(id, a)| {
            let mut line = vec![
                Span::bold(role(a.severity), a.title.to_string()),
                Span::new(Role::TextGhost, " · "),
                Span::new(Role::TextMuted, age(now, a.since)),
            ];
            if width >= 40 {
                line.push(Span::new(Role::TextGhost, " · "));
                line.push(Span::new(Role::TextMuted, id.0.to_string()));
            }
            if width >= 60 {
                line.push(Span::new(Role::TextGhost, " · "));
                line.push(Span::new(Role::Text, a.detail.to_string()));
            }
            line
        })
        .collect()
}

/// The event ring, newest first, `scroll` entries from the top.
pub fn log_lines(store: &Store, scroll: usize, rows: usize) -> Vec<Line> {
    let events: Vec<_> = store.alerts().events().rev().collect();
    if events.is_empty() {
        return vec![vec![Span::new(Role::TextGhost, "no alerts this session")]];
    }
    let n = events.len();
    let start = scroll.min(n.saturating_sub(1));
    // The hint line lives inside `rows` (review: it always fell off the end).
    let body = if n > rows.max(1) {
        rows.max(2) - 1
    } else {
        rows.max(1)
    };
    let end = (start + body).min(n);
    let mut out: Vec<Line> = events[start..end]
        .iter()
        .map(|e| {
            let r = match e.transition {
                Transition::Resolved => Role::Ok,
                _ => role(e.severity),
            };
            let what = match e.transition {
                Transition::Raised => "RAISED",
                Transition::Repeated => "ACTIVE",
                Transition::Resolved => "RESOLVED",
            };
            vec![
                Span::new(Role::TextGhost, format!("{:>7.1}s ", e.at.as_secs_f64())),
                Span::new(r, format!("{what} {}", headline(&e.title, &e.detail))),
                Span::new(Role::TextGhost, " · "),
                Span::new(Role::TextMuted, e.id.0.to_string()),
            ]
        })
        .collect();
    if start > 0 || end < n {
        out.push(vec![Span::new(
            Role::TextGhost,
            format!("({}–{} of {n}; ↑/↓)", start + 1, end),
        )]);
    }
    out
}

impl Component for Alerts {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, cx: &TickCx<'_>) -> Cow<'static, str> {
        let n = cx.store.alerts().active().count();
        if n == 0 {
            Cow::Borrowed("alerts")
        } else {
            Cow::Owned(format!("alerts ({n})"))
        }
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn tick(&mut self, _cx: &TickCx<'_>) -> Redraw {
        Redraw::No
    }

    fn on_key(&mut self, key: KeyEvent, _cx: &InputCx<'_>) -> Outcome {
        match key.code {
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::Home => self.scroll = 0,
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        let active = View::Text(active_lines(cx.store, cx.now, cx.inner.width));
        if cx.tier == TIER_LIST {
            return active;
        }
        let n_active = cx.store.alerts().active().count().max(1) as u16;
        let head = n_active.min(cx.inner.height / 2).max(1);
        View::Stack {
            dir: Dir::V,
            children: vec![
                (Constraint::Len(head), active),
                (
                    Constraint::Len(1),
                    View::Text(vec![vec![Span::bold(Role::TextMuted, "log")]]),
                ),
                (
                    Constraint::Fill(1),
                    View::Text(log_lines(
                        cx.store,
                        self.scroll,
                        usize::from(cx.inner.height.saturating_sub(head + 1)),
                    )),
                ),
            ],
        }
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_LIST => &[],
            _ => &["log"],
        }
    }
}
