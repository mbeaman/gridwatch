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
    // 4 s later: the cpu source (1.5 s visible → 4.5 s) is not stale yet,
    // the gpu (0.5 s → 1.5 s) and pins (0.5 s → 1.5 s) are.
    sh.set_clock(Ts(64_000_000_000));
    let partly = page_text(&mut sh, 250, 70);
    assert_eq!(partly.matches("stale 4s").count(), 2, "{partly}");
    sh.set_clock(Ts(70_000_000_000));
    let all = page_text(&mut sh, 250, 70);
    assert_eq!(all.matches("stale 10s").count(), 3, "{all}");
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
