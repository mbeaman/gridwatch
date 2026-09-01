//! Testkit (feature `testkit`, §12): the shared seams every component test uses.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gridwatch_store::{Batch, Msg, Store, Ts, demo};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::component::{Component, Size, Tier, pick_tier};
use crate::theme::{ColorMode, GRADIENTS, ROLES, Theme, load_builtin};

/// A store fed by the seeded synth at fixed 1.5 s ticks — the same generator
/// `--demo` uses, so snapshots and demo mode cannot drift (§12.5).
pub fn demo_store(seed: u64, ticks: usize) -> Store {
    let mut store = Store::default();
    let mut synth = demo::CpuSynth::new(seed);
    for i in 0..ticks {
        let at = Ts((i as u64 + 1) * 1_500_000_000);
        let batch: Batch = synth.tick(at);
        store.apply(&Msg::Batch(batch));
    }
    store
}

pub fn theme(name: &str) -> Theme {
    load_builtin(name, ColorMode::TrueColor).expect("built-in theme loads")
}

/// The real inner sizes the 12×6 grid produces (§6, measured): snapshot here,
/// not at round numbers.
pub fn real_grid_sizes() -> Vec<(&'static str, Size)> {
    vec![
        ("1x1_at_250x70", Size::new(17, 8)),
        ("2x1_at_250x70", Size::new(38, 8)),
        ("4x2_at_250x70", Size::new(80, 20)),
        ("6x3_at_250x70", Size::new(122, 31)),
        ("6x3_at_120x40_dense", Size::new(59, 18)),
        ("zoom_at_250x70", Size::new(248, 66)),
    ]
}

/// Render one component tier into a fresh buffer (view → default renderer).
pub fn render_component(
    c: &dyn Component,
    store: &Store,
    th: &Theme,
    size: Size,
    zoomed: bool,
) -> (usize, Buffer) {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, zoomed, None);
    let cx = crate::component::RenderCx {
        inner,
        tier,
        view_fallback: fallback,
        focused: false,
        captured: false,
        zoomed,
        dense: false,
        store,
        theme: th,
        now: store.latest(),
        wall: std::time::SystemTime::UNIX_EPOCH,
        tz_offset_s: 0,
        frame: 0,
    };
    let view = c.view(&cx);
    let mut buf = Buffer::empty(inner);
    th.renderer().render(&view, inner, th, &mut buf);
    (tier, buf)
}

/// The view a component builds at a size — the input to the renderer and to
/// `view::fingerprint`, exposed so tests can measure or inspect it directly.
pub fn view_of(c: &dyn Component, store: &Store, th: &Theme, size: Size) -> crate::view::View {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, false, None);
    let cx = crate::component::RenderCx {
        inner,
        tier,
        view_fallback: fallback,
        focused: false,
        captured: false,
        zoomed: false,
        dense: false,
        store,
        theme: th,
        now: store.latest(),
        wall: std::time::SystemTime::UNIX_EPOCH,
        tz_offset_s: 0,
        frame: 0,
    };
    c.view(&cx)
}

/// The semantic snapshot: tier name + view tree at a size.
pub fn view_snapshot(
    c: &dyn Component,
    store: &Store,
    th: &Theme,
    size: Size,
) -> serde_json::Value {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, false, None);
    let cx = crate::component::RenderCx {
        inner,
        tier,
        view_fallback: fallback,
        focused: false,
        captured: false,
        zoomed: false,
        dense: false,
        store,
        theme: th,
        now: store.latest(),
        wall: std::time::SystemTime::UNIX_EPOCH,
        tz_offset_s: 0,
        frame: 0,
    };
    serde_json::json!({
        "size": format!("{}x{}", size.w, size.h),
        "tier": c.tiers()[tier].name,
        "view": crate::dump::view_value(&c.view(&cx)),
    })
}

/// Sweep every inner size from 0×0 to the richest tier's min plus the zoomed
/// body; any panic fails the test (§12.2).
pub fn assert_never_panics(mk: &dyn Fn() -> Box<dyn Component>, store: &Store, th: &Theme) {
    let c = mk();
    let max = c
        .tiers()
        .iter()
        .map(|t| t.min)
        .fold(Size::new(8, 3), |a, b| {
            Size::new(a.w.max(b.w), a.h.max(b.h))
        });
    drop(c);
    let sweep_w = max.w + 4;
    let sweep_h = max.h + 3;
    for w in 0..=sweep_w {
        for h in 0..=sweep_h {
            let c = mk();
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = render_component(c.as_ref(), store, th, Size::new(w, h), false);
            }));
            assert!(r.is_ok(), "component panicked at {w}x{h}");
        }
    }
    // Zoomed body.
    let c = mk();
    let r = catch_unwind(AssertUnwindSafe(|| {
        let _ = render_component(c.as_ref(), store, th, Size::new(248, 66), true);
    }));
    assert!(r.is_ok(), "component panicked zoomed at 248x66");
}

/// `tiers()[0].min` must fit the grid's minimum unit (§4.6).
pub fn assert_min_tier_fits(tiers: &[Tier], min_unit_inner: Size) {
    let first = tiers.first().expect("at least one tier");
    assert!(
        first.min.fits(min_unit_inner),
        "tier 0 '{}' min {}x{} exceeds the grid minimum {}x{}",
        first.name,
        first.min.w,
        first.min.h,
        min_unit_inner.w,
        min_unit_inner.h
    );
}

/// Mins monotone non-decreasing (by area, and never shrinking on both axes),
/// zoom_only tiers form a suffix, at least one non-zoom tier (§12.2, D37).
pub fn assert_tiers_well_formed(tiers: &[Tier]) {
    assert!(!tiers.is_empty());
    assert!(
        tiers.iter().any(|t| !t.zoom_only),
        "every tier is zoom_only"
    );
    let mut seen_zoom = false;
    for pair in tiers.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            b.min.w >= a.min.w && b.min.h >= a.min.h,
            "tier '{}' min shrinks below '{}'",
            b.name,
            a.name
        );
    }
    for t in tiers {
        if t.zoom_only {
            seen_zoom = true;
        } else {
            assert!(
                !seen_zoom,
                "non-zoom tier '{}' after a zoom_only tier",
                t.name
            );
        }
    }
}

/// One line per role + eight stops per gradient: the per-theme swatch (§12.2).
pub fn role_swatch(th: &Theme) -> Vec<String> {
    let mut out = Vec::new();
    for r in ROLES {
        out.push(format!("{:?} {:?}", r, th.color(r)));
    }
    for g in GRADIENTS {
        let stops = th.gradient(g).stops8();
        out.push(format!("{:?} {:?}", g, stops));
    }
    out
}
