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
    shell_with_ticks(layout, page, 40)
}

fn shell_with_ticks(layout: &str, page: usize, ticks: usize) -> Shell {
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
        BTreeMap::new(),
        false,
    );
    sh.set_page(page.saturating_sub(1));
    gridwatch_app::feed_synth(&mut sh, 42, ticks);
    sh
}

/// Arc 2a acceptance (brief 2a, §8.1 row budget): the shipped placements pick
/// `table` for the Overview's cpu tile — **10 rows** at 250×70 (18 available,
/// capped by `table_rows`), **5 rows** in dense mode at 120×40 (59×18: exactly
/// the tier's floor) — and the Audio page's 12x3 tile honours its
/// `view = "meters"` preference instead of growing into the richer tiers.
/// Row counts are read off the synth's sort order (CPU% desc, seed 42): the
/// 10th row is `dockerd`/`pipewire` territory and `wireplumber` is 11th+;
/// the 5th row is `rsync` and the 6th the firefox content process.
#[test]
fn shipped_placements_pick_the_expected_htop_tiers() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let big = page_text(&mut sh, 250, 70);
    assert!(
        big.contains("ccd0") && big.contains("ccd1"),
        "6x3 at 250x70 lost the cores block"
    );
    assert!(big.contains("psi cpu"), "the table tier keeps the PSI row");
    assert!(
        big.contains("kthr;"),
        "the task line has htop's wording once kthr exists"
    );
    assert!(
        big.contains("time+") && big.contains("command"),
        "6x3 at 250x70 is not `table`: {big}"
    );
    assert!(
        big.contains("dockerd") && big.contains("pipewire"),
        "the 10-row table must reach the 9th/10th rows"
    );
    assert!(
        !big.contains("wireplumber"),
        "table_rows = 10 must cap the table at ten rows"
    );

    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let dense = page_text(&mut sh, 120, 40);
    assert!(
        dense.contains("ccd0") && dense.contains("ccd1"),
        "6x3 dense at 120x40 lost the cores block"
    );
    assert!(
        dense.contains("command"),
        "6x3 dense at 120x40 must keep the table (59×19)"
    );
    // The solver gives the top grid row 19 inner lines at 120×40, one more
    // than the testkit's canonical 59×18, so the budget is six rows here —
    // count them rather than trust a truncated Command column.
    let rows = dense
        .lines()
        .skip_while(|l| !l.contains("command"))
        .skip(1)
        .take_while(|l| {
            l.trim_start_matches(['┃', '│', ' '])
                .starts_with(|c: char| c.is_ascii_digit())
        })
        .count();
    assert_eq!(rows, 6, "the dense table's row budget:\n{dense}");

    // Page 2's cpu tile is 12x3 with `view = "meters"`: the pinned tier wins
    // even though `table` would fit a 248-wide rect (§4.6).
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 2);
    let audio = page_text(&mut sh, 250, 70);
    assert!(
        audio.contains("running"),
        "the meters tier draws the task line"
    );
    assert!(
        !audio.contains("ccd0") && !audio.contains("time+"),
        "`view = \"meters\"` must not grow into `cores` or `table`"
    );
}

/// Arc 2b acceptance (brief 2b, §8.1): the Overview's gpu tile is `procs` at
/// 250×70 and in dense mode at 120×40, and the *zoomed* gpu tile shows USER,
/// CPU, HOST MEM and Command for the game — joined from the cpu source's
/// scan, which the gpu tile's `demand` raised to `Detail::Table` on its own.
#[test]
fn shipped_placements_pick_the_expected_gpu_tiers() {
    use gridwatch_store::{Mods, MouseButton, MouseEvent, MouseKind};
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let big = page_text(&mut sh, 250, 70);
    assert!(
        big.contains("pcie gen 5@16x") && big.contains("pow"),
        "6x3 at 250x70 lost nvtop's header: {big}"
    );
    assert!(
        big.contains("gpu mem") && big.contains("both g+c"),
        "6x3 at 250x70 is not `procs`: {big}"
    );
    assert!(
        big.contains("12.5gib"),
        "the game's HOST MEM is joined from proc.table"
    );
    assert!(big.contains("1:util"), "the chart band's legend");

    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let dense = page_text(&mut sh, 120, 40);
    assert!(
        dense.contains("gpu mem") && dense.contains("pcie"),
        "6x3 dense at 120x40 must keep the gpu table (59×19): {dense}"
    );

    // Zoom the gpu tile (top-right 6x3) with a double click: the `full` tier
    // adds USER and the Power placeholder, and every joined column is there.
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let _ = page_text(&mut sh, 250, 70);
    let click = || {
        InputEvent::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            x: 180,
            y: 5,
            mods: Mods::NONE,
        })
    };
    sh.handle_input(click());
    let _ = page_text(&mut sh, 250, 70);
    sh.handle_input(click());
    let zoomed = page_text(&mut sh, 250, 70);
    assert!(
        zoomed.contains("user") && zoomed.contains("host mem") && zoomed.contains("command"),
        "zoomed gpu tile lacks the joined columns: {zoomed}"
    );
    let game = zoomed
        .lines()
        .find(|l| l.contains("412345"))
        .expect("the game's row");
    assert!(
        game.contains("mattbeam") && game.contains("12.5gib") && game.contains("/opt/game"),
        "the game's row: {game}"
    );
    assert!(zoomed.contains("power"), "the Power sub-panel placeholder");
    assert!(
        !zoomed.contains("ccd0"),
        "zoom shows one tile, not the cpu tile too"
    );
}

/// Arc 3a acceptance (brief seam 5): the synth's scripted overload puts the
/// red banner on **every page**, `a` acknowledges it, a new `Raised` brings
/// it back, a Warn-only state shows no banner, and the banner pulses on the
/// even seconds — one row changes between two frames a second apart.
#[test]
fn the_alert_banner_is_on_every_page_and_acknowledges() {
    use gridwatch_store::{AlertEvent, AlertId, Severity, Transition};
    // 20 synth ticks = 30 s: the overload raised at 21.5 s and is active
    // (it resolves at 50 s, which 40 ticks would already have passed).
    let mut sh = shell_with_ticks(config::DEFAULT_LAYOUT, 1, 20);
    let big = page_text(&mut sh, 250, 70);
    assert!(big.contains("⚠ alert: overload ⚠"), "page 1 banner: {big}");
    assert!(big.contains("a to acknowledge"));
    sh.set_page(1);
    let audio = page_text(&mut sh, 250, 70);
    assert!(
        audio.contains("⚠ alert: overload ⚠"),
        "page 2 banner: {audio}"
    );
    // The pulse: even second reversed, odd second plain — the row changes.
    sh.set_clock(Ts(60_000_000_000));
    let even = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    sh.set_clock(Ts(61_000_000_000));
    let odd_frame = shot_frame(&mut sh, 250, 70);
    let odd = gridwatch_ui::dump::cells(&odd_frame);
    assert_ne!(even, odd, "the banner must pulse between seconds");
    // Exactly one row (the banner) differs between the two frames.
    sh.set_clock(Ts(60_000_000_000));
    let even_frame = shot_frame(&mut sh, 250, 70);
    let differing = (0..70u16)
        .filter(|y| (0..250u16).any(|x| even_frame.cell((x, *y)) != odd_frame.cell((x, *y))))
        .count();
    assert_eq!(differing, 1, "the pulse changes one row, not the page");
    // `a` acknowledges: the banner leaves, the body grows back by one row.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('a'))));
    let acked = page_text(&mut sh, 250, 70);
    assert!(!acked.contains("⚠ alert"), "acked: {acked}");
    // A new Raised un-acks.
    sh.apply_control(ControlMsg::Alert(AlertEvent {
        id: AlertId::new("pins/overload"),
        source: SourceId("pins"),
        severity: Severity::Crit,
        transition: Transition::Raised,
        title: "OVERLOAD".into(),
        detail: "OVERLOAD pins 1+2 >9.2A".into(),
        at: Ts(62_000_000_000),
    }));
    let back = page_text(&mut sh, 250, 70);
    assert!(back.contains("⚠ alert: overload ⚠"), "re-raised: {back}");
    // Resolved: the banner leaves and a green toast says so.
    sh.apply_control(ControlMsg::Alert(AlertEvent {
        id: AlertId::new("pins/overload"),
        source: SourceId("pins"),
        severity: Severity::Crit,
        transition: Transition::Resolved,
        title: "OVERLOAD".into(),
        detail: "clear after 40s".into(),
        at: Ts(63_000_000_000),
    }));
    let clear = page_text(&mut sh, 250, 70);
    assert!(!clear.contains("⚠ alert"), "{clear}");
    assert!(clear.contains("✓ overload clear after 40s"), "{clear}");
    // Warn-only: a chip in the key bar, no banner.
    sh.apply_control(ControlMsg::Alert(AlertEvent {
        id: AlertId::new("pins/imbalance_advisory"),
        source: SourceId("pins"),
        severity: Severity::Warn,
        transition: Transition::Raised,
        title: "IMBALANCE (ADVISORY)".into(),
        detail: "IMBALANCE(advisory) hi/lo=1.54".into(),
        at: Ts(64_000_000_000),
    }));
    let warn = page_text(&mut sh, 250, 70);
    assert!(!warn.contains("⚠ alert"), "{warn}");
    assert!(warn.contains("▲ 1 advisory"), "{warn}");
    // `A` opens the alerts overlay with the active list and the log; Esc closes.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('A'))));
    let overlay = page_text(&mut sh, 250, 70);
    assert!(overlay.contains("alerts  ·  esc to close"), "{overlay}");
    assert!(overlay.contains("resolved overload"), "{overlay}");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    assert!(!page_text(&mut sh, 250, 70).contains("esc to close"));
}

/// `Command::Source` without a live source (demo, shot, tests) toasts instead
/// of vanishing; `Command::Ack` hides exactly the acknowledged id.
#[test]
fn source_commands_without_a_live_source_are_explained() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    // Focus the pins tile (third placement) and capture, then press `+`.
    for _ in 0..2 {
        sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Tab)));
    }
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Enter)));
    // `-` = slower: 500 → 600 ms is a real command (`+` at the floor is not).
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('-'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("no live source to control"),
        "the toast explains the missing control: {text}"
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

/// §4.6: a placement naming a tier that does not exist is a config warning,
/// and the tile falls back to the richest fitting tier.
#[test]
fn an_unknown_view_name_warns_and_falls_back() {
    let layout = config::DEFAULT_LAYOUT.replace("view = \"meters\"", "view = \"nonsense\"");
    let mut sh = shell_with(&layout, 2);
    assert!(
        sh.view_warnings().iter().any(|w| w.contains("nonsense")),
        "no warning for the unknown view name: {:?}",
        sh.view_warnings()
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("ccd0"),
        "the fallback must be the richest fitting tier"
    );
    // And `view = "table"` — the arc-1b warning case — now resolves silently.
    let layout = config::DEFAULT_LAYOUT.replace("view = \"meters\"", "view = \"table\"");
    let mut sh = shell_with(&layout, 2);
    assert!(sh.view_warnings().is_empty(), "{:?}", sh.view_warnings());
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("time+"), "the pinned table tier is drawn");
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

// ───────────────────────────── D46 layer B ──────────────────────────────

/// One frame's plain characters plus what the lint needs to know about it.
struct Frame {
    text: String,
    mode: gridwatch_ui::layout::SolveMode,
}

fn frame_at(sh: &mut Shell, w: u16, h: u16) -> Frame {
    let text = page_text(sh, w, h);
    Frame {
        text,
        mode: sh.mode(),
    }
}

/// The D46 frame lint (TESTING.md layer B): what every frame at every size
/// must satisfy. A blank frame is never acceptable; below the shell's minimum
/// the too-small notice is the whole frame; above it, the cpu placement is
/// either drawn or explained by a chip, and the tab bar follows the mode.
fn frame_lint(f: &Frame, w: u16, h: u16, page: usize) {
    use gridwatch_ui::layout::SolveMode;
    let t = &f.text;
    assert!(
        t.chars().any(|c| !c.is_whitespace()),
        "blank frame at {w}x{h} page {page}"
    );
    // The shell's minimum: one tile (8×3 inner + border) plus two chrome rows.
    let too_small = w < 10 || h < 7;
    // The notice is ~40 cells; narrower than that it is truncated to its
    // prefix (the size), which is still an explanation and still non-blank.
    if w >= 45 {
        assert_eq!(
            t.contains("needs at least"),
            too_small,
            "too-small notice wrong at {w}x{h}: {t:?}"
        );
    } else {
        assert!(
            !t.contains("needs at least") || too_small,
            "too-small notice shown at a drawable size {w}x{h}"
        );
    }
    if too_small {
        return;
    }
    // The mode carries hysteresis (§6), so ask the shell which one it drew in
    // rather than recomputing it from the size alone.
    let mode = f.mode;
    // The cpu placement exists on both shipped pages: either its content (any
    // tier prints a percentage or a dash) or a chip that says why not. In
    // stack mode the body may hold only the *first* placement, so the rule
    // there is "some placement is drawn or explained".
    let cpu_drawn = t.contains('%') || t.contains('—') || t.contains("▪ cpu");
    let any_drawn = cpu_drawn || t.contains("▪ ");
    if mode == SolveMode::Stack {
        assert!(
            any_drawn,
            "no placement drawn or explained at {w}x{h} page {page}:\n{t}"
        );
    } else {
        assert!(
            cpu_drawn,
            "cpu tile neither drawn nor explained at {w}x{h} page {page}:\n{t}"
        );
    }
    assert_eq!(
        t.contains("gridwatch"),
        mode != SolveMode::Dense,
        "tab bar visibility wrong at {w}x{h} (mode {mode:?})"
    );
}

/// Every size a terminal can plausibly be, both pages, all themes at the §6
/// thresholds and the full lattice in one theme.
#[test]
fn every_size_renders_something_honest() {
    let mut widths: Vec<u16> = (1..=300).step_by(7).collect();
    widths.extend([19, 20, 21, 108, 109, 110, 130, 131, 132, 157, 158, 250, 300]);
    let mut heights: Vec<u16> = (1..=80).step_by(3).collect();
    heights.extend([2, 3, 4, 26, 27, 28, 36, 37, 38, 40, 70, 80]);
    widths.sort_unstable();
    widths.dedup();
    heights.sort_unstable();
    heights.dedup();
    let thresholds = [
        (19, 2),
        (20, 3),
        (108, 26),
        (109, 27),
        (130, 36),
        (131, 37),
        (158, 40),
    ];
    for page in [1usize, 2] {
        let mut sh = shell_with(config::DEFAULT_LAYOUT, page);
        for &w in &widths {
            for &h in &heights {
                let f = frame_at(&mut sh, w, h);
                frame_lint(&f, w, h, page);
            }
        }
        for theme in ["retrowave", "modern"] {
            let mut sh = shell_with_theme(config::DEFAULT_LAYOUT, page, theme);
            for (w, h) in thresholds {
                let f = frame_at(&mut sh, w, h);
                frame_lint(&f, w, h, page);
            }
        }
    }
}

/// Resize as a sequence through one shell, not a fresh shell per size: the
/// mode, the cpu tile's tier and the notice must follow the terminal, and the
/// frame after each resize must be byte-identical to a cold start at that
/// size — which is what "no stale cells, cache invalidated" actually means.
#[test]
fn resize_sequence_follows_the_terminal() {
    let steps: [(u16, u16, &str); 6] = [
        (60, 20, "stack"),
        (200, 45, "cores"),
        (40, 8, "stack"),
        (250, 50, "cores"),
        (158, 1, "too-small"),
        (131, 38, "cores"),
    ];
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let _ = frame_at(&mut sh, 250, 70);
    for (w, h, expect) in steps {
        sh.handle_input(InputEvent::Resize(w, h));
        let f = frame_at(&mut sh, w, h);
        match expect {
            "cores" => assert!(
                f.text.contains("ccd0"),
                "{w}x{h} should be cores:\n{}",
                f.text
            ),
            "stack" => assert!(
                !f.text.contains("ccd0") && f.text.contains("cpu"),
                "{w}x{h} should be a stacked/small cpu tile:\n{}",
                f.text
            ),
            _ => assert!(
                f.text.contains("158×1 — gridwatch needs"),
                "{w}x{h}: {}",
                f.text
            ),
        }
        let mut cold = shell_with(config::DEFAULT_LAYOUT, 1);
        let cold_f = frame_at(&mut cold, w, h);
        assert_eq!(
            f.text, cold_f.text,
            "frame after resizing to {w}x{h} differs from a cold start at that size"
        );
    }
}

fn shell_with_theme(layout: &str, page: usize, theme: &str) -> Shell {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let loaded = config::load_texts(config::DEFAULT_CONFIG, layout).unwrap();
    let theme = load_builtin(theme, ColorMode::TrueColor).unwrap();
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    sh.set_page(page.saturating_sub(1));
    gridwatch_app::feed_synth(&mut sh, 42, 40);
    sh
}

// ───────────────────────────── D46 layer D ──────────────────────────────

/// §11/D46: a source entering `Unavailable` is a failure the user must see,
/// not one they find in the log — it toasts once, with the reason, and the
/// `sources` tile carries the state.
#[test]
fn a_source_failure_reaches_the_screen() {
    let mut sh = shell();
    sh.store.ensure_source(SourceId("cpu"));
    let status = |state| SourceStatus {
        state,
        reason: Some("procfs must be mounted at /proc".into()),
        hint: None,
        since: Ts(1),
        last_sample: None,
        dropped: 0,
        restarts: 0,
    };
    sh.apply_control(ControlMsg::Status(
        SourceId("cpu"),
        status(SourceState::Unavailable),
    ));
    let t = page_text(&mut sh, 250, 70);
    assert!(
        t.contains("cpu unavailable: procfs must be mounted"),
        "no toast for the failure:\n{t}"
    );
    assert!(
        t.contains("unavailable"),
        "sources tile does not show the state"
    );
    // The same status again is not a new transition: one toast, not a stream.
    sh.apply_control(ControlMsg::Status(
        SourceId("cpu"),
        status(SourceState::Unavailable),
    ));
    let t = page_text(&mut sh, 250, 70);
    assert_eq!(
        t.matches("cpu unavailable:").count(),
        1,
        "duplicate toasts:\n{t}"
    );
}

// ───────────────────────────── arc 3b (seams 8–10) ─────────────────────────────

/// The cells of a rect as plain text — one tile's content, so a toast at the
/// bottom right of the body does not enter the comparison.
fn region_text(sh: &mut Shell, w: u16, h: u16, r: ratatui::layout::Rect) -> String {
    let frame = shot_frame(sh, w, h);
    let mut out = String::new();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            out.push_str(frame.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
        }
        out.push('\n');
    }
    out
}

/// Seam 8: a reload keeps an instance whose `(kind, options)` did not change
/// — its state (here htop's inverted sort) survives — and rebuilds one whose
/// options changed; a broken file keeps everything and toasts the line.
#[test]
fn reload_keeps_unchanged_instances_and_toasts_a_broken_file() {
    use gridwatch_store::ReloadKind;
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let cpu = ratatui::layout::Rect::new(0, 1, 120, 30);
    let before = region_text(&mut sh, 250, 70, cpu);
    // Capture the cpu tile and invert its sort: component state.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Enter)));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('I'))));
    let inverted = region_text(&mut sh, 250, 70, cpu);
    assert_ne!(before, inverted, "`I` did not change the table");
    // Same files again: 1 config.toml reload, every instance kept.
    sh.reload_from_texts(
        ReloadKind::Config,
        config::DEFAULT_CONFIG,
        config::DEFAULT_LAYOUT,
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("config.toml reloaded"), "{text}");
    assert!(text.contains("kept"), "{text}");
    assert!(!text.contains("rebuilt"), "{text}");
    assert_eq!(
        region_text(&mut sh, 250, 70, cpu),
        inverted,
        "an unchanged instance lost its state"
    );
    // Changed options for `cpu`: that one is rebuilt (default sort again).
    let changed = config::DEFAULT_CONFIG.replace(
        "id = \"cpu\"\nkind = \"htop\"",
        "id = \"cpu\"\nkind = \"htop\"\noptions = { table_rows = 5 }",
    );
    assert_ne!(changed, config::DEFAULT_CONFIG);
    sh.reload_from_texts(ReloadKind::Config, &changed, config::DEFAULT_LAYOUT);
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("1 rebuilt"), "{text}");
    assert_ne!(
        region_text(&mut sh, 250, 70, cpu),
        inverted,
        "a changed instance was not rebuilt"
    );
    let rebuilt = region_text(&mut sh, 250, 70, cpu);
    // A broken config.toml: nothing changes, the toast names file and line.
    sh.reload_from_texts(
        ReloadKind::Config,
        "schema = 1\nfps = \"thirty\"\n",
        config::DEFAULT_LAYOUT,
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("kept the old config"), "{text}");
    assert!(text.contains("config.toml:2:"), "{text}");
    assert_eq!(region_text(&mut sh, 250, 70, cpu), rebuilt);
    // A layout with two pages of which the current one vanished: page 0.
    sh.set_page(1);
    let one_page = "schema = 1\n[grid]\ncolumns = 12\nrows = 6\n[[pages]]\nname = \"Only\"\nhotkey = \"1\"\nplace = [{ id = \"cpu\", at = [0, 0], size = [12, 6] }]\n";
    sh.reload_from_texts(ReloadKind::Layout, &changed, one_page);
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("layout.toml reloaded (1 pages"), "{text}");
    assert!(text.contains(" 1 only "), "{text}");
}

/// Seam 8: `T` reloads the theme; a config reload that names another theme
/// swaps it unless the CLI locked the theme.
#[test]
fn theme_reload_follows_the_config_unless_locked() {
    use gridwatch_store::ReloadKind;
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    assert_eq!(sh.theme().name, "mono");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('T'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("theme reloaded: mono"), "{text}");
    let modern = config::DEFAULT_CONFIG.replace("theme = \"retrowave\"", "theme = \"modern\"");
    sh.reload_from_texts(ReloadKind::Config, &modern, config::DEFAULT_LAYOUT);
    assert_eq!(
        sh.theme().name,
        "modern",
        "the config's theme was not followed"
    );
    assert_eq!(sh.theme_ref(), "modern");
    sh.theme_locked = true;
    let phos =
        config::DEFAULT_CONFIG.replace("theme = \"retrowave\"", "theme = \"phosphor-green\"");
    sh.reload_from_texts(ReloadKind::Config, &phos, config::DEFAULT_LAYOUT);
    assert_eq!(
        sh.theme().name,
        "modern",
        "a locked theme followed the config"
    );
    // `t` cycles through every built-in and moves the reload target with it.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('t'))));
    assert_eq!(sh.theme().name, "mono");
    assert_eq!(sh.theme_ref(), "mono");
    // A theme file that does not exist keeps the old theme and says so.
    sh.set_theme_ref("/nonexistent/theme.toml");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('T'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("kept the old theme"), "{text}");
    assert_eq!(sh.theme().name, "mono");
}

/// Seam 9: a `[components.<kind>]` override reaches the rendered tile — the
/// frame under the overriding theme differs from the base and carries the
/// override colour — and no other tile changes.
#[test]
fn component_gradient_override_reaches_the_tile() {
    let modern = include_str!("../../../themes/modern.toml");
    let over =
        format!("{modern}\n[components.htop]\ngradients.load = [\"#123456\", \"#123456\"]\n");
    let dir = std::env::temp_dir().join(format!("gridwatch-theme-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("over.toml");
    std::fs::write(&path, over).unwrap();
    let theme =
        gridwatch_app::load_theme_by_name(path.to_str().unwrap(), ColorMode::TrueColor).unwrap();
    let mut base = shell_with_theme(config::DEFAULT_LAYOUT, 1, "modern");
    let mut sh = shell_with_theme(config::DEFAULT_LAYOUT, 1, "modern");
    sh.swap_theme(theme);
    let a = gridwatch_ui::dump::cells(&shot_frame(&mut base, 250, 70));
    let b = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_ne!(a, b);
    // The LUT's own value (Oklab round-trips within a step of #123456).
    let ratatui::style::Color::Rgb(r, g, bl) = sh
        .theme()
        .for_kind("htop")
        .gradient(gridwatch_ui::GradientId::Load)
        .sample(0.5)
    else {
        panic!("rgb")
    };
    let hex = format!("#{r:02x}{g:02x}{bl:02x}");
    assert!(
        b.contains(&hex),
        "the override colour {hex} is not on screen"
    );
    assert!(!a.contains(&hex));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A registry with the real sources so the shell knows their cadences.
fn shell_with_sources(caps: gridwatch_store::CapSet) -> Shell {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    let loaded = config::load_embedded().unwrap();
    let theme = load_builtin("mono", ColorMode::Mono).unwrap();
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        caps,
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    gridwatch_app::feed_synth(&mut sh, 42, 40);
    sh
}

/// Seam 10: a tile whose source aged past 3 × its visible cadence on the
/// virtual clock is dimmed and badged `STALE Ns`; fresh data and a deliberate
/// pause show no badge.
#[test]
fn stale_sources_are_badged_after_three_cadences() {
    let mut sh = shell_with_sources(probe::probe());
    // 40 ticks at 1.5 s = 60 s of data; the clock sits at the last sample.
    sh.set_clock(Ts(60_000_000_000));
    let fresh = page_text(&mut sh, 250, 70);
    assert!(!fresh.contains("stale"), "{fresh}");
    // 4 s later: the cpu source (1.5 s visible → 4.5 s) is not stale yet;
    // the gpu (0.5 s → 1.5 s), pins (0.5 s → 1.5 s), audio (33 ms, or 1 s
    // while silent → 3 s; arc 5a) and sensors (1 s → 3 s; arc 5b) are.
    sh.set_clock(Ts(64_000_000_000));
    let partly = page_text(&mut sh, 250, 70);
    assert_eq!(partly.matches("stale 4s").count(), 4, "{partly}");
    sh.set_clock(Ts(70_000_000_000));
    let all = page_text(&mut sh, 250, 70);
    assert_eq!(all.matches("stale 10s").count(), 5, "{all}");
    // Dimmed: the cpu tile's cells are drawn in the muted role — in mono
    // that is `Reset` either way, so check the badge style instead.
    let frame = shot_frame(&mut sh, 250, 70);
    let badge = (0..250u16)
        .flat_map(|x| (0..70u16).map(move |y| (x, y)))
        .find(|&(x, y)| {
            "STALE".chars().enumerate().all(|(i, ch)| {
                frame
                    .cell((x + i as u16, y))
                    .is_some_and(|c| c.symbol() == ch.to_string())
            })
        })
        .expect("a STALE badge");
    assert!(
        frame
            .cell(badge)
            .unwrap()
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    // Paused: no badge — the pause is deliberate and the key bar says so.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    let paused = page_text(&mut sh, 250, 70);
    assert!(!paused.contains("stale"), "{paused}");
}

/// Seam 10: a component whose required capability is missing is a chip that
/// says the reason **and the fix**, in the doctor's words.
#[test]
fn a_missing_required_capability_chips_reason_and_fix() {
    let mut sh = shell_with_sources(gridwatch_store::CapSet::empty());
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("needs procfs: /proc is not mounted"),
        "{text}"
    );
    assert!(text.contains("fix: mount procfs at /proc"), "{text}");
    // With the capability the same layout builds the tile.
    let mut ok = shell_with_sources(probe::probe());
    let text = page_text(&mut ok, 250, 70);
    assert!(!text.contains("needs procfs"), "{text}");
}

/// Review (arc 3b): the staleness threshold follows the cadence a source is
/// configured to run at, never below its focused cadence; the badge sits on
/// the frame's top border, so the tile's own top-right value survives; a
/// source that is not `Ok` is not badged (its status says why already).
#[test]
fn stale_threshold_follows_the_configured_cadence() {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    let cfg = format!(
        "{}\n[sources.gpu]\nrefresh_ms = 4000\n",
        config::DEFAULT_CONFIG
    );
    let loaded = config::load_texts(&cfg, config::DEFAULT_LAYOUT).unwrap();
    let theme = load_builtin("mono", ColorMode::Mono).unwrap();
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    gridwatch_app::feed_synth(&mut sh, 42, 40);
    // 4 s past the last sample: gpu (4 s → 12 s threshold) is fresh, pins
    // (0.5 s → 1.5 s), audio (1 s while silent → 3 s) and sensors (1 s →
    // 3 s) are stale, cpu (1.5 s → 4.5 s) not yet.
    sh.set_clock(Ts(64_000_000_000));
    let text = page_text(&mut sh, 250, 70);
    assert_eq!(text.matches("stale 4s").count(), 3, "{text}");
    // The badge is on the border row (row 1 of the pins tile's frame), and
    // no data row lost its right end: the pins tile's top-right value.
    let frame = shot_frame(&mut sh, 250, 70);
    let row_of_badge = (0..70u16)
        .find(|y| {
            let line: String = (0..250u16)
                .map(|x| frame.cell((x, *y)).unwrap().symbol().to_string())
                .collect();
            line.contains("STALE 4s")
        })
        .expect("badge row");
    let line: String = (0..250u16)
        .map(|x| frame.cell((x, row_of_badge)).unwrap().symbol().to_string())
        .collect();
    assert!(
        line.contains("━") || line.contains("─"),
        "not a border row: {line}"
    );
    // A source that is Degraded is not badged, whatever its age.
    sh.apply_control(ControlMsg::Status(
        SourceId("pins"),
        SourceStatus {
            state: SourceState::Degraded,
            reason: Some("waiting for telemetry (GPU idle?)".into()),
            hint: None,
            since: Ts(1),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        },
    ));
    let text = page_text(&mut sh, 250, 70);
    // The audio and sensors tiles' badges remain (arc 5).
    assert_eq!(text.matches("stale").count(), 2, "{text}");
    assert!(
        !text.contains("pins ─stale") && !text.contains("pins ━stale"),
        "{text}"
    );
}

/// Review (arc 3b): under `--replay` the journal drives the virtual clock and
/// stops with it, so a finished replay counted no age at all; the badge now
/// counts real time from the journal's `Stopped`.
#[test]
fn a_finished_replay_goes_stale_in_real_time() {
    let mut sh = shell_with_sources(probe::probe());
    sh.age_after_journal = true; // as `run --replay` sets it; never in the harness
    sh.set_clock(Ts(60_000_000_000));
    assert!(!page_text(&mut sh, 250, 70).contains("stale"));
    sh.apply_control(ControlMsg::Status(
        gridwatch_store::JOURNAL,
        SourceStatus {
            state: SourceState::Stopped,
            reason: Some("end of journal".into()),
            hint: None,
            since: Ts(60_000_000_000),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        },
    ));
    std::thread::sleep(std::time::Duration::from_millis(1700));
    let text = page_text(&mut sh, 250, 70);
    // gpu, pins and audio (1.5 s thresholds) are stale after 1.7 s of real time.
    assert_eq!(text.matches("stale 1s").count(), 3, "{text}");
}

/// Review (arc 3b): resuming from a pause does not flash STALE while the
/// parked sources wait for their next grid tick.
#[test]
fn resuming_from_pause_does_not_flash_stale() {
    let mut sh = shell_with_sources(probe::probe());
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    sh.set_clock(Ts(80_000_000_000));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    let text = page_text(&mut sh, 250, 70);
    assert!(!text.contains("stale"), "{text}");
    // A sample stamped after the resume restores the rule for its source.
    let mut gpu = gridwatch_store::demo::GpuSynth::new(42);
    sh.store.apply(&Msg::Batch(
        gpu.tick_at(Ts(85_000_000_000), gridwatch_store::Detail::Meters),
    ));
    sh.set_clock(Ts(90_000_000_000));
    let text = page_text(&mut sh, 250, 70);
    assert_eq!(text.matches("stale 5s").count(), 1, "{text}");
}

// ───────────────────────────── arc 4a: edit mode ─────────────────────────────

/// Assert two dumps equal, naming the first differing line (a whole-frame
/// diff is unreadable in a panic message).
fn assert_same(a: &str, b: &str, what: &str) {
    if a == b {
        return;
    }
    let diff = a
        .lines()
        .zip(b.lines())
        .enumerate()
        .find(|(_, (x, y))| x != y)
        .map(|(i, (x, y))| format!("line {i}:\n  {x}\n  {y}"))
        .unwrap_or_else(|| {
            format!(
                "different line counts ({} vs {})",
                a.lines().count(),
                b.lines().count()
            )
        });
    panic!("{what}: {diff}");
}

fn key(c: char) -> InputEvent {
    InputEvent::Key(KeyEvent::ch(c))
}

fn ctrl(c: char) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code: KeyCode::Char(c),
        mods: gridwatch_store::Mods::CTRL,
    })
}

/// The text of the key bar (the last row).
fn key_bar(sh: &mut Shell) -> String {
    let frame = shot_frame(sh, 250, 70);
    (0..250u16)
        .map(|x| frame.cell((x, 69)).unwrap().symbol().to_string())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Seam 1: every key path of edit mode — move, resize, footprint, swap,
/// remove, undo/redo — renders the expected geometry; a collision leaves the
/// page unchanged and draws the red ghost; leaving a dirty page asks first.
#[test]
fn edit_keys_move_resize_swap_remove_and_undo() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let before_tiles = region_text(&mut sh, 250, 70, ratatui::layout::Rect::new(0, 0, 250, 60));
    sh.handle_input(key('e'));
    assert!(sh.editing());
    let bar = key_bar(&mut sh);
    assert!(bar.starts_with("EDIT · cpu @ (0,0) 6×3"), "{bar}");
    // The dotted unit grid is in the gutters.
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("·\n") || text.contains("· "),
        "no dotted grid"
    );
    // `L` on the cpu tile (0,0 6x3) collides with gpu (6,0): refused, ghost.
    sh.handle_input(key('L'));
    let bar = key_bar(&mut sh);
    assert!(bar.starts_with("▲ would overlap another tile"), "{bar}");
    assert!(bar.contains("cpu @ (0,0) 6×3"), "unchanged: {bar}");
    let frame = shot_frame(&mut sh, 250, 70);
    let crit = sh.theme().color(gridwatch_ui::Role::Crit);
    let ghost_cells = frame
        .content()
        .iter()
        .filter(|c| c.symbol() == "╔" && c.fg == crit)
        .count();
    assert_eq!(ghost_cells, 1, "one red double-bordered ghost");
    // Narrow it, then it moves right.
    sh.handle_input(ctrl('h'));
    assert!(
        key_bar(&mut sh).contains("cpu @ (0,0) 5×3"),
        "{}",
        key_bar(&mut sh)
    );
    sh.handle_input(key('L'));
    assert!(
        key_bar(&mut sh).contains("cpu @ (1,0) 5×3"),
        "{}",
        key_bar(&mut sh)
    );
    // Ctrl-Backspace narrows too; the plain Backspace key does nothing.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Backspace)));
    assert!(key_bar(&mut sh).contains("cpu @ (1,0) 5×3"));
    sh.handle_input(InputEvent::Key(KeyEvent {
        code: KeyCode::Backspace,
        mods: gridwatch_store::Mods::CTRL,
    }));
    assert!(key_bar(&mut sh).contains("cpu @ (1,0) 4×3"));
    // `K` at the top is out of the grid.
    sh.handle_input(key('K'));
    assert!(key_bar(&mut sh).starts_with("▲ outside the grid"));
    // Footprint cycle: htop's manifest lists footprints; `s` picks the next.
    sh.handle_input(key('s'));
    let bar = key_bar(&mut sh);
    assert!(!bar.contains("4×3") || bar.starts_with("▲"), "{bar}");
    // Undo everything back to the start, then redo one.
    for _ in 0..8 {
        sh.handle_input(key('u'));
    }
    assert!(key_bar(&mut sh).contains("nothing to undo"));
    assert!(key_bar(&mut sh).contains("cpu @ (0,0) 6×3"));
    sh.handle_input(ctrl('r'));
    assert!(key_bar(&mut sh).contains("cpu @ (0,0) 5×3"));
    // Swap with the neighbour to the right (gpu at 6,0 6x3 — a different size
    // is fine: swap exchanges geometry).
    sh.handle_input(key('u'));
    sh.handle_input(key('S'));
    assert!(key_bar(&mut sh).starts_with("EDIT · swap with"));
    sh.handle_input(key('l'));
    assert!(
        key_bar(&mut sh).contains("cpu @ (6,0) 6×3"),
        "{}",
        key_bar(&mut sh)
    );
    // Remove the focused tile: focus clamps, the page shrinks.
    sh.handle_input(key('x'));
    assert!(!key_bar(&mut sh).contains("cpu @"), "{}", key_bar(&mut sh));
    // Esc on a dirty page asks; Esc again stays; `y` discards and leaves.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    assert!(key_bar(&mut sh).starts_with("unsaved changes"));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    assert!(sh.editing());
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    sh.handle_input(key('y'));
    assert!(!sh.editing());
    // The "edit mode" toast is still live on the bottom rows: compare the
    // tile rows.
    let tiles = ratatui::layout::Rect::new(0, 0, 250, 60);
    let after = region_text(&mut sh, 250, 70, tiles);
    assert_same(&before_tiles, &after, "discard did not restore the page");
}

/// Seam 2 + 5: the picker adds a `kind:` tile at first fit; `w` writes
/// `layout.toml` that reloads to the same pages, with the file's comments
/// intact and the watcher's hash registered.
#[test]
fn picker_adds_a_tile_and_save_round_trips_through_the_file() {
    let dir = std::env::temp_dir().join(format!("gridwatch-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("layout.toml");
    let commented = format!("# hand-written\n{}", config::DEFAULT_LAYOUT);
    std::fs::write(&path, &commented).unwrap();
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    sh.watch_ignore = Some(tx);
    sh.handle_input(key('e'));
    // The Overview is full (72 of 72 units): remove the focused cpu tile so
    // there is room, then add a `kind:sources` tile through the picker.
    sh.handle_input(key('x'));
    sh.handle_input(key('a'));
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("add a tile"), "{text}");
    assert!(text.contains("kind:clock"), "{text}");
    for c in "kind:sou".chars() {
        sh.handle_input(key(c));
    }
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("▶ kind:sources"), "{text}");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Enter)));
    let bar = key_bar(&mut sh);
    assert!(bar.contains("sources @"), "the new tile is focused: {bar}");
    // Save to the temp path: comments survive, the hash reaches the watcher.
    let msg = sh.save_layout_to(&path).unwrap();
    assert_eq!(msg, "layout.toml saved (2 pages)");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("# hand-written"), "{written}");
    assert!(written.contains("kind = \"sources\""), "{written}");
    let (kind, hash) = rx.try_recv().unwrap();
    assert_eq!(kind, gridwatch_store::ReloadKind::Layout);
    assert_eq!(hash, gridwatch_app::watch::content_hash(written.as_bytes()));
    // The saved text reloads to exactly the edited pages. Toasts sit on the
    // bottom body rows, so the tile rows are what is compared, both times
    // outside edit mode (the dotted grid is edit chrome).
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    assert!(!sh.editing(), "a saved page leaves without asking");
    let tiles = ratatui::layout::Rect::new(0, 0, 250, 60);
    let a = region_text(&mut sh, 250, 70, tiles);
    sh.reload_from_texts(
        gridwatch_store::ReloadKind::Layout,
        config::DEFAULT_CONFIG,
        &written,
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("layout.toml reloaded"), "{text}");
    let b = region_text(&mut sh, 250, 70, tiles);
    assert_same(&a, &b, "saved layout does not reload identically");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Seam 3: a mouse drag moves by the unit delta with a green/red ghost and
/// applies on release; a press on the bottom-right corner resizes.
#[test]
fn mouse_drag_moves_and_corner_resizes() {
    use gridwatch_store::{Mods, MouseButton, MouseEvent, MouseKind};
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    sh.handle_input(key('e'));
    let _ = shot_frame(&mut sh, 250, 70);
    let mouse = |kind, x, y| {
        InputEvent::Mouse(MouseEvent {
            kind,
            x,
            y,
            mods: Mods::NONE,
        })
    };
    // The pins tile sits at (0,3) 4x2 → outer x 0..82, y 35..? Drag it right
    // by two units: press inside, drag, release.
    let frame = shot_frame(&mut sh, 250, 70);
    let _ = frame;
    // The pins tile's bottom-right corner, found before any ghost is drawn
    // (a refused drag's ghost paints its edges over neighbouring borders).
    let solved_corner = {
        let frame = shot_frame(&mut sh, 250, 70);
        let mut found = None;
        for y in 41..70u16 {
            for x in 0..90u16 {
                let s = frame.cell((x, y)).unwrap().symbol();
                if (s == "┘" || s == "╝" || s == "╯" || s == "┛") && found.is_none() {
                    found = Some((x, y));
                }
            }
        }
        found.expect("a corner")
    };
    sh.handle_input(mouse(MouseKind::Down(MouseButton::Left), 5, 40));
    let bar = key_bar(&mut sh);
    assert!(bar.contains("pins @ (0,3) 4×2"), "{bar}");
    sh.handle_input(mouse(MouseKind::Drag(MouseButton::Left), 47, 40));
    let text = page_text(&mut sh, 250, 70);
    let _ = text;
    sh.handle_input(mouse(MouseKind::Up(MouseButton::Left), 47, 40));
    let bar = key_bar(&mut sh);
    assert!(bar.starts_with("▲ would overlap"), "lan is at (4,3): {bar}");
    assert!(bar.contains("pins @ (0,3) 4×2"), "{bar}");
    // Drag it down one unit instead — (0,5) is free? amp sits at (0,5) 4x1 in
    // the shipped layout, so that overlaps too; resize from the corner
    // instead: the bottom-right corner of the pins tile.

    sh.handle_input(mouse(
        MouseKind::Down(MouseButton::Left),
        solved_corner.0,
        solved_corner.1,
    ));
    sh.handle_input(mouse(
        MouseKind::Drag(MouseButton::Left),
        solved_corner.0 - 20,
        solved_corner.1,
    ));
    sh.handle_input(mouse(
        MouseKind::Up(MouseButton::Left),
        solved_corner.0 - 20,
        solved_corner.1,
    ));
    let bar = key_bar(&mut sh);
    assert!(
        bar.contains("pins @ (0,3) 3×2"),
        "corner drag narrowed it (corner {solved_corner:?}): {bar}"
    );
    sh.handle_input(key('u'));
    assert!(key_bar(&mut sh).contains("pins @ (0,3) 4×2"));
}

// ───────────────────── arc 4a review: the confirmed findings ─────────────────────

/// A refused key op draws the ghost on the *attempted* rect (review): `L`
/// on cpu (0,0 6×3) into gpu paints the ghost one unit to the right, not
/// over cpu; in mono the Crit ghost is REVERSED so it differs from a fit.
#[test]
fn refused_op_ghost_sits_on_the_attempted_rect() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    sh.handle_input(key('e'));
    let _ = shot_frame(&mut sh, 250, 70);
    sh.handle_input(key('L'));
    let frame = shot_frame(&mut sh, 250, 70);
    let corner = (0..250u16)
        .flat_map(|x| (0..70u16).map(move |y| (x, y)))
        .find(|&(x, y)| frame.cell((x, y)).is_some_and(|c| c.symbol() == "╔"))
        .expect("a ghost");
    // Unit column 1 starts at x = 21 at 250 wide (12 tracks, gap 1); cpu's
    // own corner is at x = 0.
    assert_eq!(corner, (21, 1), "the ghost is not on the attempted rect");
    let c = frame.cell(corner).unwrap();
    assert!(
        c.modifier.contains(ratatui::style::Modifier::REVERSED),
        "mono: Crit = REVERSED"
    );
    // The cpu tile's own frame and title are intact underneath.
    assert_eq!(frame.cell((0, 1)).unwrap().symbol(), "┏");
    let bar = key_bar(&mut sh);
    assert!(bar.starts_with("▲ would overlap another tile"), "{bar}");
    assert!(
        bar.contains("Esc leave"),
        "the way out survives the note: {bar}"
    );
}

/// The dotted grid lives in the gutters only (review): no `·` inside any
/// tile's outer rect; a gutter cell between two tiles is dotted.
#[test]
fn dotted_grid_never_enters_a_tile() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    // cpu occupies x 0..125, y 1..36 at 250x70: its cells are untouched by
    // edit chrome (the tile's own `·` separators are content, not dots).
    let cpu = ratatui::layout::Rect::new(0, 1, 125, 35);
    let before = region_text(&mut sh, 250, 70, cpu);
    sh.handle_input(key('e'));
    let after = region_text(&mut sh, 250, 70, cpu);
    assert_same(&before, &after, "edit chrome entered the cpu tile");
    // The gutter column between cpu and gpu (x = 125) is dotted.
    let frame = shot_frame(&mut sh, 250, 70);
    assert_eq!(frame.cell((125, 10)).unwrap().symbol(), "·");
}

/// A page change on a dirty page is deferred, not refused (review): after
/// `w` or `y` the requested page opens, still in edit mode.
#[test]
fn page_change_while_dirty_is_taken_after_the_answer() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 1);
    sh.handle_input(key('e'));
    sh.handle_input(ctrl('h'));
    sh.handle_input(key('2'));
    assert!(key_bar(&mut sh).starts_with("unsaved changes"));
    sh.handle_input(key('y'));
    assert!(sh.editing(), "still editing on the new page");
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains(" 2 audio "), "{text}");
    // `q` during the prompt still quits.
    sh.handle_input(ctrl('h'));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    sh.handle_input(key('q'));
    assert!(sh.quit);
}

/// A layout reload under edit mode re-baselines the session (review): `y`
/// after the reload restores the *reloaded* page, never another page's.
#[test]
fn reload_under_edit_mode_resets_the_baseline() {
    let mut sh = shell_with(config::DEFAULT_LAYOUT, 2);
    sh.handle_input(key('e'));
    let one_page = "schema = 1\n[grid]\ncolumns = 12\nrows = 6\n[[pages]]\nname = \"Only\"\nhotkey = \"1\"\nplace = [{ id = \"cpu\", at = [0, 0], size = [12, 6] }]\n";
    sh.reload_from_texts(
        gridwatch_store::ReloadKind::Layout,
        config::DEFAULT_CONFIG,
        one_page,
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("reloaded under edit mode"), "{text}");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Esc)));
    assert!(
        !sh.editing(),
        "a clean re-baselined page leaves without asking"
    );
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains(" 1 only "), "{text}");
    assert!(
        !text.contains("winamp"),
        "audio's tiles must not leak: {text}"
    );
}

/// The picker keeps its cursor on screen (review): with more items than the
/// panel has rows, moving past the window scrolls it and says how many more.
#[test]
fn picker_scrolls_with_the_cursor() {
    let mut ids = String::new();
    for i in 0..30 {
        ids.push_str(&format!(
            "[[components]]\nid = \"c{i:02}\"\nkind = \"clock\"\n"
        ));
    }
    let cfg = format!("{}\n{ids}", config::DEFAULT_CONFIG);
    let loaded = config::load_texts(&cfg, config::DEFAULT_LAYOUT).unwrap();
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let theme = load_builtin("mono", ColorMode::Mono).unwrap();
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    sh.handle_input(key('e'));
    sh.handle_input(key('a'));
    for _ in 0..25 {
        sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Down)));
    }
    let text = page_text(&mut sh, 100, 24);
    // `amp` (configured, unplaced) leads the list, so item 25 is c24.
    assert!(text.contains("▶ c24"), "cursor off screen: {text}");
    assert!(text.contains("more above"), "{text}");
    // A filter may start with `j`.
    sh.handle_input(key('j'));
    let text = page_text(&mut sh, 100, 24);
    assert!(text.contains("filter: j"), "{text}");
}

// ───────────────────────────── arc 4b: the rain ─────────────────────────────

/// 20 synth ticks = 30 s: the overload is active (raised 21.5 s, resolved
/// 50 s), so the banner is up for the readability check.
fn matrix_shell() -> Shell {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let loaded = config::load_texts(config::DEFAULT_CONFIG, config::DEFAULT_LAYOUT).unwrap();
    let mut theme = load_builtin("matrix", ColorMode::TrueColor).unwrap();
    // Determinism under load: the governor reads wall-clock frame costs and
    // the effects watchdog trips on a wall-clock budget — on a slow CI runner
    // either would step one shell and not another. Both are exercised by
    // their own tests; here the layer must be a pure function of the inputs.
    if let Some(a) = theme.ambient.as_mut() {
        a.governor = false;
    }
    let mut sh = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    gridwatch_app::feed_synth(&mut sh, 42, 20);
    sh.set_effects(true, 1_000);
    sh
}

fn katakana_cells(frame: &ratatui::buffer::Buffer) -> usize {
    frame
        .content()
        .iter()
        .filter(|c| {
            c.symbol()
                .chars()
                .any(|ch| ('\u{FF66}'..='\u{FF9D}').contains(&ch))
        })
        .count()
}

/// The rain draws under `matrix` (seam 10): after a few frames katakana are
/// on screen, the key bar row is pinned (its text intact), and the banner
/// text survives every frame of a sweep cycle while the overload is active.
#[test]
fn matrix_rain_falls_and_the_pins_stay_readable() {
    let mut sh = matrix_shell();
    let mut seen_rain = false;
    let mut bar_ok = true;
    // 20 ticks of synth = 30 s: the overload is active from 21.5 s.
    for _ in 0..24 {
        let frame = shot_frame(&mut sh, 250, 70);
        if katakana_cells(&frame) > 0 {
            seen_rain = true;
        }
        let bar: String = (0..250u16)
            .map(|x| frame.cell((x, 69)).unwrap().symbol().to_string())
            .collect();
        if !bar.contains("q quit") {
            bar_ok = false;
        }
    }
    assert!(seen_rain, "no rain glyph in 24 frames");
    assert!(bar_ok, "the key bar was rained on");
    // The banner: every frame of a whole sweep cycle (20 s at 24 fps = 480
    // frames) carries the alert text, unrained.
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("⚠ alert: overload ⚠"), "{text}");
    for _ in 0..500 {
        let frame = shot_frame(&mut sh, 250, 70);
        let banner: String = (0..250u16)
            .map(|x| frame.cell((x, 1)).unwrap().symbol().to_string())
            .collect();
        assert!(
            banner.contains("ALERT: OVERLOAD"),
            "banner rained on: {banner}"
        );
    }
    // The pins tile (the alerting source's tile) is pinned: its title stays.
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("balance"),
        "the alerting tile is not lit: {text}"
    );
}

/// Pause freezes the layer (two frames byte-identical) and unpausing resumes;
/// `V` lights every content cell (the whole page reads); `L` keeps it lit.
#[test]
fn pause_freezes_and_v_and_l_light_the_page() {
    let mut sh = matrix_shell();
    for _ in 0..30 {
        let _ = shot_frame(&mut sh, 250, 70);
    }
    // Let the startup fade finish before comparing frames.
    std::thread::sleep(std::time::Duration::from_millis(700));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    let a = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    let b = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_eq!(a, b, "the rain moved while paused");
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    let c = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_ne!(b, c, "the rain did not resume");
    // V: the whole page is lit — the gpu tile's title is readable at once.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('V'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("gpu"), "{text}");
    assert!(text.contains("sources"), "{text}");
    // L: locked — after many frames the page still reads.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('L'))));
    // The toast is checked at once: it expires on the wall clock, and 200
    // rain frames take longer than its life on a slow runner.
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("everything lit"), "{text}");
    for _ in 0..200 {
        let _ = shot_frame(&mut sh, 250, 70);
    }
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("gpu") && text.contains("sources"), "{text}");
}

/// A quiet theme has no layer: `V`/`L` explain themselves; `--no-effects`
/// (effects off) under matrix draws the plain page.
#[test]
fn quiet_themes_and_no_effects_have_no_rain() {
    let mut sh = shell_with_theme(config::DEFAULT_LAYOUT, 1, "retrowave");
    sh.set_effects(true, 4);
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('V'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(text.contains("showcase theme"), "{text}");
    let mut off = shell_with_theme(config::DEFAULT_LAYOUT, 1, "matrix");
    off.set_effects(false, 4);
    let frame = shot_frame(&mut off, 250, 70);
    assert_eq!(katakana_cells(&frame), 0);
    let text = page_text(&mut off, 250, 70);
    assert!(text.contains("gpu") && text.contains("balance"), "{text}");
}

// ───────────────────── arc 4b review: the confirmed findings ─────────────────────

/// Every frame of a sweep cycle keeps the focused tile, the alerting tile,
/// the banner, the tab bar and the key bar as the mold (readability floor);
/// two matrix shells fed the same inputs are byte-identical frame for frame
/// (the effects tick on the run clock, not wall time).
#[test]
fn matrix_pins_hold_every_frame_and_two_shells_agree() {
    let mut a = matrix_shell();
    let mut b = matrix_shell();
    // Warm both, then compare 60 frames.
    for _ in 0..3 {
        let _ = shot_frame(&mut a, 250, 70);
        let _ = shot_frame(&mut b, 250, 70);
    }
    for _ in 0..60 {
        let fa = shot_frame(&mut a, 250, 70);
        let fb = shot_frame(&mut b, 250, 70);
        assert_eq!(fa, fb, "two matrix shells diverged");
    }
    // A whole sweep cycle: the pinned surfaces never carry a rain glyph.
    let cycle = 24 * 23;
    for _ in 0..cycle {
        let frame = shot_frame(&mut a, 250, 70);
        let row = |y: u16| -> String {
            (0..250u16)
                .map(|x| frame.cell((x, y)).unwrap().symbol().to_string())
                .collect()
        };
        assert!(row(0).contains("gridwatch"), "tab bar rained: {}", row(0));
        assert!(
            row(1).contains("ALERT: OVERLOAD"),
            "banner rained: {}",
            row(1)
        );
        assert!(row(69).contains("q quit"), "key bar rained: {}", row(69));
        // The focused (cpu) tile's title and the alerting pins tile's title
        // (matrix upper-cases titles).
        assert!(
            row(2).to_lowercase().contains("cpu"),
            "focused tile rained: {}",
            row(2)
        );
        let pins_title: String = (37..40u16)
            .map(|y| row(y).to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            pins_title.contains("pins"),
            "alerting tile rained: {pins_title}"
        );
    }
}

/// Paused (and unfocused) the rain stands still but the UI still draws: the
/// `paused` toast shows at once, a page change shows in the tab bar, and a
/// new Crit banner raised while frozen reaches the screen.
#[test]
fn frozen_matrix_still_shows_toasts_pages_and_new_alerts() {
    use gridwatch_store::{AlertEvent, AlertId, Severity, Transition};
    let mut sh = matrix_shell();
    for _ in 0..5 {
        let _ = shot_frame(&mut sh, 250, 70);
    }
    // Ack the demo overload so the banner is down, then freeze.
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('a'))));
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char(' '))));
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("paused"),
        "no pause toast while frozen: {text}"
    );
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('2'))));
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains(" 2 audio ") && text.contains("now playing"),
        "{text}"
    );
    sh.apply_control(ControlMsg::Alert(AlertEvent {
        id: AlertId::new("pins/disconnected"),
        source: SourceId("pins"),
        severity: Severity::Crit,
        transition: Transition::Raised,
        title: "DISCONNECT".into(),
        detail: "pin 3 open".into(),
        at: Ts(40_000_000_000),
    }));
    let text = page_text(&mut sh, 250, 70);
    assert!(
        text.contains("⚠ alert: disconnect ⚠"),
        "banner hidden while frozen: {text}"
    );
    // Frozen frames are stable.
    let a = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    let b = gridwatch_ui::dump::cells(&shot_frame(&mut sh, 250, 70));
    assert_eq!(a, b);
}

/// A resize under matrix re-prints the page within a few frames instead of
/// waiting for the next 20 s sweep.
#[test]
fn resize_under_matrix_relights_the_page() {
    let mut sh = matrix_shell();
    for _ in 0..50 {
        let _ = shot_frame(&mut sh, 250, 70);
    }
    sh.handle_input(InputEvent::Resize(200, 60));
    let mut lit = 0;
    for _ in 0..24 * 4 {
        let frame = shot_frame(&mut sh, 200, 60);
        let t: String = (0..200u16)
            .map(|x| frame.cell((x, 3)).unwrap().symbol().to_string())
            .collect();
        if t.contains("CPU") || t.contains("cpu") {
            lit += 1;
        }
    }
    assert!(lit > 0, "the page stayed dark after the resize");
}

/// The lock keeps the rain out of every tile; the flourish art never touches
/// a tile at either real size.
#[test]
fn lock_keeps_rain_in_gutters_and_flourishes_stay_in_holes() {
    let mut sh = matrix_shell();
    for _ in 0..5 {
        let _ = shot_frame(&mut sh, 250, 70);
    }
    sh.handle_input(InputEvent::Key(KeyEvent::plain(KeyCode::Char('L'))));
    for _ in 0..100 {
        let frame = shot_frame(&mut sh, 250, 70);
        // The cpu tile occupies x 0..125, y 1..36: no rain glyph inside.
        let inside = (0..125u16)
            .flat_map(|x| (1..36u16).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                frame.cell((x, y)).is_some_and(|c| {
                    c.symbol()
                        .chars()
                        .any(|ch| ('\u{FF66}'..='\u{FF9D}').contains(&ch))
                })
            })
            .count();
        assert_eq!(inside, 0, "rain inside a locked tile");
    }
    // Flourishes: a layout with a 4x2 hole and a 2x1 hole under retrowave.
    let layout = "schema = 1\n[grid]\ncolumns = 12\nrows = 6\n[[pages]]\nname = \"Holes\"\nhotkey = \"1\"\nplace = [\n  { id = \"cpu\", at = [0, 0], size = [6, 3] },\n  { id = \"gpu\", at = [6, 0], size = [6, 3] },\n  { id = \"pins\", at = [0, 3], size = [4, 2] },\n  { kind = \"clock\", at = [10, 5], size = [2, 1] },\n]\n";
    for (w, h) in [(250u16, 70u16), (131, 37)] {
        let mut sh = shell_with_theme(layout, 1, "retrowave");
        let with = shot_frame(&mut sh, w, h);
        // The tiles' outer rects from the same solver the shell uses (body =
        // the frame minus the tab bar and key bar): no art glyph inside any.
        let body = ratatui::layout::Rect::new(0, 1, w, h - 2);
        let spec = gridwatch_ui::layout::GridSpec::default();
        let mode = gridwatch_ui::layout::SolveMode::Configured;
        let mut inside = 0;
        for (at, size) in [
            ((0u8, 0u8), (6u8, 3u8)),
            ((6, 0), (6, 3)),
            ((0, 3), (4, 2)),
            ((10, 5), (2, 1)),
        ] {
            let r = gridwatch_ui::layout::unit_rect(&spec, body, mode, at, size).unwrap();
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    if with.cell((x, y)).unwrap().symbol() == "╱" {
                        inside += 1;
                    }
                }
            }
        }
        assert_eq!(inside, 0, "floor lines inside a tile at {w}x{h}");
        // And the art is there, in the hole.
        let hole = gridwatch_ui::layout::unit_rect(&spec, body, mode, (4, 3), (6, 2)).unwrap();
        let art = (hole.y..hole.y + hole.height)
            .flat_map(|y| (hole.x..hole.x + hole.width).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(with.cell((x, y)).unwrap().symbol(), "╱" | "█" | "▀" | "─"))
            .count();
        assert!(art > 0, "no flourish in the hole at {w}x{h}");
    }
}

// ───────────────────── arc 5a: seam 5, the Animated plumbing ─────────────────────

/// A visible audio tile with sound makes the shell animate at the tile's fps
/// (capped by `fps_max`); a silent, settled tile drops the cause; `FocusLost`
/// drops the rate to `unfocused_fps`.
#[test]
fn an_animated_tile_drives_the_frame_rate_and_silence_drops_it() {
    use gridwatch_store::keys::audio;
    let mut sh = shell_with_theme(config::DEFAULT_LAYOUT, 2, "mono");
    assert!(!sh.animated_visible(), "nothing drawn yet");
    let _ = shot_frame(&mut sh, 250, 70);
    assert!(sh.animated_visible(), "the audio tile is drawn with sound");
    assert_eq!(sh.effective_fps(), 30, "the tile's fps");
    // Unfocused: the throttle wins whatever animates.
    sh.handle_input(InputEvent::FocusLost);
    let _ = shot_frame(&mut sh, 250, 70);
    assert_eq!(sh.effective_fps(), 2);
    sh.handle_input(InputEvent::FocusGained);
    // Silence: the level Record says so and the bands are zeros; the
    // ballistics decay on the run clock, then the cause drops. The winamp
    // tile on the same page animates too (arc 6), so its player stops.
    let at = Ts(sh.store.latest().0 + 1);
    let zeros: gridwatch_store::Vec32 = std::sync::Arc::from(vec![0f32; audio::BANDS]);
    sh.store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: audio::SOURCE,
        at,
        samples: vec![
            gridwatch_store::Sample {
                id: audio::BANDS_KEY.idx(0).id,
                datum: gridwatch_store::Datum::Vector(zeros.clone()),
            },
            gridwatch_store::Sample {
                id: audio::BANDS_KEY.idx(1).id,
                datum: gridwatch_store::Datum::Vector(zeros),
            },
            gridwatch_store::Sample {
                id: audio::LEVEL.id.clone(),
                datum: gridwatch_store::Datum::Record(std::sync::Arc::new(audio::AudioLevel {
                    silent: true,
                    since: at,
                })),
            },
        ],
    }));
    sh.store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: gridwatch_store::keys::media::SOURCE,
        at,
        samples: vec![gridwatch_store::Sample {
            id: gridwatch_store::keys::media::NOW.id.clone(),
            datum: gridwatch_store::Datum::Record(std::sync::Arc::new(
                gridwatch_store::keys::media::NowPlaying {
                    status: gridwatch_store::keys::media::PlayStatus::Paused,
                    ..gridwatch_store::demo::MediaSynth::now_at(at)
                },
            )),
        }],
    }));
    let mut t = at;
    for _ in 0..200 {
        t = Ts(t.0 + 33_000_000);
        sh.set_clock(t);
        let _ = shot_frame(&mut sh, 250, 70);
    }
    assert!(!sh.animated_visible(), "silent and settled");
    // The cap: a config with `fps_max = 20` holds a 30 fps tile at 20.
    let cfg = config::DEFAULT_CONFIG.replace("fps_max = 60", "fps_max = 20");
    assert!(
        cfg.contains("fps_max = 20"),
        "the default config names fps_max"
    );
    let loaded = config::load_texts(&cfg, config::DEFAULT_LAYOUT).unwrap();
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    let theme = load_builtin("mono", ColorMode::TrueColor).unwrap();
    let mut capped = Shell::new(
        reg,
        &loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    capped.set_page(1);
    gridwatch_app::feed_synth(&mut capped, 42, 40);
    let _ = shot_frame(&mut capped, 250, 70);
    assert!(capped.animated_visible());
    assert_eq!(capped.effective_fps(), 20, "capped by fps_max");
}

/// The animation-frame term re-renders the animated tile every frame while a
/// quiet neighbour's cache holds: the audio tile's cells change between two
/// consecutive frames on the run clock, the clock tile's do not.
#[test]
fn the_cache_re_renders_only_the_animated_tile() {
    use gridwatch_store::keys::audio;
    let mut sh = shell_with_theme(config::DEFAULT_LAYOUT, 2, "mono");
    let _ = shot_frame(&mut sh, 250, 70);
    // The bands drop to zero (not silent): the bars fall on the run clock
    // over the next frames with no further batch — only the tick's clock
    // and the cache's animation term can make consecutive frames differ.
    let at = Ts(sh.store.latest().0 + 1);
    let zeros: gridwatch_store::Vec32 = std::sync::Arc::from(vec![0f32; audio::BANDS]);
    sh.store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: audio::SOURCE,
        at,
        samples: vec![
            gridwatch_store::Sample {
                id: audio::BANDS_KEY.idx(0).id,
                datum: gridwatch_store::Datum::Vector(zeros.clone()),
            },
            gridwatch_store::Sample {
                id: audio::BANDS_KEY.idx(1).id,
                datum: gridwatch_store::Datum::Vector(zeros),
            },
        ],
    }));
    sh.set_clock(Ts(at.0 + 33_000_000));
    let a = shot_frame(&mut sh, 250, 70);
    // Two seconds on (a tick advances at most 0.5 s): the caps have fallen
    // to the floor and the bars are gone.
    let mut b = a.clone();
    for i in 1..=4u64 {
        sh.set_clock(Ts(at.0 + 33_000_000 + i * 500_000_000));
        b = shot_frame(&mut sh, 250, 70);
    }
    let tile = |f: &ratatui::buffer::Buffer| -> String {
        (1..21u16)
            .map(|y| {
                (124..248u16)
                    .map(|x| f.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    };
    assert_ne!(
        tile(&a),
        tile(&b),
        "the audio tile moved between frames:\n{}",
        tile(&a)
    );
    // The winamp placeholder tile (left half of the row) is byte-identical.
    let row = |f: &ratatui::buffer::Buffer, y: u16| -> String {
        (0..60u16)
            .map(|x| f.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    };
    for y in 1..20u16 {
        assert_eq!(
            row(&a, y),
            row(&b, y),
            "a quiet tile re-rendered at row {y}"
        );
    }
}
