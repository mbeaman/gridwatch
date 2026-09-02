//! Replay through the whole app (§12.4, brief 2a task 6): the determinism
//! test drives `run_loop<TestBackend>` from `fixtures/journals/torch-idle.jsonl`
//! twice via the real `JournalSource` on a virtual clock and asserts identical
//! frame hashes; the recorder test runs the loop with a recorder attached and
//! replays what it wrote.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use gridwatch_app::{Shell, config, probe, run_loop, shot_frame, shot_replay};
use gridwatch_store::keys::cpu;
use gridwatch_store::{
    Clock, Detail, Header, InputEvent, JOURNAL, JournalSource, KeyCode, KeyEvent, Msg, RecordOpts,
    Recorder, Replay, Source, SourceId, SourceState, Ts, channels,
};
use gridwatch_ui::theme::load_builtin;
use gridwatch_ui::{ColorMode, Registry};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/journals")
        .join(name)
}

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

fn shell(clock: Clock) -> Shell {
    let loaded = config::load_embedded().unwrap();
    let theme = load_builtin("retrowave", ColorMode::TrueColor).unwrap();
    let mut sh = Shell::new(
        registry(),
        &loaded,
        theme,
        probe::probe(),
        0,
        clock,
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    sh.store.ensure_source(JOURNAL);
    sh
}

fn hash(cells: &str) -> u64 {
    let mut h = DefaultHasher::new();
    cells.hash(&mut h);
    h.finish()
}

/// One full replay: the journal source on its own thread (built by hand, as
/// the supervisor would) → the channels → `run_loop` over `TestBackend` on a
/// second thread. When the source has injected its last line, `q` ends the
/// loop; the shell comes back and its final frame is hashed. Intermediate
/// frames depend on scheduling (how many batches a frame drained), the final
/// state does not — that is the property D47 pins.
fn replay_once(path: &Path, w: u16, h: u16) -> (u64, u64, Ts) {
    let (ch, inbox) = channels();
    let clock = Clock::new_virtual();
    let cx = gridwatch_store::SourceCtx::new(
        JOURNAL,
        ch.clone(),
        clock.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        std::sync::Arc::new(gridwatch_store::Demand::default()),
        std::sync::mpsc::channel().1,
        toml::Table::new(),
        0,
    );
    let src = Box::new(JournalSource::new(path, 0.0));
    let source = std::thread::spawn(move || src.run(cx));
    let mut sh = shell(clock);
    let looper = std::thread::spawn(move || {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        run_loop(&mut term, &mut sh, &inbox).unwrap();
        sh
    });
    source.join().expect("the journal source thread");
    ch.input
        .send(InputEvent::Key(KeyEvent::plain(KeyCode::Char('q'))))
        .unwrap();
    let mut sh = looper.join().expect("the frame loop thread");
    assert_eq!(sh.store.status(JOURNAL).state, SourceState::Stopped);
    let frame = shot_frame(&mut sh, w, h);
    let cells = gridwatch_ui::dump::cells(&frame);
    (
        hash(&cells),
        sh.store.generation(cpu::SOURCE),
        sh.store.latest(),
    )
}

/// D47 / §12.4: replaying the same journal twice through the whole app on a
/// virtual clock yields byte-identical final frames, and the store ends at the
/// same generation and timestamp.
#[test]
fn replaying_a_fixture_twice_is_byte_identical() {
    let path = fixture("torch-idle.jsonl");
    let a = replay_once(&path, 250, 70);
    let b = replay_once(&path, 250, 70);
    assert_eq!(a, b, "two replays of the same journal differ");
    assert!(
        a.1 >= 30,
        "60 s of cpu batches at 1.5 s: generation {}",
        a.1
    );
    assert!(
        a.2 >= Ts(55_000_000_000),
        "the journal spans ~60 s: {:?}",
        a.2
    );
}

/// `shot --replay --at` is the same property without the loop: two renders
/// at the same instant are identical, and two instants differ.
#[test]
fn shot_replay_is_deterministic_and_time_dependent() {
    let path = fixture("torch-idle.jsonl");
    let a = shot_replay(registry(), &path, 30.0, 250, 70, "retrowave", 1, "cells").unwrap();
    let b = shot_replay(registry(), &path, 30.0, 250, 70, "retrowave", 1, "cells").unwrap();
    assert_eq!(a, b);
    let c = shot_replay(registry(), &path, 55.0, 250, 70, "retrowave", 1, "cells").unwrap();
    assert_ne!(a, c, "a later instant shows different data");
    // The frame is a real Overview from live-recorded data: the cpu tile is
    // at its table tier … except the fixture was recorded with tables off,
    // so the tier honestly says it is waiting for the scan.
    let text = a.to_lowercase();
    assert!(
        text.contains("ccd0"),
        "cpu tile missing from the replayed frame"
    );
    assert!(
        text.contains("waiting for the process scan"),
        "tables-off fixture must not fabricate rows"
    );
    let svg = shot_replay(registry(), &path, 30.0, 120, 40, "modern", 1, "svg").unwrap();
    assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
}

/// The recorder as the frame loop uses it: messages drained by `run_loop` are
/// teed to the file, and replaying that file reproduces the store.
#[test]
fn what_the_loop_records_replays_to_the_same_store() {
    let dir = std::env::temp_dir().join(format!("gw-app-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("loop.jsonl");
    let (ch, inbox) = channels();
    let mut sh = shell(Clock::new_virtual());
    sh.store.ensure_source(SourceId("cpu"));
    let header = Header::new("test", (131, 40), vec!["cpu".into()]);
    sh.recorder = Some(
        Recorder::start(
            &path,
            &header,
            RecordOpts {
                tables: true,
                input: true,
            },
        )
        .unwrap(),
    );
    let mut synth = gridwatch_store::demo::CpuSynth::new(3);
    for i in 1..=4u64 {
        ch.data
            .try_send(synth.tick_at(Ts(i * 1_500_000_000), Detail::Table))
            .unwrap();
    }
    ch.input
        .send(InputEvent::Key(KeyEvent::plain(KeyCode::Char('q'))))
        .unwrap();
    let mut term = Terminal::new(TestBackend::new(131, 40)).unwrap();
    run_loop(&mut term, &mut sh, &inbox).unwrap();
    let rec = sh.recorder.take().unwrap();
    assert_eq!(rec.dropped(), 0);
    rec.finish().unwrap();

    let mut replay = Replay::load(&path).unwrap();
    assert_eq!(replay.header.as_ref().unwrap().size, [131, 40]);
    // Four batches and the `q` (inputs on, tables on).
    let inputs = replay
        .entries
        .iter()
        .filter(|(_, m)| matches!(m, Msg::Input(_)))
        .count();
    assert_eq!(inputs, 1);
    let mut store = gridwatch_store::Store::default();
    replay.apply_all(&mut store);
    assert_eq!(store.generation(cpu::SOURCE), 4);
    assert_eq!(store.last(&cpu::TOTAL_PCT), sh.store.last(&cpu::TOTAL_PCT));
    assert!(store.record(&cpu::PROC_TABLE).is_some(), "tables on");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Arc 3a (brief seam 7): the `synth-overload` fixture carries the scripted
/// lifecycle as `al` lines — `pins/overload` raised at ≈ 21.5 s and resolved
/// at 50 s — and a replayed frame shows the banner on **page 2** while it is
/// active and not after; two replays stay byte-identical with alerts in the
/// stream.
#[test]
fn the_overload_fixture_raises_the_banner_on_page_two_at_the_scripted_instant() {
    use gridwatch_store::Transition;
    let path = fixture("synth-overload.jsonl");
    let replay = gridwatch_store::Replay::load(&path).expect("fixture loads");
    let raised: Vec<Ts> = replay
        .entries
        .iter()
        .filter_map(|(t, m)| match m {
            gridwatch_store::Msg::Control(gridwatch_store::ControlMsg::Alert(a))
                if a.id.0.as_ref() == "pins/overload" && a.transition == Transition::Raised =>
            {
                Some(*t)
            }
            _ => None,
        })
        .collect();
    assert_eq!(raised.len(), 1, "one raise: {raised:?}");
    let t = raised[0].as_secs_f64();
    assert!(
        (21.0..=23.5).contains(&t),
        "raised at {t:.1} s, expected ≈ 21.5 s (+ one 500 ms tick)"
    );
    // The ansi dump styles every cell; strip the SGR sequences so the text
    // reads as the user saw it.
    fn plain(ansi: &str) -> String {
        let mut out = String::with_capacity(ansi.len());
        let mut rest = ansi;
        while let Some(i) = rest.find('\x1b') {
            out.push_str(&rest[..i]);
            match rest[i..].find('m') {
                Some(j) => rest = &rest[i + j + 1..],
                None => break,
            }
        }
        out.push_str(rest);
        out
    }
    let frame = |at: f64| {
        plain(
            &gridwatch_app::shot_replay(registry(), &path, at, 250, 70, "mono", 2, "ansi").unwrap(),
        )
    };
    let during = frame(30.0);
    assert!(
        during.contains("ALERT: OVERLOAD"),
        "banner on page 2 at 30 s:\n{during}"
    );
    let before = frame(10.0);
    assert!(!before.contains("ALERT: OVERLOAD"), "no banner at 10 s");
    let after = frame(58.0);
    assert!(
        !after.contains("ALERT: OVERLOAD"),
        "resolved by 58 s:\n{after}"
    );
    let (a, _, _) = replay_once(&path, 250, 70);
    let (b, _, _) = replay_once(&path, 250, 70);
    assert_eq!(a, b, "alerts in the stream keep the replay deterministic");
}

/// Arc 5a: `torch-audio.jsonl` (60 s live on torch, `--record`, nothing
/// playing — the silence path) replays deterministically: the bands arrive
/// at the silence cadence, the level Record opens silent, the sink is named,
/// and page 2 at 30 s draws the audio tile's axis with its `silent` note.
#[test]
fn the_audio_fixture_replays_the_silence_path_deterministically() {
    use gridwatch_store::keys::audio;
    let path = fixture("torch-audio.jsonl");
    let replay = gridwatch_store::Replay::load(&path).expect("fixture loads");
    let header = replay.header.as_ref().expect("header");
    assert!(
        header.sources.iter().any(|s| s == "audio"),
        "{:?}",
        header.sources
    );
    let mut bands = 0;
    let mut levels: Vec<(Ts, bool)> = Vec::new();
    let mut sinks = 0;
    for (t, m) in &replay.entries {
        if let Msg::Batch(b) = m
            && b.source == audio::SOURCE
        {
            for s in &b.samples {
                match s.id.name {
                    "audio.bands" => bands += 1,
                    "audio.sink" => sinks += 1,
                    "audio.level" => {
                        if let gridwatch_store::Datum::Record(r) = &s.datum {
                            let l = r
                                .as_any()
                                .downcast_ref::<audio::AudioLevel>()
                                .expect("an AudioLevel");
                            levels.push((*t, l.silent));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Two channels per tick at 2 Hz for 60 s ≈ 240; anything above 100 is
    // the silence cadence doing its job (the sound path would be ≈ 3 600).
    assert!(bands >= 100, "bands samples: {bands}");
    assert!(
        sinks >= 1,
        "the sink Record is published once per generation"
    );
    assert!(!levels.is_empty(), "the level Record");
    assert!(levels[0].1, "opens silent: {levels:?}");
    let a =
        gridwatch_app::shot_replay(registry(), &path, 30.0, 250, 70, "mono", 2, "cells").unwrap();
    let b =
        gridwatch_app::shot_replay(registry(), &path, 30.0, 250, 70, "mono", 2, "cells").unwrap();
    assert_eq!(hash(&a), hash(&b), "two shots of the fixture differ");
    assert!(a.contains("Hz"), "the axis on page 2:\n{a}");
    if levels.iter().all(|(_, s)| *s) {
        assert!(a.contains("silent"), "the silence note:\n{a}");
    }
}
