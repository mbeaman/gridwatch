//! Testkit (feature `testkit`, §12): the shared seams every component test uses.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gridwatch_store::{Batch, Msg, Store, Ts, demo};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::component::{Component, Size, Tier, pick_tier};
use crate::theme::{ColorMode, GRADIENTS, ROLES, Theme, load_builtin};

/// A store fed by the seeded cpu and gpu synths at fixed 1.5 s ticks — the
/// same generators `--demo` uses, so snapshots and demo mode cannot drift
/// (§12.5). Fed at `Detail::Table`, so the process tables are there for the
/// table tiers.
pub fn demo_store(seed: u64, ticks: usize) -> Store {
    demo_store_at(seed, ticks, gridwatch_store::Detail::Table)
}

/// `demo_store` at an explicit demand detail (`Meters` = no process table).
pub fn demo_store_at(seed: u64, ticks: usize, detail: gridwatch_store::Detail) -> Store {
    let mut store = Store::default();
    let mut synth = demo::CpuSynth::new(seed);
    let mut gpu = demo::GpuSynth::new(seed);
    let mut pins = demo::PinsSynth::new(seed);
    let mut audio = demo::AudioSynth::new(seed);
    let mut sensors = demo::SensorsSynth::new(seed);
    let mut media = demo::MediaSynth::new(seed);
    for i in 0..ticks {
        let at = Ts((i as u64 + 1) * 1_500_000_000);
        let batch: Batch = synth.tick_at(at, detail);
        store.apply(&Msg::Batch(batch));
        // The gpu and pins synths on the same ticks (arcs 2b, 3a): one store,
        // every source, as `--demo` runs them. The pins synth's scripted
        // overload (20–40 s) and its alert events are part of the feed.
        let batch: Batch = gpu.tick_at(at, detail);
        store.apply(&Msg::Batch(batch));
        let tick = pins.tick_at(at);
        store.apply(&Msg::Batch(tick.batch));
        for a in tick.alerts {
            store.apply(&Msg::Control(gridwatch_store::ControlMsg::Alert(a)));
        }
        // The audio synth (arc 5a): silent for its first 1.5 s, then the song.
        store.apply(&Msg::Batch(audio.tick_at(at)));
        store.apply(&Msg::Batch(sensors.tick_at(at)));
        store.apply(&Msg::Batch(media.tick_at(at)));
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
    c: &mut dyn Component,
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
    tick(c, store, tier);
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

/// The shell's per-frame `tick` before `view` (§5): the table tiers derive
/// their rows here, so a test that skips it sees an empty table.
pub fn tick(c: &mut dyn Component, store: &Store, tier: usize) {
    let cx = crate::component::TickCx {
        store,
        now: store.latest(),
        visible: true,
        tier,
    };
    c.tick(&cx);
}

/// The view a component builds at a size — the input to the renderer and to
/// `view::fingerprint`, exposed so tests can measure or inspect it directly.
pub fn view_of(c: &mut dyn Component, store: &Store, th: &Theme, size: Size) -> crate::view::View {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, false, None);
    tick(c, store, tier);
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
    c: &mut dyn Component,
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
    tick(c, store, tier);
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

/// The plain characters of a buffer, row by row.
pub fn plain_text(buf: &Buffer) -> String {
    let area = *buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(c) = buf.cell((area.x + x, area.y + y)) {
                out.push_str(c.symbol());
            }
        }
        out.push('\n');
    }
    out
}

/// True when the text contains a number immediately followed by `%` — the
/// shape of a fabricated reading on an empty store (D46: `—` is honest, `0%`
/// is not).
pub fn has_fabricated_percent(text: &str) -> bool {
    let b = text.as_bytes();
    b.iter()
        .enumerate()
        .any(|(i, c)| *c == b'%' && i > 0 && b[i - 1].is_ascii_digit())
}

/// The D46 sweep: every inner size from 0×0 to the richest tier's min plus a
/// margin, plus the zoomed body, with `data` populated and `empty` not.
/// Asserts, per size: no panic (both stores); when the rect fits tier 0 the
/// buffer is non-blank on *both* stores (an honest empty tile says `—`), and
/// with data it carries the chosen tier's `signature`; on the empty store,
/// nothing that reads as a measured percentage. "Didn't panic" alone is never a pass (TESTING.md, layer A).
pub fn assert_renders_everywhere(
    mk: &dyn Fn() -> Box<dyn Component>,
    data: &Store,
    empty: &Store,
    th: &Theme,
) {
    let probe = mk();
    let tiers = probe.tiers();
    let min0 = tiers[0].min;
    let max = tiers.iter().map(|t| t.min).fold(Size::new(8, 3), |a, b| {
        Size::new(a.w.max(b.w), a.h.max(b.h))
    });
    drop(probe);
    let mut sizes: Vec<(Size, bool)> = Vec::new();
    for w in 0..=max.w + 4 {
        for h in 0..=max.h + 3 {
            sizes.push((Size::new(w, h), false));
        }
    }
    sizes.push((Size::new(248, 66), true));
    for (size, zoomed) in sizes {
        for (store, with_data) in [(data, true), (empty, false)] {
            let mut c = mk();
            let r = catch_unwind(AssertUnwindSafe(|| {
                render_component(c.as_mut(), store, th, size, zoomed)
            }));
            let Ok((tier, buf)) = r else {
                panic!(
                    "component panicked at {}x{} ({})",
                    size.w,
                    size.h,
                    if with_data { "data" } else { "empty store" }
                );
            };
            let text = plain_text(&buf);
            let blank = text.chars().all(char::is_whitespace);
            // Non-blank holds on the empty store too: an honest tile with no
            // data says `—` or "waiting", never nothing — the arc-1b blank
            // big-number tile was exactly this case.
            if min0.fits(size) {
                assert!(
                    !blank,
                    "blank frame at {}x{} on the {} store (tier {})",
                    size.w,
                    size.h,
                    if with_data { "data" } else { "empty" },
                    tiers[tier].name
                );
            }
            if with_data && min0.fits(size) {
                let c2 = mk();
                for sig in c2.signature(tier) {
                    assert!(
                        text.contains(sig),
                        "tier `{}` at {}x{} lacks its signature {sig:?}:\n{text}",
                        tiers[tier].name,
                        size.w,
                        size.h
                    );
                }
            }
            if !with_data {
                assert!(
                    !has_fabricated_percent(&text),
                    "fabricated percentage on an empty store at {}x{}:\n{text}",
                    size.w,
                    size.h
                );
            }
        }
    }
}

/// Kept for callers that only want the crash sweep; prefer
/// `assert_renders_everywhere` (D46).
pub fn assert_never_panics(mk: &dyn Fn() -> Box<dyn Component>, store: &Store, th: &Theme) {
    assert_renders_everywhere(mk, store, &Store::default(), th);
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
