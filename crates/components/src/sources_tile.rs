//! The `sources` tile (§8 free extras): the debugging view of every source —
//! state, cadence demand, generation, age, drops, restarts.

use std::borrow::Cow;

use gridwatch_store::{Detail, SourceState};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, Manifest, Redraw, RenderCx,
    Size, TickCx, Tier,
};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{ColWidth, Column, Span, View};

pub static MANIFEST: Manifest = Manifest {
    kind: "sources",
    name: "Sources",
    summary: "source health: state, generation, age, drops, restarts",
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
    keys: &[],
    example_options: "",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "list",
        min: Size::new(8, 3),
        adds: &[],
        zoom_only: false,
    },
    Tier {
        name: "table",
        min: Size::new(40, 4),
        adds: &["columns"],
        zoom_only: false,
    },
];

pub struct SourcesTile;

fn build(_cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(SourcesTile))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

/// The NOTE column: the status reason, plus the cpu source's last pid-level
/// scan time when it is running (P15's evidence, §8 "the `/proc` scan ms the
/// performance gates read").
fn note(cx: &RenderCx<'_>, s: &gridwatch_store::SourceOverview<'_>) -> String {
    let mut out = s.status.reason.as_deref().unwrap_or("").to_string();
    // The last scan's cost, whichever tick it ran on: the meters publish more
    // often than the scan, so a "this tick only" guard blinked two ticks in
    // three (review finding).
    if s.id == gridwatch_store::keys::sys::SOURCE
        && let Some((_, ms)) = cx.store.last(&gridwatch_store::keys::sys::SCAN_MS)
    {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&format!("scan {ms:.1} ms"));
    }
    // The gpu source's NVML wall time per second per call class (P11's
    // evidence, D49): `nvml 0.1+2.3+2.0 ms/s`, fast + slow + procs.
    if s.id == gridwatch_store::keys::gpu::SOURCE {
        let class = |name: &str| {
            let key = gridwatch_store::keys::gpu::NVML_MS.named(&std::sync::Arc::from(name));
            cx.store.last(&key).map(|(_, v)| v)
        };
        if let (Some(fast), Some(slow), Some(procs)) =
            (class("fast"), class("slow"), class("procs"))
        {
            if !out.is_empty() {
                out.push_str(" · ");
            }
            out.push_str(&format!(
                "nvml {fast:.1}+{slow:.1}+{procs:.1} = {:.1} ms/s",
                fast + slow + procs
            ));
        }
        if let Some((_, info)) = cx.store.record(&gridwatch_store::keys::gpu::INFO.idx(0))
            && info.spec_mismatch
        {
            out.push_str(" · spec row disagrees with NVML");
        }
    }
    // The pins source's read cost and transactions per second (P14's
    // evidence): one block read is one transaction, at the sample cadence.
    if s.id == gridwatch_store::keys::pins::SOURCE
        && let Some((_, ms)) = cx.store.last(&gridwatch_store::keys::pins::READ_MS)
    {
        let tx_per_s = cx
            .store
            .record(&gridwatch_store::keys::pins::INFO)
            .map(|(_, i)| 1000.0 / f64::from(i.interval_ms.max(1)))
            .unwrap_or(2.0);
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&format!("read {ms:.1} ms · ≤{tx_per_s:.0} tx/s"));
    }
    out
}

fn state_role(s: SourceState) -> Role {
    match s {
        SourceState::Ok => Role::Ok,
        SourceState::Starting => Role::Info,
        SourceState::Degraded => Role::Warn,
        SourceState::Unavailable | SourceState::Stopped => Role::Crit,
    }
}

fn state_name(s: SourceState) -> &'static str {
    match s {
        SourceState::Starting => "starting",
        SourceState::Ok => "ok",
        SourceState::Degraded => "degraded",
        SourceState::Unavailable => "unavailable",
        SourceState::Stopped => "stopped",
    }
}

impl Component for SourcesTile {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("sources")
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

    fn view(&self, cx: &RenderCx<'_>) -> View {
        let rows: Vec<_> = cx.store.sources().collect();
        if rows.is_empty() {
            return View::Text(vec![vec![Span::new(Role::TextGhost, "no sources running")]]);
        }
        if cx.tier == 0 {
            // Compact: one line per source.
            let lines = rows
                .iter()
                .map(|s| {
                    vec![
                        Span::new(Role::Text, s.id.0.to_string()),
                        Span::new(Role::TextGhost, " "),
                        Span::new(state_role(s.status.state), state_name(s.status.state)),
                    ]
                })
                .collect();
            return View::Text(lines);
        }
        let columns = vec![
            Column {
                title: "SRC".into(),
                width: ColWidth::Fixed(8),
                right: false,
            },
            Column {
                title: "STATE".into(),
                width: ColWidth::Fixed(11),
                right: false,
            },
            Column {
                title: "GEN".into(),
                width: ColWidth::Fixed(6),
                right: true,
            },
            Column {
                title: "AGE".into(),
                width: ColWidth::Fixed(6),
                right: true,
            },
            Column {
                title: "DROP".into(),
                width: ColWidth::Fixed(5),
                right: true,
            },
            Column {
                title: "RST".into(),
                width: ColWidth::Fixed(3),
                right: true,
            },
            Column {
                title: "NOTE".into(),
                width: ColWidth::Elastic,
                right: false,
            },
        ];
        let body = rows
            .iter()
            .map(|s| {
                let age = s
                    .last_sample
                    .map(|t| format!("{:.0}s", cx.now.since(t).as_secs_f64()))
                    .unwrap_or_else(|| "—".into());
                vec![
                    vec![Span::new(Role::Text, s.id.0.to_string())],
                    vec![Span::new(
                        state_role(s.status.state),
                        state_name(s.status.state),
                    )],
                    vec![Span::new(Role::Text, s.generation.to_string())],
                    vec![Span::new(Role::TextMuted, age)],
                    vec![Span::new(Role::TextMuted, s.status.dropped.to_string())],
                    vec![Span::new(Role::TextMuted, s.status.restarts.to_string())],
                    vec![Span::new(Role::TextGhost, note(cx, s))],
                ]
            })
            .collect();
        View::Table {
            columns,
            rows: body,
            selected: None,
            sort: None,
            scroll: 0,
        }
    }
}
