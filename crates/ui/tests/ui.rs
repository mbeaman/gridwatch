//! UI crate gate tests (§12): layout invariants, tier selection, themes.

use gridwatch_ui::component::{Size, Tier, pick_tier};
use gridwatch_ui::layout::{
    Direction, EditError, GridSpec, Page, PlaceTarget, Placement, SolveMode, derive_mode,
    focus_dir, hit, insert_first_fit, move_by, remove, resize_by, solve, swap, thresholds, tracks,
};
use gridwatch_ui::theme::{ColorMode, load_builtin, nearest_16, nearest_256};
use proptest::prelude::*;
use ratatui_core::layout::{Constraint as RC, Layout, Rect};

#[test]
fn tracks_cover_exactly_with_variance_one() {
    for len in [31u16, 68, 109, 131, 250] {
        for n in 1u8..=12 {
            for gap in [0u16, 1] {
                let t = tracks(len, n, gap);
                assert_eq!(t.len(), usize::from(n));
                let sum: u16 = t.iter().map(|(_, w)| w).sum::<u16>() + gap * (u16::from(n) - 1);
                assert_eq!(
                    sum,
                    len.max(gap * (u16::from(n) - 1)),
                    "len={len} n={n} gap={gap}"
                );
                let min = t.iter().map(|(_, w)| *w).min().unwrap();
                let max = t.iter().map(|(_, w)| *w).max().unwrap();
                assert!(max - min <= 1, "variance > 1 at len={len} n={n}");
                // Monotonic, non-overlapping starts.
                for pair in t.windows(2) {
                    assert_eq!(pair[1].0, pair[0].0 + pair[0].1 + gap);
                }
            }
        }
    }
}

#[test]
fn tracks_match_ratatui_fill_multiset() {
    // Oracle (§12.1): same width multiset as Layout::horizontal(vec![Fill(1); n]).
    for len in [109u16, 131, 250] {
        for n in [6u8, 12] {
            let mine: Vec<u16> = tracks(len, n, 0).into_iter().map(|(_, w)| w).collect();
            let area = Rect {
                x: 0,
                y: 0,
                width: len,
                height: 1,
            };
            let oracle = Layout::horizontal(vec![RC::Fill(1); usize::from(n)]).split(area);
            let mut a = mine.clone();
            let mut b: Vec<u16> = oracle.iter().map(|r| r.width).collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "len={len} n={n}");
        }
    }
}

#[test]
fn thresholds_for_default_grid_match_spec() {
    let (configured, dense) = thresholds(&GridSpec::default(), 2);
    assert_eq!((configured.w, configured.h), (131, 37));
    assert_eq!((dense.w, dense.h), (109, 27));
}

#[test]
fn mode_ladder_with_hysteresis() {
    let spec = GridSpec::default();
    let m = |w, h, prev| derive_mode(Size::new(w, h), &spec, 2, prev);
    assert_eq!(m(250, 70, SolveMode::Configured), SolveMode::Configured);
    assert_eq!(m(120, 40, SolveMode::Configured), SolveMode::Dense);
    assert_eq!(m(100, 20, SolveMode::Dense), SolveMode::Stack);
    // Hysteresis: exactly at the configured threshold, dense stays dense…
    assert_eq!(m(131, 37, SolveMode::Dense), SolveMode::Dense);
    // …and 2 cells above it recovers.
    assert_eq!(m(133, 39, SolveMode::Dense), SolveMode::Configured);
}

proptest! {
    #[test]
    fn mode_is_defined_for_all_sizes(w in 0u16..320, h in 0u16..120) {
        let spec = GridSpec::default();
        for prev in [SolveMode::Configured, SolveMode::Dense, SolveMode::Stack] {
            let _ = derive_mode(Size::new(w, h), &spec, 2, prev);
        }
    }
}

fn place(x: u8, y: u8, w: u8, h: u8) -> Placement {
    Placement {
        target: PlaceTarget::Kind("clock".into()),
        at: (x, y),
        size: (w, h),
        view: None,
        priority: 0,
    }
}

fn default_page() -> Page {
    Page {
        name: "Overview".into(),
        hotkey: Some('1'),
        place: vec![
            place(0, 0, 6, 3),
            place(6, 0, 6, 3),
            place(0, 3, 4, 2),
            place(4, 3, 4, 2),
            place(8, 3, 4, 2),
            place(0, 5, 4, 1),
            place(4, 5, 6, 1),
            place(10, 5, 2, 1),
        ],
    }
}

#[test]
fn solve_covers_the_default_layout_without_chips_at_250x70() {
    let spec = GridSpec::default();
    let body = Rect {
        x: 0,
        y: 1,
        width: 250,
        height: 68,
    };
    let s = solve(&spec, &default_page(), body, SolveMode::Configured, None, 0);
    assert_eq!(s.cells.len(), 8);
    for c in &s.cells {
        assert!(!c.chip, "cell {} starved at 250x70", c.index);
        assert!(c.inner.width >= spec.min_unit_inner.w);
        assert!(c.inner.height >= spec.min_unit_inner.h);
    }
    // The 6x3 hero is at least the spec's measured inner (§6).
    let hero = s.cells.iter().find(|c| c.index == 0).unwrap();
    assert!(
        hero.inner.width >= 122 && hero.inner.height >= 31,
        "hero inner {:?}",
        hero.inner
    );
    // hit() inverts the mouse.
    assert_eq!(hit(&s, hero.inner.x + 1, hero.inner.y + 1), Some(0));
    // Spatial focus from the hero rightward reaches the other hero.
    assert_eq!(focus_dir(&s, 0, Direction::Right), Some(1));
}

#[test]
fn solve_dense_at_120x40_keeps_every_tile_above_chip() {
    let spec = GridSpec::default();
    let body = Rect {
        x: 0,
        y: 1,
        width: 120,
        height: 38,
    };
    let s = solve(&spec, &default_page(), body, SolveMode::Dense, None, 0);
    assert_eq!(s.cells.len(), 8);
    for c in &s.cells {
        assert!(
            !c.chip,
            "cell {} starved at 120x40 dense: {:?}",
            c.index, c.inner
        );
    }
}

#[test]
fn zoom_gives_one_placement_the_body() {
    let spec = GridSpec::default();
    let body = Rect {
        x: 0,
        y: 1,
        width: 250,
        height: 68,
    };
    let s = solve(
        &spec,
        &default_page(),
        body,
        SolveMode::Configured,
        Some(3),
        0,
    );
    assert_eq!(s.cells.len(), 1);
    assert_eq!(s.cells[0].index, 3);
    assert_eq!(s.cells[0].outer, body);
}

proptest! {
    #[test]
    fn edit_ops_never_overlap_or_escape(
        idx in 0usize..8,
        dx in -3i8..=3, dy in -3i8..=3,
        dw in -2i8..=2, dh in -2i8..=2,
    ) {
        let spec = GridSpec::default();
        let page = default_page();
        for next in [move_by(&spec, &page, idx, dx, dy), resize_by(&spec, &page, idx, dw, dh)].into_iter().flatten() {
            for (i, p) in next.place.iter().enumerate() {
                prop_assert!(p.in_bounds(spec.columns, spec.rows));
                for (j, q) in next.place.iter().enumerate() {
                    if i != j {
                        prop_assert!(!p.overlaps(q));
                    }
                }
            }
        }
    }
}

#[test]
fn edit_swap_insert_remove() {
    let spec = GridSpec::default();
    let page = default_page();
    let swapped = swap(&spec, &page, 2, 3).unwrap();
    assert_eq!(swapped.place[2].at, page.place[3].at);
    let removed = remove(&page, 7).unwrap();
    assert_eq!(removed.place.len(), 7);
    let inserted = insert_first_fit(&spec, &removed, place(0, 0, 2, 1)).unwrap();
    assert_eq!(inserted.place.len(), 8);
    let full = default_page();
    // The default layout leaves a 2x1 hole at (8,5)..(10,6)? — (0..12 x 0..6): used rows fully?
    // Row 5: 0-3 amp, 4-9 temps, 10-11 clock → free none. Rows 0-4 fully covered. So a 3x1 cannot fit.
    assert_eq!(
        insert_first_fit(&spec, &full, place(0, 0, 3, 1)),
        Err(EditError::NoRoom)
    );
}

const T: &[Tier] = &[
    Tier {
        name: "tiny",
        min: Size::new(8, 3),
        adds: &[],
        zoom_only: false,
    },
    Tier {
        name: "meters",
        min: Size::new(30, 6),
        adds: &["meters"],
        zoom_only: false,
    },
    Tier {
        name: "table",
        min: Size::new(56, 18),
        adds: &["table"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &["everything"],
        zoom_only: true,
    },
];

#[test]
fn tier_selection_richest_fitting_skipping_zoom_only() {
    assert_eq!(pick_tier(T, Size::new(17, 8), false, None), (0, false));
    assert_eq!(pick_tier(T, Size::new(80, 20), false, None), (2, false)); // table (56×18) fits a 4x2 — §8.1
    assert_eq!(pick_tier(T, Size::new(122, 31), false, None), (2, false));
    // full is zoom_only: never by size alone…
    assert_eq!(pick_tier(T, Size::new(248, 66), false, None), (2, false));
    // …but zoom unlocks it.
    assert_eq!(pick_tier(T, Size::new(248, 66), true, None), (3, false));
    // A preferred view that fits wins; one that doesn't sets the fallback flag.
    assert_eq!(
        pick_tier(T, Size::new(122, 31), false, Some("meters")),
        (1, false)
    );
    assert_eq!(
        pick_tier(T, Size::new(17, 8), false, Some("table")),
        (0, true)
    );
    // Unknown view names are ignored (§4.6).
    assert_eq!(
        pick_tier(T, Size::new(122, 31), false, Some("nope")),
        (2, false)
    );
}

#[test]
fn builtin_themes_load_and_mono_is_colorless() {
    for name in ["modern", "retrowave", "mono"] {
        let t = load_builtin(name, ColorMode::TrueColor).unwrap();
        assert_eq!(t.name, name);
        // Self-contained per D37: loader enforced all roles + 8 gradients.
        assert!(t.warnings.iter().all(|w| !w.contains("missing")));
    }
    let mono = load_builtin("mono", ColorMode::TrueColor).unwrap();
    assert_eq!(
        mono.color(gridwatch_ui::Role::Text),
        ratatui_core::style::Color::Reset
    );
}

#[test]
fn nearest_colour_known_values() {
    assert_eq!(nearest_256(0, 0, 0), 16);
    assert_eq!(nearest_256(255, 255, 255), 231);
    assert_eq!(nearest_256(255, 0, 0), 196);
    assert_eq!(nearest_256(128, 128, 128), 244);
    assert_eq!(nearest_16(255, 255, 255), ratatui_core::style::Color::White);
    assert_eq!(nearest_16(200, 40, 40), ratatui_core::style::Color::Red);
}

#[test]
fn role_swatches_pin_the_palettes() {
    for name in ["modern", "retrowave", "mono"] {
        let t = load_builtin(name, ColorMode::TrueColor).unwrap();
        insta::assert_yaml_snapshot!(
            format!("swatch_{name}"),
            gridwatch_ui::testkit::role_swatch(&t)
        );
    }
}

mod review_regressions {
    use gridwatch_ui::component::{Size, Tier, pick_tier};
    use gridwatch_ui::layout::{PlaceTarget, Placement};

    const T: &[Tier] = &[
        Tier {
            name: "mini",
            min: Size::new(8, 3),
            adds: &[],
            zoom_only: false,
        },
        Tier {
            name: "meters",
            min: Size::new(20, 8),
            adds: &[],
            zoom_only: false,
        },
        Tier {
            name: "full",
            min: Size::new(40, 16),
            adds: &[],
            zoom_only: true,
        },
    ];

    /// §4.6: zoom always gives the richest tier; a pinned view applies un-zoomed only.
    #[test]
    fn zoom_overrides_pinned_view() {
        let big = Size::new(200, 60);
        assert_eq!(pick_tier(T, big, false, Some("mini")), (0, false));
        assert_eq!(pick_tier(T, big, true, Some("mini")), (2, false));
    }

    /// `at + size` can exceed u8::MAX; it must fail bounds, not wrap.
    #[test]
    fn in_bounds_does_not_wrap_u8() {
        let p = Placement {
            target: PlaceTarget::Id("x".into()),
            at: (250, 0),
            size: (20, 1),
            view: None,
            priority: 0,
        };
        assert!(!p.in_bounds(12, 6));
    }
}
