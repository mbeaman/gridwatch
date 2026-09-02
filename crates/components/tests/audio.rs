//! audio component gate tests (§8, brief arc 5 seam 4): the tier per real
//! grid size, the keys (`m`, `g`, `[`/`]`, the sink picker's `Domain`
//! command), the animated redraw policy with silence dropping it, the empty
//! and `Unavailable` tiles, and the mirrored bar layout.

use std::sync::Arc;

use gridwatch_components::audio::{Audio, Mode, Options, Preset, TIER_SPECTRUM, resample_max};
use gridwatch_store::keys::audio::{self, SetSink};
use gridwatch_store::{
    Control, ControlMsg, KeyCode, KeyEvent, Mods, Msg, SourceState, SourceStatus, Store, Ts,
};
use gridwatch_ui::component::{
    Command, Component, InputCx, Outcome, Redraw, RedrawPolicy, Size, TickCx, pick_tier,
};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn tile() -> Audio {
    Audio::default()
}

#[test]
fn audio_tiers_match_the_real_grid_sizes() {
    let c = tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("mini", false), "1x1 at 250x70");
    assert_eq!(tier(9, 5, false), ("vu", false), "1x1 dense");
    assert_eq!(tier(38, 8, false), ("scope", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("spectrum", false), "4x2 at 250x70");
    assert_eq!(
        tier(122, 31, false),
        ("spectrum", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
}

fn cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, 80, 20),
        caps,
        readonly: false,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        mods: Mods::NONE,
    }
}

#[test]
fn keys_cycle_mode_preset_and_window() {
    let store = demo_store(42, 3);
    let caps = gridwatch_store::CapSet::default();
    let mut a = tile();
    assert_eq!(a.mode(), Mode::Bars);
    assert!(matches!(
        a.on_key(key(KeyCode::Char('m')), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert_eq!(a.mode(), Mode::Scope);
    a.on_key(key(KeyCode::Char('m')), &cx(&store, &caps));
    assert_eq!(a.mode(), Mode::Both);
    a.on_key(key(KeyCode::Char('m')), &cx(&store, &caps));
    assert_eq!(a.mode(), Mode::Bars);
    assert_eq!(a.preset(), Preset::Winamp);
    a.on_key(key(KeyCode::Char('g')), &cx(&store, &caps));
    assert_eq!(a.preset(), Preset::Cava);
    let w = a.window();
    a.on_key(key(KeyCode::Char('[')), &cx(&store, &caps));
    assert!(a.window().len() < w.len(), "narrowed");
    a.on_key(key(KeyCode::Char(']')), &cx(&store, &caps));
    assert_eq!(a.window(), w, "widened back");
    assert!(matches!(
        a.on_key(key(KeyCode::Char('z')), &cx(&store, &caps)),
        Outcome::Ignored
    ));
}

/// `s` opens the picker and asks the source to enumerate; `Enter` sends the
/// first `Domain` control — a boxed `SetSink` the source downcasts.
#[test]
fn the_sink_picker_sends_a_domain_command_that_downcasts() {
    let mut store = demo_store(42, 3);
    let caps = gridwatch_store::CapSet::default();
    let mut a = tile();
    tick(&mut a, &store, TIER_SPECTRUM);
    let out = a.on_key(key(KeyCode::Char('s')), &cx(&store, &caps));
    match out {
        Outcome::Command(Command::Source(id, Control::SetOption(k, v))) => {
            assert_eq!(id, audio::SOURCE);
            assert_eq!(k, "enumerate");
            assert_eq!(v, toml::Value::Boolean(true));
        }
        _ => panic!("not the expected command"),
    }
    assert_eq!(
        a.picker().unwrap().sinks.len(),
        1,
        "the current sink at once"
    );
    // The source answers with the list; the picker follows it on the next tick.
    let sinks = audio::AudioSinks {
        sinks: vec![
            gridwatch_store::demo::audio_sink(),
            audio::AudioSink {
                name: "alsa_output.hdmi".into(),
                description: "HDMI".into(),
                serial: 366,
                state: "suspended".into(),
                is_default: false,
                rate: 48_000,
                channels: 2,
            },
        ],
    };
    store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: audio::SOURCE,
        at: Ts(9_000_000_000),
        samples: vec![gridwatch_store::Sample {
            id: audio::SINKS.id.clone(),
            datum: gridwatch_store::Datum::Record(Arc::new(sinks)),
        }],
    }));
    tick(&mut a, &store, TIER_SPECTRUM);
    assert_eq!(a.picker().unwrap().sinks.len(), 2);
    a.on_key(key(KeyCode::Down), &cx(&store, &caps));
    assert_eq!(a.picker().unwrap().selected, 1);
    let th = theme("modern");
    let (_, buf) = render_component(&mut a, &store, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(
        text.contains("HDMI") && text.contains("suspended"),
        "{text}"
    );
    let out = a.on_key(key(KeyCode::Enter), &cx(&store, &caps));
    match out {
        Outcome::Command(Command::Source(id, Control::Domain(b))) => {
            assert_eq!(id, audio::SOURCE);
            let s = b.downcast::<SetSink>().expect("a SetSink");
            assert_eq!(s.0, "alsa_output.hdmi");
        }
        _ => panic!("not the expected command"),
    }
    assert!(a.picker().is_none(), "closed on Enter");
    // Esc closes and stops the enumeration.
    a.on_key(key(KeyCode::Char('s')), &cx(&store, &caps));
    match a.on_key(key(KeyCode::Esc), &cx(&store, &caps)) {
        Outcome::Command(Command::Source(_, Control::SetOption(k, v))) => {
            assert_eq!((k.as_str(), v), ("enumerate", toml::Value::Boolean(false)));
        }
        _ => panic!("not the expected command"),
    }
}

/// Animated while there is sound; `Redraw::No` once silent and settled.
#[test]
fn animated_with_sound_and_quiet_once_settled() {
    let a = tile();
    assert_eq!(a.redraw_policy(), RedrawPolicy::Animated { fps: 30 });
    let fast = Audio::new(Options {
        fps: 60,
        ..Options::default()
    });
    assert_eq!(fast.redraw_policy(), RedrawPolicy::Animated { fps: 60 });

    let store = demo_store(42, 3); // lit at 4.5 s
    let mut a = tile();
    let mut now = store.latest();
    let tick_at = |a: &mut Audio, now: Ts| {
        a.tick(&TickCx {
            store: &store,
            now,
            visible: true,
            tier: TIER_SPECTRUM,
        })
    };
    assert_eq!(tick_at(&mut a, now), Redraw::Yes);
    assert!(!a.silent());
    for _ in 0..10 {
        now = Ts(now.0 + 33_000_000);
        assert_eq!(
            tick_at(&mut a, now),
            Redraw::Yes,
            "sound keeps it animating"
        );
    }
    // A silent store: the level Record says so; the bars decay, then rest.
    let mut quiet = Store::default();
    let mut synth = gridwatch_store::demo::AudioSynth::new(1);
    quiet.apply(&Msg::Batch(synth.tick_at(Ts(500_000_000))));
    let mut b = tile();
    let mut t = Ts(500_000_000);
    let mut last = Redraw::Yes;
    for _ in 0..60 {
        t = Ts(t.0 + 33_000_000);
        last = b.tick(&TickCx {
            store: &quiet,
            now: t,
            visible: true,
            tier: TIER_SPECTRUM,
        });
    }
    assert!(b.silent());
    assert_eq!(last, Redraw::No, "silent and settled: no animation frame");
}

/// The empty store draws no numbers; an `Unavailable` source names its
/// reason and hint in the tile.
#[test]
fn empty_and_unavailable_tiles_are_honest() {
    let th = theme("modern");
    let empty = Store::default();
    let mut a = tile();
    let (_, buf) = render_component(&mut a, &empty, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(text.contains("no audio"), "{text}");
    assert!(!text.contains("dB"), "no fabricated levels: {text}");
    assert!(text.contains("Hz"), "the axis still reads: {text}");

    let mut store = Store::default();
    store.ensure_source(audio::SOURCE);
    store.apply(&Msg::Control(ControlMsg::Status(
        audio::SOURCE,
        SourceStatus {
            state: SourceState::Unavailable,
            reason: Some(Arc::from("pw-record not found")),
            hint: Some(Arc::from("install pipewire-bin")),
            since: Ts(1),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        },
    )));
    let mut a = tile();
    let (_, buf) = render_component(&mut a, &store, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(
        text.contains("pw-record not found") && text.contains("install pipewire-bin"),
        "{text}"
    );
}

/// The spectrum's mirrored layout: ⌊(w+1)/3⌋ bars, the left channel
/// reversed on the left, a gap column after every bar.
#[test]
fn spectrum_bars_are_mirrored_and_thick() {
    let store = demo_store(42, 3);
    let mut a = tile();
    tick(&mut a, &store, TIER_SPECTRUM);
    let (values, peaks) = gridwatch_components::audio::mirrored_for_test(&a, 80);
    assert_eq!(values.len(), peaks.len());
    assert!(values.len() <= 80);
    // ⌊(80+1)/3⌋ = 27 bars → 13 per side, 78 cells, centred with one cell
    // of padding on the left.
    let n = 26; // bars actually drawn (13 per channel)
    let pad = (80 - n * 3) / 2;
    assert_eq!(pad, 1);
    assert_eq!(values[0], 0.0, "left pad");
    // Thick: value pairs then a zero gap.
    assert_eq!(values[pad], values[pad + 1]);
    assert_eq!(values[pad + 2], 0.0);
    // Mirrored: the two centre bars are both channels' lowest band.
    let (l, _) = a.resampled(0, n / 2);
    let (r, _) = a.resampled(1, n / 2);
    let centre_left = values[pad + (n / 2 - 1) * 3];
    let centre_right = values[pad + (n / 2) * 3];
    assert_eq!(centre_left, l[0]);
    assert_eq!(centre_right, r[0]);
    assert_eq!(resample_max(&[0.1, 0.9], 1), [0.9]);
}
