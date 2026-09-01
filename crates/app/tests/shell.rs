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
