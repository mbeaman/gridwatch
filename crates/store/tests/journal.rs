//! Journal gate tests (§4.5, §12.1, D47): the line format is a seam. Every
//! catalogued Record type round-trips, every datum kind round-trips, statuses,
//! alerts and inputs survive, unknown names are skipped exactly once, the
//! recorder tees without stalling, and `JournalSource` re-emits a file through
//! the normal channels on the virtual clock.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::journal::{Entry, JOURNAL, encode, encode_at};
use gridwatch_store::keys::{audio, cpu, gpu, pins, sys};
use gridwatch_store::*;

fn exemplar(name: &str) -> Option<Arc<dyn RecordValue>> {
    Some(match name {
        "cpu.breakdown" => Arc::new(cpu::CoreBreakdown {
            nice: 0.02,
            user: 0.5,
            kernel: 0.2,
            virt: 0.01,
            iowait: 0.03,
        }),
        "cpu.topology" => Arc::new(demo::CpuSynth::topology()),
        "proc.table" => Arc::new(demo::proc_table(3, 3)),
        "gpu.throttle" => Arc::new(gpu::Throttle {
            bits: gpu::Throttle::SW_POWER_CAP,
        }),
        "gpu.info" => Arc::new(gridwatch_store::demo::GpuSynth::info_exemplar()),
        "gpu.procs" => Arc::new(demo::gpu_procs(3, 3)),
        "pins.info" => Arc::new(demo::pins_info()),
        "sensor.info" => Arc::new(demo::sensors_info()),
        "media.now" => Arc::new(demo::MediaSynth::now_at(Ts(5_000_000_000))),
        "media.players" => Arc::new(gridwatch_store::keys::media::Players {
            list: vec![gridwatch_store::keys::media::PlayerInfo {
                bus: gridwatch_store::demo::MEDIA_BUS.into(),
                identity: "Demo Player".into(),
                status: gridwatch_store::keys::media::PlayStatus::Playing,
                is_current: true,
            }],
        }),
        // A 2x1 cover keeps the journal line small; the real Record is
        // 64x64 (the synth) or up to 256 px (the source).
        "media.art" => Arc::new(gridwatch_store::keys::media::Art {
            track: 7,
            w: 2,
            h: 1,
            rgb: vec![1, 2, 3, 4, 5, 6],
        }),
        "media.history" => Arc::new(gridwatch_store::keys::media::History {
            tracks: vec![gridwatch_store::keys::media::HistoryItem {
                track: 7,
                title: "Short Interlude".into(),
                artist: "Demo Set".into(),
                at: Ts(1_000_000_000),
            }],
        }),
        "audio.sink" => Arc::new(demo::audio_sink()),
        "audio.sinks" => Arc::new(audio::AudioSinks {
            sinks: vec![demo::audio_sink()],
        }),
        "audio.level" => Arc::new(audio::AudioLevel {
            silent: true,
            since: Ts(1_500_000_000),
        }),
        "pins.state" => Arc::new(pins::PinsState {
            telemetry_lost: false,
            misses: 0,
            active: vec![pins::ActiveCondition {
                id: "overload".into(),
                detail: "OVERLOAD pins 1+2 >9.2A".into(),
                since: Ts(21_500_000_000),
            }],
            service_active: vec![],
        }),
        _ => return None,
    })
}

/// D47: `decode(to_json(x)) == x` for **every** `KeyMeta` with `kind ==
/// Record`. A new Record type must add an exemplar here or this test fails —
/// that is the point.
#[test]
fn every_catalogued_record_round_trips_through_the_journal() {
    let mut seen = 0;
    for meta in CATALOGUE.iter().flat_map(|d| d.iter()) {
        if meta.kind != DatumKind::Record {
            assert!(meta.decode.is_none(), "{} is not a Record", meta.name);
            continue;
        }
        seen += 1;
        let value = exemplar(meta.name)
            .unwrap_or_else(|| panic!("no exemplar for Record key `{}`", meta.name));
        let decode = meta
            .decode
            .unwrap_or_else(|| panic!("{} has no decoder", meta.name));
        let revived = decode(value.to_json()).expect("decodes");
        assert_eq!(
            revived.to_json(),
            value.to_json(),
            "{} changed through the journal",
            meta.name
        );
        // And through a whole batch line, where the JSON *type* picks the kind.
        let msg = Msg::Batch(Batch {
            source: meta.source,
            at: Ts(1_500_000_000),
            samples: vec![Sample {
                id: Key::<f64>::new(meta.name).id,
                datum: Datum::Record(value.clone()),
            }],
        });
        let line = encode_at(Ts(1_500_000_000), &msg, true).unwrap();
        let Entry::Msg(t, Msg::Batch(b)) = journal::decode(&line).unwrap() else {
            panic!("not a batch");
        };
        assert_eq!(t, Ts(1_500_000_000));
        assert_eq!(b.samples.len(), 1, "{}: {line}", meta.name);
        match &b.samples[0].datum {
            Datum::Record(r) => assert_eq!(r.to_json(), value.to_json()),
            other => panic!("{} came back as {other:?}", meta.name),
        }
    }
    assert!(seen >= 8, "the catalogue lost its Record keys");
}

#[test]
fn scalars_vectors_and_labels_round_trip() {
    let name: Arc<str> = Arc::from("k10temp:Tccd1");
    let msg = Msg::Batch(Batch {
        source: cpu::SOURCE,
        at: Ts(7),
        samples: vec![
            Sample {
                id: cpu::CORE_PCT.idx(3).id,
                datum: Datum::Scalar(12.5),
            },
            Sample {
                id: cpu::TEMP_C.named(&name).id,
                datum: Datum::Scalar(61.25),
            },
            Sample {
                id: cpu::TOTAL_PCT.id.clone(),
                datum: Datum::Scalar(f64::NAN),
            },
        ],
    });
    let line = encode_at(Ts(7), &msg, true).unwrap();
    assert!(line.contains(r#"["cpu.core_pct{3}",12.5]"#), "{line}");
    assert!(line.contains(r#"["sensor.temp_c{k10temp:Tccd1}",61.25]"#));
    assert!(line.contains(r#"["cpu.total_pct",null]"#), "NaN is null");
    let Entry::Msg(_, Msg::Batch(b)) = journal::decode(&line).unwrap() else {
        panic!()
    };
    // The NaN sample revives as NaN — never as a fabricated 0, never dropped.
    assert_eq!(b.samples.len(), 3);
    assert_eq!(b.samples[0].id, cpu::CORE_PCT.idx(3).id);
    assert!(matches!(b.samples[0].datum, Datum::Scalar(v) if v == 12.5));
    assert_eq!(b.samples[1].id.label, Label::Name(name));
    assert!(matches!(b.samples[2].datum, Datum::Scalar(v) if v.is_nan()));
    // Labels: an index parses back as an index, a name as a name.
    assert_eq!(parse_name("cpu.core_pct{3}").1, Label::Index(3));
    assert_eq!(parse_name("x{}").1, Label::Name(Arc::from("")));
    assert_eq!(
        parse_name("gpu.fan_pct{0:1}").1,
        Label::Name(Arc::from("0:1"))
    );
    assert_eq!(parse_name("mem.total_b").1, Label::None);
}

#[test]
fn status_alert_and_input_lines_round_trip() {
    let st = SourceStatus {
        state: SourceState::Degraded,
        reason: Some(Arc::from("nvidia-smi fallback")),
        hint: None,
        since: Ts(9),
        last_sample: Some(Ts(8)),
        dropped: 4,
        restarts: 2,
    };
    let line = encode(&Msg::Control(ControlMsg::Status(cpu::SOURCE, st.clone()))).unwrap();
    assert!(line.starts_with(r#"{"t":9,"st":{"#), "{line}");
    let Entry::Msg(t, Msg::Control(ControlMsg::Status(id, got))) = journal::decode(&line).unwrap()
    else {
        panic!()
    };
    assert_eq!((t, id), (Ts(9), cpu::SOURCE));
    assert_eq!(got.state, st.state);
    assert_eq!(got.reason, st.reason);
    assert_eq!((got.dropped, got.restarts), (4, 2));
    assert_eq!(got.since, Ts(9), "since is the line's t");

    let ev = AlertEvent {
        id: AlertId::new("pins/overload"),
        source: cpu::SOURCE,
        severity: Severity::Crit,
        transition: Transition::Raised,
        title: Arc::from("pin 3 over 9.2 A"),
        detail: Arc::from("9.4 A"),
        at: Ts(11),
    };
    let line = encode(&Msg::Control(ControlMsg::Alert(ev.clone()))).unwrap();
    let Entry::Msg(_, Msg::Control(ControlMsg::Alert(got))) = journal::decode(&line).unwrap()
    else {
        panic!()
    };
    assert_eq!(got, ev);

    let input = InputEvent::Key(KeyEvent::ch('z'));
    let line = encode_at(Ts(12), &Msg::Input(input.clone()), true).unwrap();
    let Entry::Msg(t, Msg::Input(got)) = journal::decode(&line).unwrap() else {
        panic!()
    };
    assert_eq!((t, got), (Ts(12), input));

    // Messages the journal does not carry.
    assert!(encode(&Msg::Heartbeat).is_none());
    assert!(
        encode(&Msg::Control(ControlMsg::Done(ActionId(1), Ok("x".into())))).is_none(),
        "Done is not journaled"
    );
}

#[test]
fn tables_off_omits_only_the_process_tables() {
    let msg = Msg::Batch(Batch {
        source: cpu::SOURCE,
        at: Ts(1),
        samples: vec![
            Sample {
                id: cpu::PROC_TABLE.id.clone(),
                datum: Datum::Record(Arc::new(demo::proc_table(0, 1))),
            },
            Sample {
                id: sys::TASKS_KERNEL.id.clone(),
                datum: Datum::Scalar(431.0),
            },
        ],
    });
    let off = encode_at(Ts(1), &msg, false).unwrap();
    assert!(!off.contains("proc.table"));
    assert!(off.contains("tasks.kernel"), "{off}");
    let on = encode_at(Ts(1), &msg, true).unwrap();
    assert!(on.contains("proc.table"));
}

#[test]
fn unknown_names_are_skipped_once_and_unknown_sources_fail() {
    let text = r#"{"v":1,"wall_epoch":1,"host":"t","size":[80,24],"sources":["cpu","audio"]}
{"t":1,"b":{"src":"cpu","s":[["future.bands{0}",[0.1,0.2]],["cpu.total_pct",4.0]]}}
{"t":2,"b":{"src":"cpu","s":[["future.bands{0}",[0.3]],["cpu.total_pct","not a number"]]}}
"#;
    let r = Replay::parse(text).unwrap();
    assert_eq!(r.header.as_ref().unwrap().sources, vec!["cpu", "audio"]);
    assert_eq!(r.entries.len(), 2);
    // `future.bands` has no catalogue row → unknown (once); `cpu.total_pct`
    // is a known key with a datum this build cannot revive → undecodable,
    // and never reported as unknown.
    let expected: BTreeSet<String> = ["future.bands{0}"].into_iter().map(String::from).collect();
    assert_eq!(r.unknown, expected, "one entry per skipped name");
    let mut dec = Decoder::default();
    for line in text.lines().skip(1) {
        let _ = dec.decode(line);
    }
    assert_eq!(dec.undecodable, 1);
    // A truncated tail is skipped and counted, not fatal (a SIGKILLed run).
    let truncated = format!("{text}{{\"t\":3,\"b\":{{\"src\":\"cpu\",\"s\":[[\"cpu.to");
    let r = Replay::parse(&truncated).unwrap();
    assert_eq!((r.entries.len(), r.malformed), (2, 1));
    // Entries are ordered by `t` whatever the file order.
    let shuffled = r#"{"v":1,"wall_epoch":1,"host":"t","size":[80,24],"sources":["cpu"]}
{"t":251909677,"st":{"src":"cpu","state":"Ok","reason":null,"hint":null,"dropped":0,"restarts":0}}
{"t":1900287,"b":{"src":"cpu","s":[["cpu.total_pct",4.0]]}}
"#;
    let mut r = Replay::parse(shuffled).unwrap();
    assert_eq!(r.entries[0].0, Ts(1_900_287));
    let mut store = Store::default();
    assert_eq!(
        r.apply_until(Ts(2_000_000), &mut store),
        1,
        "the early batch is not hidden"
    );
    let bad = r#"{"t":1,"b":{"src":"nope","s":[]}}"#;
    assert!(
        journal::decode(bad).is_err(),
        "an unknown source is an error"
    );
    assert!(intern_source("cpu").is_some());
    assert_eq!(intern_source("journal"), Some(JOURNAL));
    assert!(intern_source("nope").is_none());
    let v2 = r#"{"v":2,"wall_epoch":1,"host":"t","size":[80,24],"sources":[]}"#;
    assert!(journal::decode(v2).is_err(), "a newer version is refused");
}

#[test]
fn replay_apply_until_is_ordered_and_resumable() {
    let mut synth = demo::CpuSynth::new(5);
    let mut lines = vec![Header::new("t", (250, 70), vec!["cpu".into()]).encode()];
    for i in 1..=4u64 {
        let at = Ts(i * 1_500_000_000);
        lines.push(encode(&Msg::Batch(synth.tick(at))).unwrap());
    }
    let mut r = Replay::parse(&lines.join("\n")).unwrap();
    let mut store = Store::default();
    assert_eq!(r.apply_until(Ts(3_000_000_000), &mut store), 2);
    assert_eq!(store.generation(cpu::SOURCE), 2);
    assert_eq!(store.latest(), Ts(3_000_000_000));
    assert_eq!(
        r.apply_until(Ts(3_000_000_000), &mut store),
        0,
        "no re-apply"
    );
    assert_eq!(r.apply_all(&mut store), 2);
    assert_eq!(store.generation(cpu::SOURCE), 4);
    assert_eq!(r.end(), Ts(6_000_000_000));
}

#[test]
fn recorder_tees_to_a_file_and_replay_reproduces_the_store() {
    let dir = std::env::temp_dir().join(format!("gw-journal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rec.jsonl");
    let header = Header::new("torch", (250, 70), vec!["cpu".into()]);
    let rec = Recorder::start(
        &path,
        &header,
        RecordOpts {
            tables: false,
            input: false,
        },
    )
    .unwrap();
    let mut synth = demo::CpuSynth::new(9);
    let mut live = Store::default();
    for i in 1..=5u64 {
        let at = Ts(i * 1_500_000_000);
        let msg = Msg::Batch(synth.tick_at(at, Detail::Table));
        live.apply(&msg);
        rec.record(at, &msg);
    }
    rec.record(Ts(1), &Msg::Input(InputEvent::Key(KeyEvent::ch('q'))));
    rec.record(Ts(1), &Msg::Heartbeat);
    assert_eq!(rec.dropped(), 0);
    rec.finish().unwrap();

    let mut r = Replay::load(&path).unwrap();
    assert_eq!(r.header.as_ref().unwrap().host, "torch");
    assert_eq!(r.entries.len(), 5, "no input line without --record-input");
    let mut replayed = Store::default();
    r.apply_all(&mut replayed);
    assert_eq!(
        replayed.generation(cpu::SOURCE),
        live.generation(cpu::SOURCE)
    );
    assert_eq!(
        replayed.last(&cpu::TOTAL_PCT),
        live.last(&cpu::TOTAL_PCT),
        "the last total survives"
    );
    assert!(
        replayed.record(&cpu::PROC_TABLE).is_none(),
        "tables off: no proc.table on replay"
    );
    assert!(replayed.record(&cpu::TOPOLOGY).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// D47 seam 2: the replay source drives the virtual clock and re-emits every
/// line through the normal channels; the receiving side is the ordinary
/// `Inbox`, so nothing downstream can tell replay from live.
#[test]
fn journal_source_replays_through_the_channels_on_the_virtual_clock() {
    let dir = std::env::temp_dir().join(format!("gw-journal-src-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("src.jsonl");
    let mut synth = demo::CpuSynth::new(2);
    let mut lines = vec![Header::new("t", (80, 24), vec!["cpu".into()]).encode()];
    let status = SourceStatus {
        state: SourceState::Ok,
        reason: None,
        hint: None,
        since: Ts(1),
        last_sample: None,
        dropped: 0,
        restarts: 0,
    };
    lines.push(encode(&Msg::Control(ControlMsg::Status(cpu::SOURCE, status))).unwrap());
    for i in 1..=3u64 {
        let at = Ts(i * 1_000_000_000);
        lines.push(encode(&Msg::Batch(synth.tick(at))).unwrap());
    }
    lines.push(encode_at(Ts(3_500_000_000), &Msg::Input(InputEvent::FocusLost), true).unwrap());
    std::fs::write(&path, lines.join("\n")).unwrap();

    let (ch, inbox) = channels();
    let clock = Clock::new_virtual();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (_ctl_tx, ctl_rx) = std::sync::mpsc::channel();
    let cx = SourceCtx::new(
        JOURNAL,
        ch,
        clock.clone(),
        stop,
        Arc::new(Demand::default()),
        ctl_rx,
        toml::Table::new(),
        0,
    );
    let src = Box::new(JournalSource::new(&path, 0.0));
    let t = std::thread::spawn(move || src.run(cx));
    t.join().unwrap();
    assert_eq!(
        clock.now(),
        Ts(3_500_000_000),
        "the clock followed the last line"
    );
    let batches: Vec<Batch> = inbox.data.try_iter().collect();
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[2].at, Ts(3_000_000_000));
    let controls: Vec<ControlMsg> = inbox.control.try_iter().collect();
    // The replayed cpu status, framed by the journal source's own Ok/Stopped.
    assert!(
        controls
            .iter()
            .any(|c| matches!(c, ControlMsg::Status(id, s) if *id == cpu::SOURCE && s.state == SourceState::Ok))
    );
    assert!(
        controls
            .iter()
            .any(|c| matches!(c, ControlMsg::Status(id, s) if *id == JOURNAL && s.state == SourceState::Stopped))
    );
    let inputs: Vec<InputEvent> = inbox.input.try_iter().collect();
    assert_eq!(inputs, vec![InputEvent::FocusLost]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = Duration::ZERO;
}
