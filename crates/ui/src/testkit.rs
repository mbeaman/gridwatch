//! Testkit (feature `testkit`, §12): the shared seams every component test uses.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gridwatch_store::{Batch, Msg, Store, Ts, demo};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::component::{Component, Size, Tier, pick_tier};
use crate::theme::{ColorMode, GRADIENTS, ROLES, Theme, load_builtin};
use crate::view::Renderer as _;

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
    let mut net = demo::NetSynth::new(seed);
    let mut disk = demo::DiskSynth::new(seed);
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
        store.apply(&Msg::Batch(net.tick_at(at, detail)));
        // The disk synth takes no `Detail` — the disk source never raises
        // one (D64 §8), and a synth that cannot vary by detail is the
        // cheapest proof of it.
        store.apply(&Msg::Batch(disk.tick_at(at)));
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
    render_component_view(c, store, th, size, zoomed, None)
}

/// `render_component` with the shell's `view = "<tier>"` preference (§4.6), so
/// a test can pin the tier a rect lands on instead of taking the richest that
/// fits. `pick_tier` ignores the preference when `zoomed`, so a caller that
/// depends on the tier must check the index it gets back.
pub fn render_component_view(
    c: &mut dyn Component,
    store: &Store,
    th: &Theme,
    size: Size,
    zoomed: bool,
    view: Option<&str>,
) -> (usize, Buffer) {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, zoomed, view);
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

/// One tier's entry in the growth sweep (D62 §4).
#[derive(Clone, Copy, Debug)]
pub struct Growth {
    /// Index into the component's `tiers()`.
    pub tier: usize,
    /// Doubling the width must draw at least 1.5× the cells.
    pub width: bool,
    /// Doubling the height must too.
    pub height: bool,
}

impl Growth {
    pub const fn new(tier: usize, width: bool, height: bool) -> Self {
        Self {
            tier,
            width,
            height,
        }
    }
}

/// The ratio a doubled axis must reach. A ratio rather than a factor of two
/// because the gaps between bars, a legend row and a header line are
/// legitimately constant (D62 §4).
const GROWTH_RATIO: f64 = 1.5;

/// The smallest extent `assert_every_drawing_grows` will double. Below eight
/// cells a drawing's own quantisation swamps the measurement: a bar at a tenth
/// of full scale is one cell tall in a three-row band and still one cell tall
/// in a six-row band, because a cell is the unit of ink. The rule is about the
/// room a drawing is given, and a three-row band is not room.
const AXIS_MIN: u16 = 8;

fn non_blank(buf: &Buffer) -> usize {
    let area = *buf.area();
    (0..area.width)
        .flat_map(|x| (0..area.height).map(move |y| (x, y)))
        .filter(|(x, y)| {
            buf.cell((area.x + *x, area.y + *y))
                .is_some_and(|c| !c.symbol().trim().is_empty())
        })
        .count()
}

/// D62 §4: a drawing inside a `Fill` band scales with its rect. For each
/// listed tier, render at the tier's `min`, then at double the width (and, if
/// listed, double the height) and assert the buffer carries at least
/// `GROWTH_RATIO` × the non-blank cells. The doubled rect is rendered with the
/// tier pinned as a `view` preference so a bigger rect cannot silently step up
/// a tier and measure something else; the tier index is asserted, because
/// `pick_tier` ignores the preference when zoomed.
pub fn assert_grows_with_area(
    mk: &dyn Fn() -> Box<dyn Component>,
    data: &Store,
    th: &Theme,
    growth: &[Growth],
) {
    let probe = mk();
    let tiers: Vec<(String, Size, bool)> = probe
        .tiers()
        .iter()
        .map(|t| (t.name.to_string(), t.min, t.zoom_only))
        .collect();
    drop(probe);
    for g in growth {
        let (name, min, zoom_only) = &tiers[g.tier];
        let zoomed = *zoom_only;
        let render = |size: Size, what: &str| -> usize {
            let mut c = mk();
            let (tier, buf) =
                render_component_view(c.as_mut(), data, th, size, zoomed, Some(name.as_str()));
            assert_eq!(
                tier, g.tier,
                "the {what} rect {}x{} landed on tier `{}`, not `{name}` — the growth sweep would \
                 measure the wrong tier",
                size.w, size.h, tiers[tier].0
            );
            non_blank(&buf)
        };
        let base = render(*min, "base");
        assert!(base > 0, "tier `{name}` draws nothing at its own minimum");
        for (axis, doubled) in [
            (g.width, Size::new(min.w * 2, min.h)),
            (g.height, Size::new(min.w, min.h * 2)),
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(i, (on, s))| on.then_some((if i == 0 { "width" } else { "height" }, s)))
        {
            let grown = render(doubled, axis);
            assert!(
                grown as f64 >= base as f64 * GROWTH_RATIO,
                "tier `{name}` does not fill a bigger rect: {base} cells at {}x{}, {grown} at \
                 {}x{} — double the {axis} must draw at least {GROWTH_RATIO}x (D62 §4, \
                 ARCHITECTURE §4.6)",
                min.w,
                min.h,
                doubled.w,
                doubled.h
            );
        }
    }
}

/// The view a component builds at a size with the tier pinned as a `view`
/// preference — `view_of` at an explicit tier, so a sweep can walk every
/// tier's tree instead of only the richest that fits.
fn view_at(
    c: &mut dyn Component,
    store: &Store,
    th: &Theme,
    size: Size,
    zoomed: bool,
    tier_name: &str,
) -> (usize, crate::view::View) {
    let inner = Rect {
        x: 0,
        y: 0,
        width: size.w,
        height: size.h,
    };
    let (tier, fallback) = pick_tier(c.tiers(), size, zoomed, Some(tier_name));
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
    (tier, c.view(&cx))
}

/// What can be asked of a drawing leaf, as a property of the **renderer**
/// rather than of any caller. D65 §10 asks for every exemption to carry a
/// reason; these are the reasons, and because they are the same for every
/// component they live here once instead of at seven call sites.
#[derive(Clone, Copy)]
struct Oracle {
    /// Its ink must span the rect it was given, edge to edge.
    reach: bool,
    /// Doubling its width must draw `GROWTH_RATIO` times the cells.
    double_w: bool,
    /// Doubling its height must.
    double_h: bool,
}

/// - `Sparkline` answers everything: one sample per column, held across empty
///   ones (D62 §1), and eighths up the rows. A series shorter than the rect
///   draws only the rightmost columns, which is the wide-terminal defect
///   itself, so the doubling test is the one that catches it.
/// - `Gauge`: span and width. Its bar is the rect minus its label and its
///   text, and the unfilled part is drawn in the empty glyph, so a wider rect
///   really is more ink. It draws on `area.y` alone, so there is no height.
/// - `Segmented`: span only. htop's meter draws its *unfilled* part as blank
///   space, so a wider rect adds nothing to a meter that is nearly empty —
///   `SWP` measures 16 cells at 30 columns and 16 at 60. What it must still do
///   is put its closing bracket at the right edge.
/// - `Chart`: reach only. A braille line's ink is one cell per column plus its
///   vertical excursion, and doubling the columns *halves* the excursion per
///   column — the audio scope measures 1153 → 1727 cells (1.50×) across a
///   doubled width, on the boundary by arithmetic rather than by design. The
///   height is worse and it is on purpose: a taller band buys y-resolution
///   rather than ink (D62 amendment 5, D65 §4), which is why the band now
///   carries gridlines instead of being asked for more ink. What a chart must
///   still do is reach the far edge, which is exactly the defect D62 found.
/// - `Bars`: nothing, and this is the one whole exemption. The renderer draws
///   `values.len()` bars whatever the width, so the count is the component's:
///   `audio`'s `mini` tier fixes it at ten by design (§8, "8–10 thin bars")
///   and its `spectrum` tier leaves a gap column per bar, so neither reach nor
///   a doubling means anything here. The count coming from the rect is what
///   `assert_grows_with_area` checks per tier, where the component's `view`
///   re-runs.
///
/// `None` for everything that is not a drawing: text leaves are question (ii)
/// of §4.6's three, `View::Custom` paints itself, and `View::Empty` is blank
/// on purpose.
fn drawing_oracle(v: &crate::view::View) -> Option<Oracle> {
    use crate::view::View as V;
    let o = |reach, double_w, double_h| {
        Some(Oracle {
            reach,
            double_w,
            double_h,
        })
    };
    match v {
        V::Sparkline { .. } => o(true, false, true),
        V::Gauge { .. } => o(true, true, false),
        V::Segmented { .. } => o(true, false, false),
        V::Chart { .. } => o(true, false, false),
        V::Bars { .. } => None,
        _ => None,
    }
}

/// The first and last columns of `buf` that carry ink.
fn lit_span(buf: &Buffer) -> Option<(u16, u16)> {
    let area = *buf.area();
    let lit = |x: u16| {
        (0..area.height).any(|y| {
            buf.cell((area.x + x, area.y + y))
                .is_some_and(|c| !c.symbol().trim().is_empty())
        })
    };
    let first = (0..area.width).find(|x| lit(*x))?;
    let last = (0..area.width).rev().find(|x| lit(*x))?;
    Some((first, last))
}

/// D65 §10, the per-drawing oracle: `assert_grows_with_area` counts a whole
/// tier's cells, so a capped drawing that is a small share of that tier's ink
/// hides inside it — it passed `pins` and `gpu` while both were still broken.
/// This walks `layout::leaves` for **every** tier at its own minimum and at
/// two and four times it, and for each drawing leaf whose rect grows with the
/// tile (no `Len` pinning that axis) renders the leaf alone at its rect and
/// at double, requiring `GROWTH_RATIO`.
///
/// The qualification is the whole point: unqualified it fails on `Len`
/// children and gets weakened until it means nothing, which is the road
/// `assert_grows_with_area` walked to five exclusions in one arc.
pub fn assert_every_drawing_grows(mk: &dyn Fn() -> Box<dyn Component>, data: &Store, th: &Theme) {
    let probe = mk();
    let tiers: Vec<(String, Size, bool)> = probe
        .tiers()
        .iter()
        .map(|t| (t.name.to_string(), t.min, t.zoom_only))
        .collect();
    drop(probe);
    for (name, min, zoom_only) in &tiers {
        for scale in [1u16, 2, 4] {
            let size = Size::new(min.w * scale, min.h * scale);
            let mut c = mk();
            let (tier, view) = view_at(c.as_mut(), data, th, size, *zoom_only, name.as_str());
            if tiers[tier].0 != *name {
                continue;
            }
            let inner = Rect {
                x: 0,
                y: 0,
                width: size.w,
                height: size.h,
            };
            for leaf in crate::layout::leaves(&view, inner) {
                let Some(oracle) = drawing_oracle(leaf.view) else {
                    continue;
                };
                let render = |w: u16, h: u16| -> Buffer {
                    let r = Rect {
                        x: 0,
                        y: 0,
                        width: w,
                        height: h,
                    };
                    let mut buf = Buffer::empty(r);
                    th.renderer().render(leaf.view, r, th, &mut buf);
                    buf
                };
                let draw = |w: u16, h: u16| -> usize { non_blank(&render(w, h)) };
                let (w, h) = (leaf.area.width, leaf.area.height);
                let base_buf = render(w, h);
                let base = non_blank(&base_buf);
                if base == 0 {
                    continue;
                }
                // Span: the wide-terminal defect itself — a drawing whose ink
                // stops part-way across a band it was given the whole of, at
                // either end. A sparkline with fewer samples than columns is
                // drawn right-anchored, so the *left* edge is the one that
                // catches it; a chart whose buckets stop short shows at the
                // right. Both are D62's report.
                if oracle.reach && leaf.fill_w && w >= AXIS_MIN {
                    let slack = (w / 16).max(2);
                    let (first, last) = lit_span(&base_buf).unwrap_or((w, 0));
                    assert!(
                        first <= slack && last + slack >= w,
                        "tier `{name}` at {}x{}: the {:?} leaf is {w} columns wide and its ink \
                         runs from column {first} to {last} — a drawing takes the rect's size \
                         (D62 §1, ARCHITECTURE §4.6)",
                        size.w,
                        size.h,
                        leaf.view
                    );
                }
                for (axis, on, dw, dh) in [
                    (
                        "width",
                        oracle.double_w && leaf.fill_w && w >= AXIS_MIN,
                        w.saturating_mul(2),
                        h,
                    ),
                    (
                        "height",
                        oracle.double_h && leaf.fill_h && h >= AXIS_MIN,
                        w,
                        h.saturating_mul(2),
                    ),
                ] {
                    if !on {
                        continue;
                    }
                    let grown = draw(dw, dh);
                    assert!(
                        grown as f64 >= base as f64 * GROWTH_RATIO,
                        "tier `{name}` at {}x{}: the {:?} leaf at {w}x{h} draws {base} cells and \
                         {grown} at {dw}x{dh} — a drawing in a band that grows with the tile \
                         must draw at least {GROWTH_RATIO}x when its {axis} doubles (D65 §10, \
                         ARCHITECTURE §4.6)",
                        size.w,
                        size.h,
                        leaf.view
                    );
                }
            }
        }
    }
}

/// D65 §9 question (ii): **every table ends where its content ends.** Run at
/// four times each tier's minimum, where a stretch is unmistakable, and it
/// would have caught the `sensors` table in arc 5b — at 250×70 its value sat
/// eighty-seven cells from the sensor it belonged to, at *every* size and not
/// only a wide one.
///
/// Two rules, because a column's width comes from two different places:
///
/// 1. **An elastic column is checked exactly.** It grows to the widest cell it
///    holds — measured over every row and never the visible page — and stops.
///    No allowance: a stretch is a failure. This is the renderer's cap (D65
///    §3) asserted from the outside, over the registry rather than over the
///    six tables that were known to stretch.
/// 2. **A fixed column is checked for the damage it does**, not against the
///    fixture. Every declared column must be drawn, and the row must fit the
///    rect. D65 §5 asked for "the natural width plus one pad per column" over
///    the whole table, and that formula does not survive contact with this
///    tree: a fixed width is a *declaration* sized for the widest value the
///    column can ever hold, and the demo store is never that case — `disk`'s
///    nine columns declare fifteen cells more than the fixture fills,
///    `winamp`'s `artist` ten, `net`'s `state` five (`no carrier` against a
///    fixture whose interfaces are all `up`). A rule tight enough to catch
///    `Fixed(999)` that way would be calibrated on the synths, which is the
///    fixture-shaped floor D65 §10's own trap 1 warns against. What a
///    `Fixed(999)` actually *does* is objective and fixture-independent: it
///    pushes the columns after it off the right edge, where the renderer skips
///    them — the same defect D57 amendment 19 fixed for elastic columns, when
///    the net table drew its remote address under a `local` header.
pub fn assert_tables_end_at_their_content(
    mk: &dyn Fn() -> Box<dyn Component>,
    data: &Store,
    th: &Theme,
) {
    let probe = mk();
    let tiers: Vec<(String, Size, bool)> = probe
        .tiers()
        .iter()
        .map(|t| (t.name.to_string(), t.min, t.zoom_only))
        .collect();
    drop(probe);
    for (name, min, zoom_only) in &tiers {
        let size = Size::new(min.w * 4, min.h * 4);
        let mut c = mk();
        let (tier, view) = view_at(c.as_mut(), data, th, size, *zoom_only, name.as_str());
        if tiers[tier].0 != *name {
            continue;
        }
        let inner = Rect {
            x: 0,
            y: 0,
            width: size.w,
            height: size.h,
        };
        for leaf in crate::layout::leaves(&view, inner) {
            let crate::view::View::Table { columns, rows, .. } = leaf.view else {
                continue;
            };
            if columns.is_empty() || rows.is_empty() {
                continue;
            }
            // Measured from a rendered buffer at two widths, never from the
            // renderer's own width helper: an assertion that compares
            // `table_widths` against `table_natural_widths` is comparing a
            // function with the helper it just called, and can only fail if
            // someone deletes the `min` (arc 15 review, F1). The external
            // property is the rule itself — **a table that ends at its
            // content does not move its ink right when given more room** —
            // so the same leaf drawn twice as wide must ink the same extent.
            let titles: Vec<&str> = columns.iter().map(|c| c.title.as_ref()).collect();
            let extent = |w: u16| -> u16 {
                let r = Rect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: leaf.area.height.max(1),
                };
                let mut buf = Buffer::empty(r);
                crate::renderer::DefaultRenderer.render(leaf.view, r, th, &mut buf);
                let mut last = 0;
                for y in r.y..r.y + r.height {
                    for x in r.x..r.x + r.width {
                        if let Some(c) = buf.cell((x, y))
                            && !c.symbol().trim().is_empty()
                        {
                            last = last.max(x + 1);
                        }
                    }
                }
                last
            };
            let narrow = extent(leaf.area.width);
            let wide = extent(leaf.area.width.saturating_mul(2).max(leaf.area.width));
            assert!(
                wide <= narrow,
                "tier `{name}` at {}x{}: a table inks {narrow} cells in a {}-wide rect and \
                 {wide} in one twice as wide — it is stretching rather than ending at its \
                 content (D65 §1, ARCHITECTURE §4.6). Columns {titles:?}",
                size.w,
                size.h,
                leaf.area.width
            );
            let widths = crate::renderer::table_widths(columns, rows, leaf.area.width);
            let total: u16 = widths.iter().sum::<u16>() + columns.len().saturating_sub(1) as u16;
            assert!(
                total <= leaf.area.width,
                "tier `{name}` at {}x{}: a table's columns need {total} cells in a {}-cell rect, \
                 so the last of them is drawn off the right edge and silently skipped — a column \
                 that is declared is a column a reader can see (D65 §1). Widths {widths:?} for \
                 {titles:?}",
                size.w,
                size.h,
                leaf.area.width
            );
            let natural = crate::renderer::table_natural_widths(columns, rows);
            for ((c, drawn), want) in columns.iter().zip(&widths).zip(&natural) {
                assert!(
                    *drawn > 0 || *want == 0,
                    "tier `{name}` at {}x{}: the `{}` column is drawn at zero width in a {}-cell \
                     rect while it holds {want} cells of content — something beside it took the \
                     room (D65 §1). Widths {widths:?} for {titles:?}",
                    size.w,
                    size.h,
                    c.title,
                    leaf.area.width
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
