//! The clock tile — the 60-line template every new component is copied from
//! (§8 free extras). Chrome::Borderless; time comes from `cx.wall` + the app's
//! timezone offset, so replay and snapshots are deterministic.

use std::borrow::Cow;
use std::time::UNIX_EPOCH;

use gridwatch_store::Detail;
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, Manifest, Redraw, RenderCx,
    Size, TickCx, Tier,
};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{Span, View};

pub static MANIFEST: Manifest = Manifest {
    kind: "clock",
    name: "Clock",
    summary: "wall-clock time",
    contract: 1,
    footprints: &[Footprint { w: 1, h: 1 }, Footprint { w: 2, h: 1 }],
    default_footprint: Footprint { w: 2, h: 1 },
    requires: &[],
    optional: &[],
    sources: &[],
    optional_sources: &[],
    chrome: Chrome::Borderless,
    keys: &[],
    example_options: "",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "mini",
        min: Size::new(8, 3),
        adds: &[],
        zoom_only: false,
    },
    Tier {
        name: "big",
        min: Size::new(26, 6),
        adds: &["big digits"],
        zoom_only: false,
    },
];

pub struct Clock;

fn build(_cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Clock))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

fn hh_mm(cx: &RenderCx<'_>) -> (u64, u64) {
    let secs = cx
        .wall
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let local = secs.saturating_add_signed(i64::from(cx.tz_offset_s));
    ((local / 3600) % 24, (local / 60) % 60)
}

impl Component for Clock {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("clock")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn tick(&mut self, _cx: &TickCx<'_>) -> Redraw {
        // The heartbeat redraw (1 Hz) carries the minute change; nothing to derive.
        Redraw::No
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        let (h, m) = hh_mm(cx);
        let text = format!("{h:02}:{m:02}");
        match cx.tier {
            0 => View::Text(vec![vec![Span::bold(Role::AccentSecondary, text)]]),
            _ => View::BigNumber {
                text: text.into(),
                role: Role::AccentSecondary,
            },
        }
    }
}
