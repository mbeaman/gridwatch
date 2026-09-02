//! winamp component gate tests (§8, brief arc 6 seam 3): the tier per real
//! grid size, the transport keys as `Domain` commands (and what a player
//! cannot do), stream mode, the idle skin, the vis with and without the
//! audio source, and the art column.

use std::sync::Arc;

use gridwatch_components::winamp::{Options, TIER_MAIN, Vis, Winamp};
use gridwatch_store::keys::media::{self, Art, Caps, MediaCmd, NowPlaying, PlayStatus};
use gridwatch_store::{
    Batch, Datum, KeyCode, KeyEvent, Mods, Msg, Sample, SourceState, SourceStatus, Store, Ts,
};
use gridwatch_ui::component::{
    Command, Component, InputCx, Outcome, Redraw, Size, TickCx, pick_tier,
};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme, tick, view_of};
use ratatui_core::layout::Rect;

fn tile() -> Winamp {
    Winamp::default()
}

#[test]
fn winamp_tiers_match_the_real_grid_sizes() {
    let c = tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("status", false), "1x1 at 250x70");
    assert_eq!(tier(38, 8, false), ("shade", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("main+art", false), "4x2 at 250x70");
    assert_eq!(
        tier(122, 31, false),
        ("main+art", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(40, 10, false), ("main", false));
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
}

fn store_with(now: NowPlaying) -> Store {
    let mut store = Store::default();
    store.apply(&Msg::Batch(Batch {
        source: media::SOURCE,
        at: Ts(1_000_000_000),
        samples: vec![Sample {
            id: media::NOW.id.clone(),
            datum: Datum::Record(Arc::new(now)),
        }],
    }));
    store
}

fn playing(len_us: Option<i64>) -> NowPlaying {
    NowPlaying {
        player: "synth".into(),
        identity: "Synth".into(),
        title: "A Title Long Enough To Scroll Across Any Tile".into(),
        artist: "The Artist".into(),
        album: "An Album".into(),
        url: "https://example.invalid/x".into(),
        status: PlayStatus::Playing,
        pos_us: 30_000_000,
        read_at: Ts(1_000_000_000),
        len_us,
        rate: 1.0,
        volume: 0.4,
        caps: Caps {
            play_pause: true,
            next: true,
            prev: true,
            seek: true,
            control: true,
            raise: false,
        },
        track: 42,
    }
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

fn cmd_of(out: Outcome) -> MediaCmd {
    match out {
        Outcome::Command(Command::Source(id, gridwatch_store::Control::Domain(b))) => {
            assert_eq!(id, media::SOURCE);
            *b.downcast::<MediaCmd>().expect("a MediaCmd")
        }
        _ => panic!("not a source command"),
    }
}

#[test]
fn the_transport_keys_send_domain_commands() {
    let store = store_with(playing(Some(200_000_000)));
    let caps = gridwatch_store::CapSet::default();
    let mut a = tile();
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Char('x')), &cx(&store, &caps))),
        MediaCmd::Play
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Char('c')), &cx(&store, &caps))),
        MediaCmd::Pause
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Char('v')), &cx(&store, &caps))),
        MediaCmd::Stop
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Char('b')), &cx(&store, &caps))),
        MediaCmd::Next
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Char('z')), &cx(&store, &caps))),
        MediaCmd::Prev
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Right), &cx(&store, &caps))),
        MediaCmd::SeekBy(5_000_000)
    );
    assert_eq!(
        cmd_of(a.on_key(key(KeyCode::Left), &cx(&store, &caps))),
        MediaCmd::SeekBy(-5_000_000)
    );
    // Volume steps from what the player reported.
    match cmd_of(a.on_key(key(KeyCode::Char('+')), &cx(&store, &caps))) {
        MediaCmd::SetVolume(v) => assert!((v - 0.45).abs() < 1e-9, "{v}"),
        other => panic!("{other:?}"),
    }
    // An unsupported action is consumed, not sent: the player said it
    // cannot raise.
    assert!(matches!(
        a.on_key(key(KeyCode::Char('r')), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    // A player that can do nothing: every transport key is consumed.
    let mut none = playing(Some(1));
    none.caps = Caps::default();
    let store = store_with(none);
    let mut b = tile();
    for k in ['x', 'c', 'v', 'b', 'z'] {
        assert!(
            matches!(
                b.on_key(key(KeyCode::Char(k)), &cx(&store, &caps)),
                Outcome::Consumed
            ),
            "{k} was not consumed"
        );
    }
}

/// The tile animates while a track plays and stops asking for frames once
/// it is paused and settled (the arc 5a rule).
#[test]
fn animation_follows_playback() {
    let store = store_with(playing(Some(200_000_000)));
    let mut a = tile();
    let mut at = Ts(2_000_000_000);
    let tick_now = |a: &mut Winamp, now: Ts, store: &Store| {
        a.tick(&TickCx {
            store,
            now,
            visible: true,
            tier: TIER_MAIN,
        })
    };
    assert_eq!(tick_now(&mut a, at, &store), Redraw::Yes);
    at = Ts(at.0 + 100_000_000);
    assert_eq!(
        tick_now(&mut a, at, &store),
        Redraw::Yes,
        "playing animates"
    );
    let mut paused = playing(Some(200_000_000));
    paused.status = PlayStatus::Paused;
    let store = store_with(paused);
    let mut b = tile();
    assert_eq!(
        tick_now(&mut b, at, &store),
        Redraw::Yes,
        "the first frame draws the paused tile"
    );
    at = Ts(at.0 + 100_000_000);
    assert_eq!(tick_now(&mut b, at, &store), Redraw::No, "then quiet");
}

#[test]
fn stream_mode_and_the_idle_skin() {
    let th = theme("modern");
    // No player at all.
    let mut store = Store::default();
    store.ensure_source(media::SOURCE);
    let mut a = tile();
    let (_, buf) = render_component(&mut a, &store, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(text.contains("no player"), "{text}");
    assert!(!text.contains("0:00"), "no fabricated clock: {text}");
    // An unavailable source says why.
    store.apply(&Msg::Control(gridwatch_store::ControlMsg::Status(
        media::SOURCE,
        SourceStatus {
            state: SourceState::Unavailable,
            reason: Some(Arc::from("no session bus")),
            hint: Some(Arc::from("start a desktop session")),
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
        text.contains("no session bus") && text.contains("desktop session"),
        "{text}"
    );
    // A stream: no posbar fraction, the word instead.
    let store = store_with(playing(None));
    let mut a = tile();
    let (tier, buf) = render_component(&mut a, &store, &th, Size::new(40, 10), false);
    assert_eq!(a.tiers()[tier].name, "main");
    let text = plain_text(&buf);
    assert!(text.contains("stream"), "{text}");
    assert!(text.contains("vol"), "the tier's signature: {text}");
}

/// The vis draws the audio source's bands when it is running and a static
/// skin when it is not; the art column appears only with a matching cover.
#[test]
fn the_vis_borrows_audio_and_the_art_needs_a_matching_track() {
    let th = theme("modern");
    let store = store_with(playing(Some(200_000_000)));
    let mut a = tile();
    let (_, plain) = render_component(&mut a, &store, &th, Size::new(80, 20), false);
    let without = plain_text(&plain);
    assert!(
        without.contains("█") || without.contains("▄"),
        "a static skin: {without}"
    );
    // With every synth (audio included) the bars follow the store.
    let demo = demo_store(42, 8);
    let mut b = tile();
    tick(&mut b, &demo, TIER_MAIN);
    let (_, with) = render_component(&mut b, &demo, &th, Size::new(80, 20), false);
    let with = plain_text(&with);
    assert!(with.contains("█"), "{with}");
    // Art for another track is ignored.
    let mut store = store_with(playing(Some(200_000_000)));
    store.apply(&Msg::Batch(Batch {
        source: media::SOURCE,
        at: Ts(2_000_000_000),
        samples: vec![Sample {
            id: media::ART.id.clone(),
            datum: Datum::Record(Arc::new(Art {
                track: 999,
                w: 2,
                h: 2,
                rgb: vec![255; 12],
            })),
        }],
    }));
    // The art is a `View::Custom` the ui crate paints; the tree says
    // whether it is there (the big digits use ▀ too, so cells cannot).
    let has_art = |c: &mut Winamp, store: &Store| -> bool {
        let tree = view_of(c, store, &th, Size::new(80, 20));
        // `Debug` for `View` prints one level; the dump walks the tree.
        gridwatch_ui::dump::view_value(&tree)
            .to_string()
            .contains("album art")
    };
    let mut c = tile();
    assert!(
        !has_art(&mut c, &store),
        "another track's cover is not drawn"
    );
    // The right track's cover is.
    store.apply(&Msg::Batch(Batch {
        source: media::SOURCE,
        at: Ts(3_000_000_000),
        samples: vec![Sample {
            id: media::ART.id.clone(),
            datum: Datum::Record(Arc::new(Art {
                track: 42,
                w: 2,
                h: 2,
                rgb: vec![255; 12],
            })),
        }],
    }));
    let mut d = tile();
    assert!(has_art(&mut d, &store), "the cover is drawn");
    // `art = false` refuses it.
    let mut no_art = Winamp::new(Options {
        art: false,
        ..Options::default()
    });
    assert!(!has_art(&mut no_art, &store));
    // `vis = "off"` draws no bars.
    let mut off = Winamp::new(Options {
        vis: Vis::Off,
        ..Options::default()
    });
    let (_, buf) = render_component(&mut off, &demo, &th, Size::new(80, 20), false);
    let text = plain_text(&buf);
    assert!(text.contains("vol"), "the rest is still there: {text}");
}
