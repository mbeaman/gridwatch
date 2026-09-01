//! Shell-level regression tests (§5, §11): cache invalidation, drain order,
//! and the mouse double-click window — each reproduces a confirmed arc-1a
//! review finding.

use std::collections::BTreeMap;

use gridwatch_app::{Shell, config, probe, run_loop, shot_frame};
use gridwatch_store::demo::CpuSynth;
use gridwatch_store::{
    Clock, ControlMsg, InputEvent, KeyCode, KeyEvent, Msg, SourceId, SourceState, SourceStatus, Ts,
    channels,
};
use gridwatch_ui::theme::load_builtin;
use gridwatch_ui::{ColorMode, Registry};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn shell() -> Shell {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let loaded = config::load_embedded().unwrap();
    let theme = load_builtin("mono", ColorMode::Mono).unwrap();
    Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        false,
    )
}

/// The review's headline finding: a tile whose view reads store state outside
/// its declared sources (the sources tile) must repaint when that state moves.
/// The view fingerprint in the cache key is the §5 backstop that makes it so.
#[test]
fn sources_tile_never_serves_stale_state() {
    let mut sh = shell();
    sh.store.ensure_source(SourceId("cpu"));
    let mut synth = CpuSynth::new(7);
    sh.store.apply(&Msg::Batch(synth.tick(Ts(1_500_000_000))));
    let a = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    sh.store.apply(&Msg::Batch(synth.tick(Ts(3_000_000_000))));
    let b = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_ne!(a, b, "generation advanced but the frame did not change");
    // And stability: no data change → byte-identical frame (cache hit).
    let c = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_eq!(b, c, "nothing changed, but the frame differs");
}

/// §4.2/§5 drain order: 'q' quits, but queued control and data land first.
#[test]
fn drain_order_applies_control_and_data_before_quit() {
    let (ch, inbox) = channels();
    let mut sh = shell();
    sh.store.ensure_source(SourceId("cpu"));
    let mut synth = CpuSynth::new(1);
    ch.control
        .send(ControlMsg::Status(
            SourceId("cpu"),
            SourceStatus {
                state: SourceState::Ok,
                reason: None,
                hint: None,
                since: Ts(1),
                last_sample: None,
                dropped: 3,
                restarts: 9,
            },
        ))
        .unwrap();
    ch.data.try_send(synth.tick(Ts(1_500_000_000))).unwrap();
    ch.input
        .send(InputEvent::Key(KeyEvent::plain(KeyCode::Char('q'))))
        .unwrap();
    let mut term = Terminal::new(TestBackend::new(131, 40)).unwrap();
    run_loop(&mut term, &mut sh, &inbox).unwrap();
    assert!(sh.quit);
    assert_eq!(
        sh.store.status(SourceId("cpu")).restarts,
        9,
        "control drained"
    );
    assert!(sh.store.generation(SourceId("cpu")) > 0, "data drained");
}

/// Two clicks inside 400 ms zoom the tile (the review found the old
/// frame-counter coincidence could never fire).
#[test]
fn double_click_zooms_and_esc_restores() {
    use gridwatch_store::{Mods, MouseButton, MouseEvent, MouseKind};
    let mut sh = shell();
    sh.store.ensure_source(SourceId("cpu"));
    let mut synth = CpuSynth::new(7);
    sh.store.apply(&Msg::Batch(synth.tick(Ts(1_500_000_000))));
    let a = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    let click = || {
        InputEvent::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            x: 5,
            y: 3,
            mods: Mods::NONE,
        })
    };
    sh.handle_input(click());
    let _ = shot_frame(&mut sh, 250, 70); // a frame between the clicks
    sh.handle_input(click());
    let zoomed = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_ne!(a, zoomed, "double-click did not zoom");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    let back = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_eq!(a, back, "Esc did not restore the grid");
}

/// Every tile of a page, as plain text (style tags stripped): the cheapest
/// honest way to ask "which tier did the shell pick?" from outside.
fn page_text(sh: &mut Shell, w: u16, h: u16) -> String {
    let frame = shot_frame(sh, w, h);
    let dump = gridwatch_ui::dump::cells(&frame);
    let mut out = String::new();
    let mut in_tag = false;
    for c in dump.chars() {
        match c {
            '[' => in_tag = true,
            ']' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.to_lowercase()
}

/// `page` is 1-based, like the CLI's `--page`.
fn shell_with(layout: &str, page: usize) -> Shell {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let loaded = config::load_texts(config::DEFAULT_CONFIG, layout).unwrap();
    let theme = load_builtin("mono", ColorMode::Mono).unwrap();
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        false,
    );
    sh.set_page(page.saturating_sub(1));
    gridwatch_app::feed_synth(&mut sh, 42, 40);
    sh
}

/// Arc 1b acceptance (brief task 5): the shipped placements pick `cores` for
/// the Overview's cpu tile at 250×70 *and* in dense mode at 120×40, and the
/// Audio page's 12x3 tile honours its `view = "meters"` preference instead of
/// growing into the richer tier that would also fit.
#[test]
fn shipped_placements_pick_the_expected_htop_tiers() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let big = page_text(&mut sh, 250, 70);
    assert!(
        big.contains("ccd0") && big.contains("ccd1"),
        "6x3 at 250x70 is not `cores`"
    );
    assert!(big.contains("psi cpu"), "the cores tier draws the PSI row");

    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let dense = page_text(&mut sh, 120, 40);
    assert!(
        dense.contains("ccd0") && dense.contains("ccd1"),
        "6x3 dense at 120x40 must keep `cores` (59×18 fits the 56×12 minimum)"
    );

    // Page 2's cpu tile is 12x3 with `view = "meters"`: the pinned tier wins
    // even though `cores` would fit a 248-wide rect (§4.6).
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 2);
    let audio = page_text(&mut sh, 250, 70);
    assert!(
        audio.contains("pids,"),
        "the meters tier draws the task line"
    );
    assert!(
        !audio.contains("ccd0"),
        "`view = \"meters\"` must not grow into `cores`"
    );
}

/// The laptop layout fixture is the 120×40 case in a file, so a change to the
/// shipped default cannot silently drop the dense-mode guarantee.
#[test]
fn laptop_fixture_stays_above_chip_level_in_dense_mode() {
    let layout = include_str!("../../../fixtures/layouts/laptop-120x40.toml");
    let mut sh = shell_with(layout, 1);
    let text = page_text(&mut sh, 120, 40);
    assert!(text.contains("ccd0"), "the cpu tile lost `cores` at 120x40");
    assert!(!text.contains("starved"), "a tile fell to a chip at 120x40");
    assert!(text.contains("00:00"), "the clock tile is missing");
}

/// §4.6: a placement naming a tier that does not exist (`table` until arc 2)
/// is a config warning, and the tile falls back to the richest fitting tier.
#[test]
fn an_unknown_view_name_warns_and_falls_back() {
    let layout = config::DEFAULT_LAYOUT.replace("view = \"meters\"", "view = \"table\"");
    let mut sh = shell_with(&layout, 2);
    assert!(
        sh.view_warnings().iter().any(|w| w.contains("table")),
        "no warning for the unknown view name: {:?}",
        sh.view_warnings()
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("ccd0"),
        "the fallback must be the richest fitting tier"
    );
}

/// The whole live path in one test: the real cpu source on its supervised
/// thread → the bounded data channel → `Store::apply` → the htop tile's view.
/// Nothing else in the suite proves the source the binary actually runs (the
/// snapshots are all demo-driven, §12.5).
#[test]
fn the_live_cpu_source_reaches_the_htop_tile() {
    use std::time::{Duration, Instant};

    use gridwatch_store::{Detail, Level};

    let (ch, inbox) = channels();
    let clock = Clock::real_starting_now();
    let handle = gridwatch_sources::spawn_source(
        SourceId("cpu"),
        || Box::new(gridwatch_sources::cpu::CpuSource::new(&toml::Table::new())),
        ch,
        clock,
        toml::Table::new(),
    );
    // Focused is htop's own 500 ms; two scans give the deltas percentages need.
    handle.demand.set(Level::Focused, Detail::Meters);

    let mut sh = shell();
    sh.store.ensure_source(SourceId("cpu"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && sh.store.generation(SourceId("cpu")) < 2 {
        while let Ok(b) = inbox.data.try_recv() {
            sh.store.apply(&Msg::Batch(b));
        }
        while let Ok(c) = inbox.control.try_recv() {
            sh.store.apply(&Msg::Control(c));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    handle.shutdown();

    assert!(
        sh.store.generation(SourceId("cpu")) >= 2,
        "the live cpu source produced no second batch in 10 s"
    );
    let (_, total) = sh
        .store
        .last(&gridwatch_store::keys::cpu::TOTAL_PCT)
        .expect("cpu.total_pct after two scans");
    assert!((0.0..=100.0).contains(&total), "total_pct = {total}");
    let topo = sh
        .store
        .record(&gridwatch_store::keys::cpu::TOPOLOGY)
        .expect("the die map is published on the first scan");
    assert!(!topo.1.is_empty(), "empty topology");
    // And the tile draws it: the CCD block header names the die the source found.
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("ccd0"),
        "the htop tile did not draw live data"
    );
    assert!(text.contains("pids,"), "the task line is missing");
}

/// §9: a source's options live under `[sources.<id>]` and an instance's options
/// are view-only; no name may appear in both, or `refresh_ms` would silently
/// mean two things. The app crate is the lowest place that can see both lists —
/// `gridwatch-components` must not depend on `gridwatch-sources`, even in dev.
#[test]
fn source_and_component_option_names_are_disjoint() {
    for name in gridwatch_components::htop::OPTION_NAMES {
        assert!(
            !gridwatch_sources::cpu::OPTION_NAMES.contains(name),
            "`{name}` is both an htop view option and a [sources.cpu] option"
        );
    }
}
