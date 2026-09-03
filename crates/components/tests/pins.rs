//! pins component gate tests (§8, brief arc 3 seam 4): the tier per real
//! grid size, the balance classes, the limit line's styling through roles,
//! the keys (`p` freeze, `r` peaks, `+`/`−` as a `Command::Source`), the
//! header without the gpu source, and the honest empty tile.

use gridwatch_components::pins::model::{BalanceClass, balance_class};
use gridwatch_components::pins::{Pins, TIER_BARS};
use gridwatch_store::keys::pins;
use gridwatch_store::{Control, KeyCode, KeyEvent, Mods, Store, Ts};
use gridwatch_ui::component::{Command, Component, InputCx, Outcome, Size, pick_tier};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme};
use gridwatch_ui::theme::Role;
use ratatui_core::layout::Rect;

fn pins_tile() -> Pins {
    Pins::default()
}

#[test]
fn pins_tiers_match_the_real_grid_sizes() {
    let c = pins_tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("watts-badge", false), "1x1 at 250x70");
    assert_eq!(tier(38, 8, false), ("mini-bars", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("trend", false), "4x2 at 250x70");
    assert_eq!(tier(40, 8, false), ("bars", false));
    assert_eq!(
        tier(59, 13, false),
        ("bars", false),
        "one row short of trend"
    );
    assert_eq!(
        tier(122, 31, false),
        ("trend", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
}

#[test]
fn balance_classes_follow_tui_rs() {
    assert_eq!(
        balance_class(Some(1.2), Some(9.0), 5.0, 1.5),
        BalanceClass::Normal
    );
    assert_eq!(
        balance_class(Some(1.4), Some(9.0), 5.0, 1.5),
        BalanceClass::Warn
    );
    assert_eq!(
        balance_class(Some(1.6), Some(9.0), 5.0, 1.5),
        BalanceClass::Alarm
    );
    assert_eq!(
        balance_class(Some(1.6), Some(4.0), 5.0, 1.5),
        BalanceClass::Idle,
        "below min_load the ratio is noise"
    );
    assert_eq!(
        balance_class(None, Some(9.0), 5.0, 1.5),
        BalanceClass::Unknown
    );
}

/// The limit line paints `┄` only on empty cells and only through `Role::Crit`.
#[test]
fn limit_line_is_crit_role_on_empty_cells_only() {
    let store = demo_store(42, 6);
    let th = theme("modern");
    let mut p = pins_tile();
    let (tier, buf) = render_component(&mut p, &store, &th, Size::new(40, 8), false);
    assert_eq!(p.tiers()[tier].name, "bars");
    let crit = th.style(Role::Crit);
    let mut dashes = 0;
    for y in 0..8u16 {
        for x in 0..40u16 {
            let cell = buf.cell((x, y)).unwrap();
            if cell.symbol() == "┄" {
                dashes += 1;
                assert_eq!(cell.fg, crit.fg.unwrap(), "limit line styled by role");
            }
        }
    }
    assert!(dashes > 0, "no limit line drawn:\n{}", plain_text(&buf));
    // Row 0 of the bars is 9.2/10 of the way up: the line sits near the top.
    let text = plain_text(&buf);
    let row_with_dash = text.lines().position(|l| l.contains('┄')).unwrap();
    assert!(
        row_with_dash <= 1,
        "limit line at row {row_with_dash}:\n{text}"
    );
}

fn cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, 80, 20),
        caps,
        readonly: false,
        zoomed: false,
        tier: 0,
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        mods: Mods::NONE,
    }
}

/// `+`/`−` are the first `Command::Source` in the product: a `SetOption`
/// clamped to 500–5000 ms; `p` freezes the picture; `r` resets the peaks.
#[test]
fn keys_freeze_reset_and_command_the_interval() {
    let store = demo_store(42, 6);
    let caps = gridwatch_store::CapSet::empty();
    let mut p = pins_tile();
    gridwatch_ui::testkit::tick(&mut p, &store, TIER_BARS);
    assert!(p.model().peak_w > 0.0);
    // tui.rs's sense: `-` is slower (a longer interval), `+` faster.
    match p.on_key(key('-'), &cx(&store, &caps)) {
        Outcome::Command(Command::Source(id, Control::SetOption(k, v))) => {
            assert_eq!(id, pins::SOURCE);
            assert_eq!(k, "interval_ms");
            assert_eq!(v.as_integer(), Some(600));
        }
        _ => panic!("expected a SetOption command"),
    }
    // The pending value is the base for the next press (review: reading it
    // back from `pins.info` lagged a tick): `+` returns to 500.
    match p.on_key(key('+'), &cx(&store, &caps)) {
        Outcome::Command(Command::Source(_, Control::SetOption(_, v))) => {
            assert_eq!(v.as_integer(), Some(500));
        }
        _ => panic!("expected a SetOption command"),
    }
    // `+` from 500 is already the floor: consumed, no command.
    assert!(matches!(
        p.on_key(key('+'), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert!(!p.frozen());
    assert!(matches!(
        p.on_key(key('p'), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert!(p.frozen());
    // Frozen reads PAUSED, never STALE (review).
    let th = theme("mono");
    let (_, buf) = render_component(&mut p, &store, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(text.contains("PAUSED") && !text.contains("STALE"), "{text}");
    p.on_key(key('r'), &cx(&store, &caps));
    assert_eq!(p.model().peak_w, 0.0);
}

/// A telemetry-loss tick makes the picture STALE at once, and an unavailable
/// source says why in the tile (review).
#[test]
fn loss_is_stale_and_an_unavailable_source_explains_itself() {
    use gridwatch_store::keys::pins::PinsState;
    use gridwatch_store::{Batch, Datum, Msg, Sample, SourceState, SourceStatus};
    let mut store = Store::default();
    let mut synth = gridwatch_store::demo::PinsSynth::new(7);
    let t = synth.tick_at(Ts(500_000_000));
    store.apply(&Msg::Batch(t.batch));
    // A loss tick: only `pins.state` with telemetry_lost.
    store.apply(&Msg::Batch(Batch {
        source: pins::SOURCE,
        at: Ts(1_000_000_000),
        samples: vec![Sample {
            id: pins::STATE.id.clone(),
            datum: Datum::Record(std::sync::Arc::new(PinsState {
                telemetry_lost: true,
                misses: 1,
                active: vec![],
                service_active: vec![],
            })),
        }],
    }));
    let th = theme("mono");
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &store, &th, Size::new(40, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains("STALE"), "{text}");
    store.apply(&Msg::Control(gridwatch_store::ControlMsg::Status(
        pins::SOURCE,
        SourceStatus {
            state: SourceState::Unavailable,
            reason: Some("permission denied on /dev/i2c-*".into()),
            hint: Some("add yourself to the i2c group".into()),
            since: Ts(1_000_000_000),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        },
    )));
    let (_, buf) = render_component(&mut p, &store, &th, Size::new(40, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains("pins: permission denied"), "{text}");
}

/// The synth's scripted overload: pins 1–2 at 9.5 A after 20 s, the banner
/// row inside the trend tile, the log line, `‼` in the badge.
#[test]
fn the_scripted_overload_reaches_every_tier() {
    // 20 testkit ticks = 30 s: raised at 21.5 s (the 22.5 s tick), active
    // until the scripted resolve at 50 s.
    let store = demo_store(42, 20);
    let th = theme("mono");
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &store, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(text.contains("⚠ ALERT: OVERLOAD ⚠"), "{text}");
    assert!(text.contains("RAISED OVERLOAD"), "the log line: {text}");
    assert!(text.contains("balance"), "{text}");
    assert!(text.contains("p1 9.") && text.contains("p2 9."), "{text}");
    // 40 ticks = 60 s: resolved at 51 s, the log keeps both lines.
    let later = demo_store(42, 40);
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &later, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(!text.contains("⚠ ALERT"), "{text}");
    assert!(text.contains("RESOLVED OVERLOAD clear after"), "{text}");
    let (_, buf) = render_component(&mut p, &store, &th, Size::new(17, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains('‼'), "the badge's alert glyph: {text}");
    assert!(text.contains("W"));
    // Before the overload (6 ticks = 9 s): no alarm, a `·` glyph.
    let calm = demo_store(42, 6);
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &calm, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(!text.contains("ALERT"), "{text}");
    assert!(text.contains("no alerts this session"), "{text}");
}

/// The zoomed `full` tier: the device header from the gpu source's keys, the
/// six-pin trend legend, `samples N`; and every gpu field `—` without it.
#[test]
fn full_tier_reads_the_gpu_source_and_survives_without_it() {
    let store = demo_store(42, 40);
    let th = theme("mono");
    let mut p = pins_tile();
    let (tier, buf) = render_component(&mut p, &store, &th, Size::new(248, 66), true);
    assert_eq!(p.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    assert!(text.contains("ROG Astral RTX 5090"), "{text}");
    assert!(text.contains("PCIe Gen5×16"), "{text}");
    assert!(text.contains("i2c-3 @ 0x2b"), "{text}");
    assert!(text.contains("PWR "), "{text}");
    assert!(
        text.contains("p1 p2 p3 p4 p5 p6"),
        "the trend legend: {text}"
    );
    assert!(text.contains("samples "), "{text}");
    // Only the pins source: the gpu line says `—`, nothing fabricated.
    let mut only_pins = Store::default();
    let mut synth = gridwatch_store::demo::PinsSynth::new(7);
    for i in 1..=4u64 {
        let t = synth.tick_at(Ts(i * 500_000_000));
        only_pins.apply(&gridwatch_store::Msg::Batch(t.batch));
    }
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &only_pins, &th, Size::new(248, 66), true);
    let text = plain_text(&buf);
    assert!(text.contains("GPU — · PWR —"), "{text}");
    assert!(text.contains("connector 9."), "{text}");
}

/// Empty store: `—` everywhere, no fabricated numbers (the D46 sweep covers
/// every size; this pins the words).
#[test]
fn empty_store_is_honest() {
    let empty = Store::default();
    let th = theme("mono");
    let mut p = pins_tile();
    let (_, buf) = render_component(&mut p, &empty, &th, Size::new(40, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains("— W") && text.contains("— A"), "{text}");
    assert!(!gridwatch_ui::testkit::has_fabricated_percent(&text));
}
