//! Component gate tests (§12): snapshot matrix, no-panic sweep, tier hygiene.

use gridwatch_components::htop::{Htop, OPTION_NAMES, Options};
use gridwatch_ui::component::{Component, Size, pick_tier};
use gridwatch_ui::testkit::{
    Growth, assert_every_drawing_grows, assert_grows_with_area, assert_min_tier_fits,
    assert_renders_everywhere, assert_tables_end_at_their_content, assert_tiers_well_formed,
    demo_store, real_grid_sizes, render_component, theme, view_of, view_snapshot,
};

fn clock() -> Box<dyn Component> {
    Box::new(gridwatch_components::clock::Clock)
}

fn htop() -> Box<dyn Component> {
    Box::new(Htop::default())
}

fn sources() -> Box<dyn Component> {
    Box::new(gridwatch_components::sources_tile::SourcesTile)
}

fn gpu() -> Box<dyn Component> {
    Box::new(gridwatch_components::gpu::Gpu::default())
}

fn pins() -> Box<dyn Component> {
    Box::new(gridwatch_components::pins::Pins::default())
}

fn alerts() -> Box<dyn Component> {
    Box::new(gridwatch_components::alerts::Alerts::default())
}

fn audio() -> Box<dyn Component> {
    Box::new(gridwatch_components::audio::Audio::default())
}

fn sensors() -> Box<dyn Component> {
    Box::new(gridwatch_components::sensors::Sensors::default())
}

fn winamp() -> Box<dyn Component> {
    Box::new(gridwatch_components::winamp::Winamp::default())
}

fn net() -> Box<dyn Component> {
    Box::new(gridwatch_components::net::Net::default())
}

fn disk() -> Box<dyn Component> {
    Box::new(gridwatch_components::disk::Disk::default())
}

#[test]
fn tiers_are_well_formed() {
    for mk in [
        clock, sources, htop, gpu, pins, alerts, audio, sensors, winamp, net, disk,
    ] {
        let c = mk();
        assert_tiers_well_formed(c.tiers());
        assert_min_tier_fits(c.tiers(), Size::new(8, 3));
    }
}

/// D46 layer A: no panic, non-blank where the rect fits, the tier's signature
/// present, and nothing fabricated on an empty store — at every size.
#[test]
fn renders_everywhere() {
    let store = demo_store(42, 40);
    let empty = gridwatch_store::Store::default();
    for th in ["modern", "retrowave", "mono"].map(theme) {
        assert_renders_everywhere(&|| clock(), &store, &empty, &th);
        assert_renders_everywhere(&|| sources(), &store, &empty, &th);
        assert_renders_everywhere(&|| htop(), &store, &empty, &th);
        assert_renders_everywhere(&|| gpu(), &store, &empty, &th);
        assert_renders_everywhere(&|| pins(), &store, &empty, &th);
        assert_renders_everywhere(&|| alerts(), &store, &empty, &th);
        assert_renders_everywhere(&|| audio(), &store, &empty, &th);
        assert_renders_everywhere(&|| sensors(), &store, &empty, &th);
        assert_renders_everywhere(&|| winamp(), &store, &empty, &th);
        assert_renders_everywhere(&|| disk(), &store, &empty, &th);
    }
}

#[test]
fn view_snapshots_at_real_grid_sizes() {
    let store = demo_store(42, 3);
    let history = demo_store(42, 40);
    let th = theme("modern");
    for (name, size) in real_grid_sizes() {
        let mut c = clock();
        insta::assert_yaml_snapshot!(
            format!("clock_{name}"),
            view_snapshot(c.as_mut(), &store, &th, size)
        );
        let mut s = sources();
        insta::assert_yaml_snapshot!(
            format!("sources_{name}"),
            view_snapshot(s.as_mut(), &store, &th, size)
        );
        // A minute of history, so the snapshot pins a real sparkline rather
        // than three samples in one bucket.
        let mut h = htop();
        insta::assert_yaml_snapshot!(
            format!("htop_{name}"),
            view_snapshot(h.as_mut(), &history, &th, size)
        );
        let mut g = gpu();
        insta::assert_yaml_snapshot!(
            format!("gpu_{name}"),
            view_snapshot(g.as_mut(), &history, &th, size)
        );
        // Forty ticks reach 60 s: the scripted overload has raised (21.5 s)
        // and resolved (50 s), so the pins and alerts snapshots pin both log lines.
        let mut p = pins();
        insta::assert_yaml_snapshot!(
            format!("pins_{name}"),
            view_snapshot(p.as_mut(), &history, &th, size)
        );
        let mut a = alerts();
        insta::assert_yaml_snapshot!(
            format!("alerts_{name}"),
            view_snapshot(a.as_mut(), &history, &th, size)
        );
        // Three ticks reach 4.5 s: past the synth's 1.5 s of silence, so the
        // bars, the scope and the levels are lit.
        let mut au = audio();
        insta::assert_yaml_snapshot!(
            format!("audio_{name}"),
            view_snapshot(au.as_mut(), &store, &th, size)
        );
        let mut se = sensors();
        insta::assert_yaml_snapshot!(
            format!("sensors_{name}"),
            view_snapshot(se.as_mut(), &history, &th, size)
        );
        let mut wa = winamp();
        insta::assert_yaml_snapshot!(
            format!("winamp_{name}"),
            view_snapshot(wa.as_mut(), &history, &th, size)
        );
        let mut di = disk();
        insta::assert_yaml_snapshot!(
            format!("disk_{name}"),
            view_snapshot(di.as_mut(), &history, &th, size)
        );
    }
}

/// D62 §4 (ARCHITECTURE §4.6): a drawing inside a `Fill` band scales with its
/// rect. Doubling an axis must draw at least 1.5x the non-blank cells — a
/// ratio and not a factor of two, because a tier's header lines, legend and
/// gaps are legitimately constant. This is the instrument the wide-terminal
/// bug needed: `assert_renders_everywhere` sweeps only to `max(tier min) + 4`
/// and never reaches 480x135.
///
/// Measured on `demo_store(42, 40)` at the reference theme, 2026-09-06, and
/// re-measured after the arc's review (the htop header's MEM sparkline, the
/// stretched power trace and the pins rows that grow moved four numbers). The
/// axes listed below are asserted; the axes measured and *not* asserted are
/// recorded here with their numbers, because in each case the drawing does
/// take its size from the rect and the cell count is the wrong oracle for it:
///
/// - `gpu` `charts` height 448 -> 507 (1.13x). The band grows 4 -> 16 rows,
///   but a braille *line* mark lights about one cell per column per series
///   however tall the band is: a taller band buys y-resolution, not cells.
///   The zoom-only `full` tier has the same shape for the same reason
///   (1259 -> 1410 at 100x24 -> 100x48: five demo processes fill five rows of
///   a 35-row table); its zoomed band rule is pinned by the row-budget test
///   in `gpu.rs` instead.
/// - `audio` `spectrum` width 96 -> 132 (1.38x). The bar count does come from
///   the rect (13 groups at 40 cells, 27 at 80), but the demo's quiet bands
///   contribute only their one-row `▔` peak cap each.
/// - `sensors` `chart` width 378 -> 561 (1.48x). The chart's cells do double;
///   the tier's other half at its 60x14 minimum is a fixed-width text table.
/// - `winamp` `main+art` height 338 -> 394 (1.17x): a skin of `Len` rows.
///   (`pins` `trend` height was 1.08x for the same reason until the review
///   made its bar band and sparkline grow; it is 1.53x and asserted now.)
/// - `net` `table` 108 -> 119 (1.10x) wide, 108 -> 108 tall: the tier holds a
///   text table and a footer, no `Fill` drawing at all.
///
/// Arc 14 measured the `disk` tile's five tiers the same way (2026-09-09);
/// three axes are asserted below and four are recorded here:
///
/// - `disk` `rates` 16 -> 23 (1.44x) wide, 16 -> 16 (1.00x) tall. The tier is
///   three `Len(1)` text lines inside an 8x3 minimum — a rate pair and a busy
///   chip, with the drive's name added from 14 columns. There is no `Fill`
///   drawing to scale, and a taller 8-wide tile has nothing more to say.
/// - `disk` `table` height 95 -> 95 (1.00x): the row count is the number of
///   drives, not the number of rows that fit. (Its width is 1.87x and is
///   asserted: the column-drop order really does put content in a wider
///   rect.)
/// - `disk` `chart` height 318 -> 351 (1.10x), for `gpu` `charts`' reason: a
///   braille *line* mark lights about one cell per column per series however
///   tall the band is, so height buys y-resolution rather than cells.
/// - `disk` `full` 1003 -> 1212 (1.21x) wide, -> 1274 (1.27x) tall. The
///   zoom-only tier is fixed-width text panes over a chart with the same line
///   shape; at 100x24 the panes already fill it and a taller rect draws the
///   drives that were below the fold, which is a handful of rows and not a
///   proportional gain.
#[test]
fn drawings_grow_with_the_rect() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    // Tier indices, poorest first, from each component's `TIERS`.
    let (htop_cores, gpu_charts, pins_trend) = (3usize, 3usize, 3usize);
    let (net_sparks, audio_spectrum, sensors_chart) = (1usize, 3usize, 3usize);
    assert_grows_with_area(
        &|| htop(),
        &store,
        &th,
        &[Growth::new(htop_cores, true, true)],
    );
    assert_grows_with_area(
        &|| gpu(),
        &store,
        &th,
        &[Growth::new(gpu_charts, true, false)],
    );
    assert_grows_with_area(
        &|| pins(),
        &store,
        &th,
        &[Growth::new(pins_trend, true, true)],
    );
    // The `table` tier asserted **neither** axis before arc 15, because it had
    // no drawing at all: an interface table, a probe strip and a footer. It
    // now carries the mirrored rx/tx chart (D65 §5) and both axes pass —
    // width 1.50x (203 -> 305 cells) and height 1.54x (-> 312). Both are close
    // to the bar and that is honest rather than lucky: the tier is mostly
    // content-sized text plus one chart, so only the chart grows, and D62 §4
    // set the ratio at 1.5 precisely because a header line and a footer are
    // legitimately constant. The brief predicted the *height* axis would fail
    // as `gpu` `charts` does; it does not, because this band absorbs every row
    // the constant text leaves.
    let net_table = 2usize;
    assert_grows_with_area(
        &|| net(),
        &store,
        &th,
        &[
            Growth::new(net_sparks, true, true),
            Growth::new(net_table, true, true),
        ],
    );
    assert_grows_with_area(
        &|| audio(),
        &store,
        &th,
        &[Growth::new(audio_spectrum, false, true)],
    );
    assert_grows_with_area(
        &|| sensors(),
        &store,
        &th,
        &[Growth::new(sensors_chart, false, true)],
    );
    // The disk tile's three tiers with a drawing that takes its size from
    // the rect: the sparklines both ways, the table's columns and the
    // chart's buckets by width. The other four axes are in the comment
    // above with their measured numbers.
    let (disk_sparks, disk_table, disk_chart) = (1usize, 2usize, 3usize);
    assert_grows_with_area(
        &|| disk(),
        &store,
        &th,
        &[
            Growth::new(disk_sparks, true, true),
            Growth::new(disk_table, true, false),
            Growth::new(disk_chart, true, false),
        ],
    );
}

/// Every component the registry holds, built the way the app builds one, so a
/// new tile joins the two sweeps below by being registered rather than by
/// being remembered (D65 §9: the instruments run over the registry, not over
/// a hand-picked list).
type MakeComponent = Box<dyn Fn() -> Box<dyn Component>>;

fn every_registered_component() -> Vec<(&'static str, MakeComponent)> {
    let mut reg = gridwatch_ui::Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let kinds: Vec<&'static str> = reg.components().map(|d| d.manifest.kind).collect();
    kinds
        .into_iter()
        .map(|kind| {
            let mk: MakeComponent = Box::new(move || {
                let mut reg = gridwatch_ui::Registry::default();
                gridwatch_components::builtin_components(&mut reg);
                let def = reg.component(kind).expect("a registered kind");
                let options = toml::Table::new();
                let caps: gridwatch_store::CapSet =
                    gridwatch_store::ALL_CAPABILITIES.iter().copied().collect();
                let mut cx = gridwatch_ui::BuildCx {
                    options: &options,
                    caps: &caps,
                    instance: "test",
                };
                (def.build)(&mut cx).expect("a built-in builds with default options")
            });
            (kind, mk)
        })
        .collect()
}

/// D65 §10, question (i) at the resolution `assert_grows_with_area` cannot
/// reach: **per leaf**, not per tier. A capped drawing that is a small share
/// of a tier's ink hides inside the tier-wide count — that is how D62 passed
/// `pins` and `gpu` while both were still broken.
///
/// The exemptions are named once, in `testkit::drawing_oracle`, because they
/// are properties of the *renderer* and identical for every component: a
/// `Bars` leaf draws `values.len()` bars whatever the width, a `Chart`'s line
/// mark lights about one cell per column however tall the band, and a `Gauge`
/// or `Segmented` draws on one row. `Len`-pinned axes, `View::Custom` and
/// text leaves are exempt by rule.
#[test]
fn every_drawing_grows_with_its_own_rect() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    for (kind, mk) in every_registered_component() {
        println!("assert_every_drawing_grows: {kind}");
        assert_every_drawing_grows(&*mk, &store, &th);
    }
}

/// D65 §9, question (ii): **every table ends where its content ends** — a
/// defect at every size, not a wide-terminal one. It would have caught the
/// `sensors` table at 250×70 in arc 5b, where the value column sat 87 cells
/// from the sensor it belonged to.
#[test]
fn every_table_ends_at_its_content() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    for (kind, mk) in every_registered_component() {
        println!("assert_tables_end_at_their_content: {kind}");
        assert_tables_end_at_their_content(&*mk, &store, &th);
    }
}

/// The numbers the comment on `drawings_grow_with_the_rect` records, printed
/// for whoever adds the next tier or argues with an exclusion.
/// `cargo test -p gridwatch-components --test components -- --ignored growth_ratios`
#[test]
#[ignore = "diagnostic; prints the growth ratios the sweep is calibrated against"]
fn growth_ratios() {
    use gridwatch_ui::testkit::render_component_view;
    let store = demo_store(42, 40);
    let th = theme("modern");
    let nb = |buf: &ratatui_core::buffer::Buffer| -> usize {
        buf.content()
            .iter()
            .filter(|c| !c.symbol().trim().is_empty())
            .count()
    };
    type Mk = fn() -> Box<dyn Component>;
    let cands: [(&str, Mk, usize); 13] = [
        ("htop", htop, 3),
        ("gpu", gpu, 3),
        ("pins", pins, 3),
        ("net", net, 1),
        ("net", net, 2),
        ("audio", audio, 3),
        ("sensors", sensors, 3),
        ("winamp", winamp, 3),
        ("disk", disk, 0),
        ("disk", disk, 1),
        ("disk", disk, 2),
        ("disk", disk, 3),
        ("disk", disk, 4),
    ];
    for (name, mk, ti) in cands {
        let t = mk().tiers()[ti];
        let (min, z) = (t.min, t.zoom_only);
        let one = |sz: Size| {
            let (_, buf) = render_component_view(mk().as_mut(), &store, &th, sz, z, Some(t.name));
            nb(&buf)
        };
        let base = one(min);
        let (w, h) = (
            one(Size::new(min.w * 2, min.h)),
            one(Size::new(min.w, min.h * 2)),
        );
        println!(
            "{name}/{} min {}x{}: base {base} · width {w} ({:.2}x) · height {h} ({:.2}x)",
            t.name,
            min.w,
            min.h,
            w as f64 / base as f64,
            h as f64 / base as f64
        );
    }
}

/// The tier the real grid hands each rect (§6 measured sizes, brief 1b task 5).
#[test]
fn htop_tiers_match_the_real_grid_sizes() {
    let c = htop();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    // §8.1: both table tiers have min 56×18, so the table appears in any 6x3
    // whose inner height reaches 18 — 250×70, 120×40 dense, and the 4x2.
    assert_eq!(tier(122, 31, false), ("table", false), "6x3 at 250x70");
    assert_eq!(tier(59, 18, false), ("table", false), "6x3 dense at 120x40");
    assert_eq!(tier(80, 20, false), ("table", false), "4x2 at 250x70");
    assert_eq!(
        tier(56, 17, false),
        ("cores", false),
        "one row short of the table"
    );
    assert_eq!(tier(39, 11, false), ("meters", false), "4x2 dense");
    assert_eq!(tier(38, 8, false), ("meters", false), "2x1 at 250x70");
    assert_eq!(tier(17, 8, false), ("big-number", false), "1x1 at 250x70");
    assert_eq!(tier(9, 5, false), ("tiny", false), "1x1 dense at 120x40");
    // Arc 8a: zooming gives htop its whole face, not a wider dashboard.
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
    assert_eq!(tier(122, 31, false), ("table", false), "a 6x3 tile is not");
    // A pinned view that does not fit falls back and raises the chip (§4.6).
    let (i, fallback) = pick_tier(c.tiers(), Size::new(17, 8), false, Some("cores"));
    assert_eq!((c.tiers()[i].name, fallback), ("big-number", true));
    // `view = "table"` resolves now (the arc-1b warning path goes quiet).
    let (i, fallback) = pick_tier(c.tiers(), Size::new(122, 31), false, Some("table"));
    assert_eq!((c.tiers()[i].name, fallback), ("table", false));
    // `view = "cores"` pins the tier below it in a rect that could hold the table.
    let (i, fallback) = pick_tier(c.tiers(), Size::new(122, 31), false, Some("cores"));
    assert_eq!((c.tiers()[i].name, fallback), ("cores", false));
    // An unknown view name is ignored, not fatal.
    let (i, fallback) = pick_tier(c.tiers(), Size::new(122, 31), false, Some("nonsense"));
    assert_eq!((c.tiers()[i].name, fallback), ("table", false));
}

/// `OPTION_NAMES` is the list §9's disjointness rule is checked against (in
/// `crates/app/tests/shell.rs`, which may depend on both crates); here we only
/// assert it has not drifted from the struct it claims to describe.
#[test]
fn option_names_match_the_options_struct() {
    let table = toml::Table::try_from(Options::default()).expect("options serialise");
    let fields: Vec<&str> = table.keys().map(String::as_str).collect();
    let mut listed = OPTION_NAMES.to_vec();
    listed.sort_unstable();
    assert_eq!(fields, listed, "OPTION_NAMES has drifted from Options");
}

/// Options go through the real `build`, which is where validation lives — a
/// test that parses the struct directly would miss every rule in `validate`.
#[test]
fn options_reject_typos_and_the_table_floor_is_five() {
    let build = |text: &str| -> Result<Htop, String> {
        let options: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        Htop::from_table(&options).map_err(|e| e.0)
    };
    assert!(build("").is_ok(), "the defaults build");
    assert!(
        build("hide_kernel_thread = true").is_err(),
        "a mistyped option must not be swallowed"
    );
    assert!(
        build("refresh_ms = 500").is_err(),
        "source options belong in [sources.cpu]"
    );
    assert!(
        build("sort = \"nonsense\"").is_err(),
        "an unknown sort key must not build"
    );
    assert!(
        build("columns = [\"pid\", \"nonsense\"]").is_err(),
        "an unknown column must not build"
    );
    // htop never shows fewer than five table rows (§8): the floor is applied by
    // `validate`, so it can only be observed through `build`.
    let o = build("table_rows = 2").expect("builds").options().clone();
    assert_eq!(o.table_rows, 5, "table_rows floors at 5");
    assert_eq!(o.sort, "cpu");
}

#[test]
fn rendered_cells_snapshot_modern_only() {
    // Styled dumps at the reference theme only (§12.2): one per component at
    // one representative size; themes are covered by the role swatches.
    let store = demo_store(42, 3);
    let th = theme("modern");
    let (_, buf) = render_component(clock().as_mut(), &store, &th, Size::new(38, 8), false);
    insta::assert_snapshot!("clock_cells_2x1", gridwatch_ui::dump::cells(&buf));
    let (_, buf) = render_component(sources().as_mut(), &store, &th, Size::new(80, 20), false);
    insta::assert_snapshot!("sources_cells_4x2", gridwatch_ui::dump::cells(&buf));
    // The hero: the tier the screenshot is of, and the dense 6x3 beside it.
    let history = demo_store(42, 40);
    let (_, buf) = render_component(htop().as_mut(), &history, &th, Size::new(122, 31), false);
    insta::assert_snapshot!("htop_cells_6x3", gridwatch_ui::dump::cells(&buf));
    let (_, buf) = render_component(htop().as_mut(), &history, &th, Size::new(59, 18), false);
    insta::assert_snapshot!("htop_cells_6x3_dense", gridwatch_ui::dump::cells(&buf));
}

/// §13 caps `view` construction at 0.3 ms per visible tile, and the render
/// cache hashes the whole tree once per visible tile per frame
/// (`ui::view::fingerprint`). `cores` at 122×31 is the first tree big enough to
/// argue with that, so measure it rather than assume.
/// `cargo test -p gridwatch-components --release -- --ignored view_cost`
#[test]
#[ignore = "timing; run in release on the target machine"]
fn view_cost_at_the_hero_size_stays_inside_the_budget() {
    use std::time::Instant;
    let store = demo_store(42, 120);
    let th = theme("modern");
    let mut c = htop();
    let size = Size::new(122, 31);
    // Warm up, then measure view + fingerprint together — the pair is what a
    // frame pays for a tile whose data moved.
    for _ in 0..50 {
        let _ = render_component(c.as_mut(), &store, &th, size, false);
    }
    let n = 500u32;
    let t = Instant::now();
    let mut sink = 0u64;
    for _ in 0..n {
        let (_, _buf) = render_component(c.as_mut(), &store, &th, size, false);
        sink = sink.wrapping_add(1);
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    assert_eq!(sink, u64::from(n));

    // The cache key's backstop on its own: it serialises the tree and hashes
    // the string, and this is the biggest tree the arc ships.
    let view = view_of(c.as_mut(), &store, &th, size);
    let t = Instant::now();
    let mut h = 0u64;
    for _ in 0..n {
        h ^= gridwatch_ui::view::fingerprint(&view);
    }
    let fp = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    assert_ne!(h, u64::MAX);
    println!("view+render at 122x31: {per:.3} ms · fingerprint: {fp:.3} ms");
    assert!(
        fp < 1.0,
        "fingerprint is {fp:.3} ms per tile per frame — the §5 note's hand-rolled walker is due"
    );
    assert!(
        per < 3.0,
        "a single tile's view+render is {per:.3} ms — §13 budgets 0.3 ms of view and 3 ms of render for the whole frame"
    );
}

/// The `big-number` tier must never hand the `—` sentinel to the big-text font:
/// font8x8 has no glyph for U+2014 and `tui-big-text` draws *nothing* for a
/// character it cannot render, so a tile with no delta yet would be silently
/// blank. Reproduces a confirmed arc-1b review finding.
#[test]
fn a_tile_with_no_data_says_so_instead_of_going_blank() {
    let empty = gridwatch_store::Store::default();
    let th = theme("modern");
    for size in [Size::new(17, 8), Size::new(12, 4), Size::new(38, 8)] {
        let (_, buf) = render_component(htop().as_mut(), &empty, &th, size, false);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.trim().chars().any(|c| !c.is_whitespace()),
            "htop at {}x{} rendered a completely blank tile with no data",
            size.w,
            size.h
        );
        assert!(
            text.contains('—'),
            "htop at {}x{} must show the missing-data dash, got {text:?}",
            size.w,
            size.h
        );
    }
}

/// The no-panic sweep stops at `max(tier min) + 4` = 60 cells wide, so the
/// two-column header (76+) and the odd rectangles around every layout threshold
/// are never swept. Pin them explicitly.
#[test]
fn layout_thresholds_never_panic() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let store = demo_store(42, 40);
    let empty = gridwatch_store::Store::default();
    let th = theme("modern");
    let sizes = [
        (75, 3),
        (76, 3),
        (76, 4),
        (76, 5),
        (76, 6),
        (77, 6),
        (75, 6),
        (76, 12),
        (80, 12),
        (55, 12),
        (56, 11),
        (56, 12),
        (57, 13),
        (122, 4),
        (122, 12),
        (248, 4),
        (248, 66),
        (29, 5),
        (30, 6),
    ];
    for (w, h) in sizes {
        for s in [&store, &empty] {
            let r = catch_unwind(AssertUnwindSafe(|| {
                render_component(htop().as_mut(), s, &th, Size::new(w, h), false)
            }));
            assert!(r.is_ok(), "htop panicked at {w}x{h}");
        }
    }
}

/// Arc 11 (D61): a plugin whose uncatalogued names filled
/// `Retention::max_uncatalogued` is **visible**, not merely bounded — the
/// `sources` tile's NOTE carries `capped N` beside the reason, in the same
/// muted style as `dropped`. It is a note and never an alert: the run is
/// healthy, and the count is the evidence the plugin was throttled.
#[test]
fn the_sources_tile_says_when_a_plugin_was_capped() {
    use gridwatch_store::{Batch, Datum, Label, MetricId, Msg, Retention, Sample, SourceId, Ts};
    let mut store = gridwatch_store::Store::new(Retention {
        max_len: 64,
        max_age: std::time::Duration::from_secs(600),
        max_uncatalogued: 1,
    });
    let weather = SourceId("weather");
    store.ensure_source(weather);
    store.apply(&Msg::Batch(Batch {
        source: weather,
        at: Ts(1_000_000_000),
        samples: ["oslo", "kyoto", "quito"]
            .iter()
            .map(|c| Sample {
                id: MetricId {
                    name: "weather.temp",
                    label: Label::Name(std::sync::Arc::from(*c)),
                },
                datum: Datum::Scalar(11.0),
            })
            .collect(),
    }));
    assert_eq!(store.capped(weather), 2, "the fixture must actually cap");
    let th = theme("mono");
    let (tier, buf) = render_component(&mut *sources(), &store, &th, Size::new(80, 20), false);
    assert_eq!(tier, 1, "the note lives in the table tier's NOTE column");
    let text = gridwatch_ui::dump::cells(&buf);
    assert!(text.contains("capped 2"), "{text}");
    // A source that refused nothing says nothing.
    assert_eq!(text.matches("capped").count(), 1, "{text}");
}
