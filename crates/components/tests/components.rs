//! Component gate tests (§12): snapshot matrix, no-panic sweep, tier hygiene.

use gridwatch_components::htop::{Htop, OPTION_NAMES, Options};
use gridwatch_ui::component::{Component, Size, pick_tier};
use gridwatch_ui::testkit::{
    assert_min_tier_fits, assert_never_panics, assert_tiers_well_formed, demo_store,
    real_grid_sizes, render_component, theme, view_of, view_snapshot,
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

#[test]
fn tiers_are_well_formed() {
    for mk in [clock, sources, htop] {
        let c = mk();
        assert_tiers_well_formed(c.tiers());
        assert_min_tier_fits(c.tiers(), Size::new(8, 3));
    }
}

#[test]
fn never_panics_across_sizes() {
    let store = demo_store(42, 3);
    let th = theme("modern");
    assert_never_panics(&|| clock(), &store, &th);
    assert_never_panics(&|| sources(), &store, &th);
    assert_never_panics(&|| htop(), &store, &th);
}

#[test]
fn view_snapshots_at_real_grid_sizes() {
    let store = demo_store(42, 3);
    let history = demo_store(42, 40);
    let th = theme("modern");
    for (name, size) in real_grid_sizes() {
        let c = clock();
        insta::assert_yaml_snapshot!(
            format!("clock_{name}"),
            view_snapshot(c.as_ref(), &store, &th, size)
        );
        let s = sources();
        insta::assert_yaml_snapshot!(
            format!("sources_{name}"),
            view_snapshot(s.as_ref(), &store, &th, size)
        );
        // A minute of history, so the snapshot pins a real sparkline rather
        // than three samples in one bucket.
        let h = htop();
        insta::assert_yaml_snapshot!(
            format!("htop_{name}"),
            view_snapshot(h.as_ref(), &history, &th, size)
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
    assert_eq!(tier(122, 31, false), ("cores", false), "6x3 at 250x70");
    assert_eq!(tier(59, 18, false), ("cores", false), "6x3 dense at 120x40");
    assert_eq!(tier(80, 20, false), ("cores", false), "4x2 at 250x70");
    assert_eq!(tier(39, 11, false), ("meters", false), "4x2 dense");
    assert_eq!(tier(38, 8, false), ("meters", false), "2x1 at 250x70");
    assert_eq!(tier(17, 8, false), ("big-number", false), "1x1 at 250x70");
    assert_eq!(tier(9, 5, false), ("tiny", false), "1x1 dense at 120x40");
    assert_eq!(tier(248, 66, true), ("cores", false), "zoomed");
    // A pinned view that does not fit falls back and raises the chip (§4.6).
    let (i, fallback) = pick_tier(c.tiers(), Size::new(17, 8), false, Some("cores"));
    assert_eq!((c.tiers()[i].name, fallback), ("big-number", true));
    // An unknown view name (`table` until arc 2) is ignored, not fatal.
    let (i, fallback) = pick_tier(c.tiers(), Size::new(122, 31), false, Some("table"));
    assert_eq!((c.tiers()[i].name, fallback), ("cores", false));
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
    let (_, buf) = render_component(clock().as_ref(), &store, &th, Size::new(38, 8), false);
    insta::assert_snapshot!("clock_cells_2x1", gridwatch_ui::dump::cells(&buf));
    let (_, buf) = render_component(sources().as_ref(), &store, &th, Size::new(80, 20), false);
    insta::assert_snapshot!("sources_cells_4x2", gridwatch_ui::dump::cells(&buf));
    // The hero: the tier the screenshot is of, and the dense 6x3 beside it.
    let history = demo_store(42, 40);
    let (_, buf) = render_component(htop().as_ref(), &history, &th, Size::new(122, 31), false);
    insta::assert_snapshot!("htop_cells_6x3", gridwatch_ui::dump::cells(&buf));
    let (_, buf) = render_component(htop().as_ref(), &history, &th, Size::new(59, 18), false);
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
    let c = htop();
    let size = Size::new(122, 31);
    // Warm up, then measure view + fingerprint together — the pair is what a
    // frame pays for a tile whose data moved.
    for _ in 0..50 {
        let _ = render_component(c.as_ref(), &store, &th, size, false);
    }
    let n = 500u32;
    let t = Instant::now();
    let mut sink = 0u64;
    for _ in 0..n {
        let (_, _buf) = render_component(c.as_ref(), &store, &th, size, false);
        sink = sink.wrapping_add(1);
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    assert_eq!(sink, u64::from(n));

    // The cache key's backstop on its own: it serialises the tree and hashes
    // the string, and this is the biggest tree the arc ships.
    let view = view_of(c.as_ref(), &store, &th, size);
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
        let (_, buf) = render_component(htop().as_ref(), &empty, &th, size, false);
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
                render_component(htop().as_ref(), s, &th, Size::new(w, h), false)
            }));
            assert!(r.is_ok(), "htop panicked at {w}x{h}");
        }
    }
}
