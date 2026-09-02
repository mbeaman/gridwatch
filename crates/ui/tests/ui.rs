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
    for name in gridwatch_ui::theme::BUILTIN_THEMES {
        let t = load_builtin(name, ColorMode::TrueColor).unwrap();
        assert_eq!(t.name, *name);
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
    for name in gridwatch_ui::theme::BUILTIN_THEMES {
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

// ───────────────────────── theme loader v2 (D52) ─────────────────────────

mod loader_v2 {
    use gridwatch_ui::ColorMode;
    use gridwatch_ui::theme::{
        BorderKind, GaugeStyle, GradientId, Role, build_theme, contrast_ratio, load_builtin,
        load_theme_file,
    };
    use ratatui_core::style::Color;

    const MODERN: &str = include_str!("../../../themes/modern.toml");

    /// `inherits` merges key by key: phosphor-green takes mono's widget and
    /// title tables, keeps its own borders and paints everything itself.
    #[test]
    fn inherits_merges_key_by_key() {
        let t = load_builtin("phosphor-green", ColorMode::TrueColor).unwrap();
        assert_eq!(t.name, "phosphor-green");
        assert_eq!(t.widgets.gauge, GaugeStyle::Bar, "from mono");
        assert!(t.title.bold, "from mono");
        assert_eq!(t.borders.set, BorderKind::Plain, "its own");
        assert_eq!(t.borders.focused, BorderKind::Double, "its own");
        assert_eq!(t.color(Role::Text), Color::Rgb(0xb6, 0xff, 0xc9));
        assert_ne!(
            t.gradient(GradientId::Load).sample(1.0),
            Color::Reset,
            "its own gradients"
        );
        assert!(
            t.warnings.iter().all(|w| !w.contains("WCAG")),
            "phosphor-green passes the gate on its own numbers: {:?}",
            t.warnings
        );
    }

    /// A child that inherits may override one key; a parent that itself
    /// inherits is a chain and an error; a missing parent is an error.
    #[test]
    fn a_child_overrides_one_key_and_chains_are_refused() {
        let parent = load_theme_file(MODERN).unwrap();
        let child = load_theme_file(
            "[meta]\nname = \"kid\"\nschema = 1\ninherits = \"modern\"\n[colors]\ntext = \"#ffffff\"\n",
        )
        .unwrap();
        let t = build_theme(&child, Some(&parent), ColorMode::TrueColor).unwrap();
        assert_eq!(t.name, "kid");
        assert_eq!(t.color(Role::Text), Color::Rgb(255, 255, 255));
        assert_eq!(
            t.color(Role::Bg),
            Color::Rgb(0x1e, 0x1e, 0x2e),
            "the rest is modern's"
        );
        let Err(err) = build_theme(&child, None, ColorMode::TrueColor) else {
            panic!("an orphan child built")
        };
        assert!(err.to_string().contains("no parent"), "{err}");
        let grand =
            load_theme_file("[meta]\nname = \"mid\"\nschema = 1\ninherits = \"modern\"\n").unwrap();
        let Err(err) = build_theme(&child, Some(&grand), ColorMode::TrueColor) else {
            panic!("a chain built")
        };
        assert!(err.to_string().contains("chains"), "{err}");
        // Self-contained files still have to be complete (D37).
        let partial = load_theme_file("[meta]\nname = \"p\"\nschema = 1\n").unwrap();
        let Err(err) = build_theme(&partial, None, ColorMode::TrueColor) else {
            panic!("a partial self-contained theme built")
        };
        assert!(err.to_string().contains("colors.surface missing"), "{err}");
    }

    /// `[components.<kind>]` derives a theme for that kind only.
    #[test]
    fn component_overrides_derive_a_theme_per_kind() {
        let text =
            format!("{MODERN}\n[components.htop]\ngradients.load = [\"#000000\", \"#ffffff\"]\n");
        let t = build_theme(&load_theme_file(&text).unwrap(), None, ColorMode::TrueColor).unwrap();
        let base = t.gradient(GradientId::Load).sample(1.0);
        let htop = t.for_kind("htop").gradient(GradientId::Load).sample(1.0);
        // Oklab round-trips within one step of the stop.
        assert!(
            matches!(htop, Color::Rgb(r, g, b) if r >= 254 && g >= 254 && b >= 254),
            "{htop:?}"
        );
        assert_ne!(base, htop);
        assert_eq!(
            t.for_kind("gpu").gradient(GradientId::Load).sample(1.0),
            base,
            "an unmentioned kind gets the base theme"
        );
        assert_eq!(t.for_kind("htop").color(Role::Text), t.color(Role::Text));
        assert_eq!(t.overridden_kinds().collect::<Vec<_>>(), vec!["htop"]);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    }

    /// The WCAG gate on known pairs (brief 3b): black on white 21:1, #767676
    /// on white 4.54:1; a theme below the floor warns and still loads; mono's
    /// `default` colours cannot be judged and say nothing.
    #[test]
    fn wcag_gate_on_known_pairs() {
        let r = contrast_ratio(Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255)).unwrap();
        assert!((r - 21.0).abs() < 0.01, "{r}");
        let r = contrast_ratio(Color::Rgb(0x76, 0x76, 0x76), Color::Rgb(255, 255, 255)).unwrap();
        assert!((r - 4.54).abs() < 0.01, "{r}");
        assert!(contrast_ratio(Color::Reset, Color::Rgb(0, 0, 0)).is_none());
        let low = MODERN
            .replace("text = \"#cdd6f4\"", "text = \"#767676\"")
            .replace("text_muted = \"#a6adc8\"", "text_muted = \"#3a3a4a\"");
        let t = build_theme(&load_theme_file(&low).unwrap(), None, ColorMode::TrueColor).unwrap();
        let wcag: Vec<&String> = t.warnings.iter().filter(|w| w.contains("WCAG")).collect();
        assert_eq!(wcag.len(), 4, "{wcag:?}");
        assert!(wcag[0].contains("text on panel"), "{}", wcag[0]);
        assert!(wcag[0].contains("below 4.5:1"), "{}", wcag[0]);
        assert!(wcag[2].contains("text_muted on panel"), "{}", wcag[2]);
        let report = t.contrast_report();
        assert!(
            report
                .iter()
                .any(|l| l.contains("text_ghost on panel") && l.contains("info"))
        );
        for name in ["modern", "retrowave", "mono", "terminal"] {
            let t = load_builtin(name, ColorMode::TrueColor).unwrap();
            assert!(
                t.warnings.iter().all(|w| !w.contains("WCAG")),
                "{name}: {:?}",
                t.warnings
            );
        }
    }

    /// `terminal` names the sixteen colours; its gradients step through them.
    #[test]
    fn terminal_theme_uses_the_palette_by_name() {
        let t = load_builtin("terminal", ColorMode::TrueColor).unwrap();
        assert_eq!(t.color(Role::Bg), Color::Reset);
        assert_eq!(t.color(Role::Text), Color::Reset, "the terminal's own pair");
        assert_eq!(t.color(Role::TextMuted), Color::DarkGray);
        assert_eq!(t.color(Role::Crit), Color::Red);
        let g = t.gradient(GradientId::Load);
        assert_eq!(g.sample(0.0), Color::Green);
        assert_eq!(g.sample(1.0), Color::Red);
        // Downsampling never touches a named colour; mono blanks it.
        let t16 = load_builtin("terminal", ColorMode::Ansi16).unwrap();
        assert_eq!(t16.color(Role::Crit), Color::Red);
        let mono = load_builtin("terminal", ColorMode::Mono).unwrap();
        assert_eq!(mono.color(Role::Crit), Color::Reset);
        assert_eq!(
            gridwatch_ui::theme::parse_color("ansi:208").unwrap(),
            Color::Indexed(208)
        );
        assert!(gridwatch_ui::theme::parse_color("ansi:300").is_err());
    }

    /// A built-in that inherits (phosphor-green → mono) is flattened, so a
    /// user file may inherit it; `class` is inherited unless set.
    #[test]
    fn builtins_are_flattened_and_class_is_inherited() {
        let parent = gridwatch_ui::theme::builtin_file("phosphor-green").unwrap();
        assert!(parent.meta.inherits.is_none());
        let child = load_theme_file(
            "[meta]\nname = \"dull\"\nschema = 1\ninherits = \"phosphor-green\"\n[colors]\ntext = \"#ffffff\"\n",
        )
        .unwrap();
        let t = build_theme(&child, Some(&parent), ColorMode::TrueColor).unwrap();
        assert_eq!(t.color(Role::Text), Color::Rgb(255, 255, 255));
        assert_eq!(t.color(Role::Bg), Color::Rgb(0x0a, 0x0f, 0x0a));
        assert_eq!(
            t.widgets.gauge,
            GaugeStyle::Bar,
            "mono's, through phosphor-green"
        );
        let showcase = load_theme_file(&MODERN.replace(
            "variant = \"dark\"",
            "variant = \"dark\"\nclass = \"showcase\"",
        ))
        .unwrap();
        let kid =
            load_theme_file("[meta]\nname = \"kid\"\nschema = 1\ninherits = \"modern\"\n").unwrap();
        let t = build_theme(&kid, Some(&showcase), ColorMode::TrueColor).unwrap();
        assert_eq!(t.class, gridwatch_ui::PerfClass::Showcase);
        // Self-inheritance is refused by name.
        let me = load_theme_file("[meta]\nname = \"me\"\nschema = 1\ninherits = \"me\"\n").unwrap();
        let Err(err) = build_theme(&me, Some(&me), ColorMode::TrueColor) else {
            panic!("self-inheritance built")
        };
        assert!(err.to_string().contains("cannot inherit itself"), "{err}");
        // A parse error names line and column.
        let Err(err) = load_theme_file("[meta]\nname = 3\n") else {
            panic!("parsed")
        };
        assert!(err.to_string().starts_with("theme: 2:8: "), "{err}");
    }

    /// `overlay::dim` really strips BOLD/REVERSED and adds DIM (review: the
    /// first version used `set_style`, which only adds), and the badge sets
    /// its own modifiers over whatever it covers.
    #[test]
    fn dim_strips_modifiers_and_the_badge_owns_its_style() {
        use gridwatch_ui::overlay::{dim, stale_badge};
        use ratatui_core::buffer::Buffer;
        use ratatui_core::layout::Rect;
        use ratatui_core::style::{Modifier, Style};
        let t = load_builtin("terminal", ColorMode::TrueColor).unwrap();
        let area = Rect::new(0, 0, 20, 2);
        let mut buf = Buffer::empty(area);
        buf.set_string(
            0,
            0,
            "PID USER CPU% MEM%  ",
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        );
        dim(area, &t, &mut buf);
        let c = buf.cell((1, 0)).unwrap();
        assert!(!c.modifier.contains(Modifier::REVERSED));
        assert!(!c.modifier.contains(Modifier::BOLD));
        assert!(c.modifier.contains(Modifier::DIM));
        assert_eq!(c.fg, t.color(Role::TextMuted));
        buf.set_string(
            0,
            1,
            "header header header",
            Style::new().add_modifier(Modifier::REVERSED),
        );
        stale_badge(837, Rect::new(0, 1, 20, 1), &t, &mut buf);
        let text: String = (0..20)
            .map(|x| buf.cell((x, 1)).unwrap().symbol().to_string())
            .collect();
        assert!(text.ends_with("STALE 13m"), "{text}");
        let b = buf.cell((19, 1)).unwrap();
        assert_eq!(b.modifier, Modifier::BOLD, "no inherited REVERSED");
        assert_eq!(gridwatch_ui::overlay::stale_age_text(119), "119s");
        assert_eq!(gridwatch_ui::overlay::stale_age_text(7200), "2h");
    }
}

// ───────────────────────── arc 4a: the mouse inverse ─────────────────────────

mod unit_inverse {
    use gridwatch_ui::component::Size;
    use gridwatch_ui::layout::{
        GridSpec, Page, PlaceTarget, Placement, SolveMode, footprint_cycle, solve, unit_at,
        unit_rect,
    };
    use proptest::prelude::*;
    use ratatui_core::layout::Rect;

    fn page() -> Page {
        let p = |id: &str, at, size| Placement {
            target: PlaceTarget::Id(id.into()),
            at,
            size,
            view: None,
            priority: 0,
        };
        Page {
            name: "p".into(),
            hotkey: None,
            place: vec![
                p("a", (0, 0), (6, 3)),
                p("b", (6, 0), (6, 3)),
                p("c", (0, 3), (4, 2)),
                p("d", (10, 5), (2, 1)),
            ],
        }
    }

    proptest! {
        /// Every cell of every solved tile maps back to a unit inside that
        /// placement, in both grid modes and at every body size that solves.
        #[test]
        fn unit_at_inverts_solve(w in 60u16..300, h in 20u16..90, dense in any::<bool>()) {
            let spec = GridSpec::default();
            let mode = if dense { SolveMode::Dense } else { SolveMode::Configured };
            let body = Rect::new(3, 2, w, h);
            let solved = solve(&spec, &page(), body, mode, None, 0);
            for cell in &solved.cells {
                let p = &page().place[cell.index];
                let r = cell.outer;
                // Dense mode shares one border column/row between neighbours
                // (§6): that cell belongs to both tiles, and `unit_at` gives
                // it to the left/upper one — so the shared edge is skipped.
                let ov = u16::from(dense);
                for y in r.y + ov..r.y + r.height {
                    for x in r.x + ov..r.x + r.width {
                        let (ux, uy) = unit_at(&spec, body, mode, x, y).expect("inside the body");
                        prop_assert!(ux >= p.at.0 && ux < p.at.0 + p.size.0, "x {x} → unit {ux} outside {:?}", p.at);
                        prop_assert!(uy >= p.at.1 && uy < p.at.1 + p.size.1, "y {y} → unit {uy} outside {:?}", p.at);
                    }
                }
                // And the ghost rect for that placement is its outer rect.
                prop_assert_eq!(unit_rect(&spec, body, mode, p.at, p.size), Some(r));
            }
            prop_assert!(unit_at(&spec, body, mode, body.x + body.width, body.y).is_none());
            prop_assert!(unit_at(&spec, body, SolveMode::Stack, body.x, body.y).is_none());
        }
    }

    #[test]
    fn footprint_cycle_wraps_and_starts_over() {
        let fps = [(1, 1), (2, 1), (4, 2)];
        assert_eq!(footprint_cycle(&fps, (1, 1)), Some((2, 1)));
        assert_eq!(footprint_cycle(&fps, (4, 2)), Some((1, 1)));
        assert_eq!(
            footprint_cycle(&fps, (6, 3)),
            Some((1, 1)),
            "unlisted → first"
        );
        assert_eq!(footprint_cycle(&[], (1, 1)), None);
        let _ = Size::new(8, 3);
    }
}
