//! Store crate gate tests (§12.1).

use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::keys::cpu;
use gridwatch_store::*;

fn scalar(key: &Key<f64>, v: f64) -> Sample {
    Sample {
        id: key.id.clone(),
        datum: Datum::Scalar(v),
    }
}

fn batch(at_ms: u64, samples: Vec<Sample>) -> Msg {
    Msg::Batch(Batch {
        source: cpu::SOURCE,
        at: Ts(at_ms * 1_000_000),
        samples,
    })
}

#[test]
fn ring_evicts_and_prunes() {
    let mut r = ring::Ring::new(3);
    for i in 0..5 {
        r.push(i);
    }
    assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    r.prune_front(|v| *v < 4);
    assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![4]);
}

#[test]
fn apply_bumps_generation_and_serves_last() {
    let mut store = Store::default();
    assert_eq!(store.generation(cpu::SOURCE), 0);
    store.apply(&batch(1000, vec![scalar(&cpu::TOTAL_PCT, 40.0)]));
    store.apply(&batch(2000, vec![scalar(&cpu::TOTAL_PCT, 60.0)]));
    assert_eq!(store.generation(cpu::SOURCE), 2);
    let (t, v) = store.last(&cpu::TOTAL_PCT).unwrap();
    assert_eq!(t, Ts(2_000_000_000));
    assert!((v - 60.0).abs() < f64::EPSILON);
    assert_eq!(store.last_sample(cpu::SOURCE), Some(Ts(2_000_000_000)));
}

#[test]
fn retention_caps_length_and_age() {
    let mut store = Store::new(Retention {
        max_len: 4,
        max_age: Duration::from_secs(5),
    });
    for i in 0..10u64 {
        store.apply(&batch(i * 1000, vec![scalar(&cpu::TOTAL_PCT, i as f64)]));
    }
    let pts: Vec<_> = store
        .window(&cpu::TOTAL_PCT, Duration::from_secs(60))
        .collect();
    assert_eq!(pts.len(), 4);
    assert_eq!(pts.first().unwrap().1, 6.0);
    store.apply(&batch(100_000, vec![scalar(&cpu::TOTAL_PCT, 99.0)]));
    let pts: Vec<_> = store
        .window(&cpu::TOTAL_PCT, Duration::from_secs(600))
        .collect();
    assert_eq!(pts.len(), 1);
}

#[test]
fn resample_buckets_align_and_gap_is_none() {
    let mut store = Store::default();
    for i in 1..=8u64 {
        if i == 5 {
            continue; // gap
        }
        store.apply(&batch(i * 1000, vec![scalar(&cpu::TOTAL_PCT, i as f64)]));
    }
    let mut out = Vec::new();
    store.resample(
        &cpu::TOTAL_PCT,
        Duration::from_secs(8),
        8,
        Agg::Last,
        &mut out,
    );
    assert_eq!(out.len(), 8);
    assert_eq!(out[0], None);
    assert_eq!(out[1], Some(1.0));
    assert_eq!(out[5], None);
    assert_eq!(out[7], Some(7.0));
    let mut store2 = Store::default();
    store2.apply(&batch(500, vec![scalar(&cpu::TOTAL_PCT, 10.0)]));
    store2.apply(&batch(700, vec![scalar(&cpu::TOTAL_PCT, 20.0)]));
    store2.apply(&batch(4000, vec![scalar(&cpu::TOTAL_PCT, 1.0)]));
    let mut out2 = Vec::new();
    store2.resample(
        &cpu::TOTAL_PCT,
        Duration::from_secs(4),
        4,
        Agg::Avg,
        &mut out2,
    );
    assert_eq!(out2[0], Some(15.0));
}

#[test]
fn labels_iterate_in_deterministic_order() {
    let mut store = Store::default();
    let mut samples = Vec::new();
    for i in [3u16, 0, 2, 1] {
        samples.push(Sample {
            id: cpu::CORE_PCT.idx(i).id,
            datum: Datum::Scalar(f64::from(i)),
        });
    }
    let b: Arc<str> = Arc::from("b");
    let a: Arc<str> = Arc::from("a");
    samples.push(Sample {
        id: cpu::TEMP_C.named(&b).id,
        datum: Datum::Scalar(1.0),
    });
    samples.push(Sample {
        id: cpu::TEMP_C.named(&a).id,
        datum: Datum::Scalar(2.0),
    });
    store.apply(&batch(1000, samples));
    let idx: Vec<String> = store
        .labels("cpu.core_pct")
        .map(|l| format!("{l}"))
        .collect();
    assert_eq!(idx, vec!["{0}", "{1}", "{2}", "{3}"]);
    let named: Vec<String> = store
        .labels("sensor.temp_c")
        .map(|l| format!("{l}"))
        .collect();
    assert_eq!(named, vec!["{a}", "{b}"]);
}

#[test]
fn record_roundtrip_via_downcast() {
    let mut store = Store::default();
    let table = cpu::ProcTable {
        rows: vec![],
        pid_digits: 7,
    };
    store.apply(&Msg::Batch(Batch {
        source: cpu::SOURCE,
        at: Ts(1),
        samples: vec![Sample {
            id: cpu::PROC_TABLE.id.clone(),
            datum: Datum::Record(Arc::new(table.clone())),
        }],
    }));
    let (_, got) = store.record(&cpu::PROC_TABLE).unwrap();
    assert_eq!(got, &table);
}

#[test]
fn catalogue_covers_every_emitted_key_and_decodes_records() {
    let mut synth = demo::CpuSynth::new(7);
    let b = synth.tick(Ts(1_000_000_000));
    for s in &b.samples {
        assert!(
            lookup(s.id.name).is_some(),
            "{} missing from CATALOGUE",
            s.id.name
        );
    }
    let meta = lookup("cpu.breakdown").unwrap();
    let rec = cpu::CoreBreakdown {
        nice: 0.1,
        user: 0.5,
        kernel: 0.2,
        virt: 0.0,
        iowait: 0.1,
    };
    let revived = (meta.decode.unwrap())(rec.to_json()).unwrap();
    assert_eq!(
        revived.as_any().downcast_ref::<cpu::CoreBreakdown>(),
        Some(&rec)
    );

    // Every Record in the catalogue round-trips, not just the first one (§4.5).
    let topo = demo::CpuSynth::topology();
    let meta = lookup("cpu.topology").unwrap();
    let revived = (meta.decode.unwrap())(topo.to_json()).unwrap();
    assert_eq!(
        revived.as_any().downcast_ref::<cpu::CpuTopology>(),
        Some(&topo),
        "cpu.topology must survive the journal"
    );
    let table = cpu::ProcTable::default();
    let meta = lookup("proc.table").unwrap();
    let revived = (meta.decode.unwrap())(table.to_json()).unwrap();
    assert_eq!(
        revived.as_any().downcast_ref::<cpu::ProcTable>(),
        Some(&table)
    );
    // And the map the synth publishes is torch's, so the CCD grouping the
    // `cores` tier draws is pinned here as well as in the sources tests.
    let dies = topo.dies();
    assert_eq!(dies.len(), 2);
    assert_eq!(dies[0].1[0], vec![0, 16], "SMT sibling of cpu0 is cpu16");
    assert_eq!(dies[1].1[7], vec![15, 31]);
}

#[test]
fn alert_on_control_channel_survives_full_data_channel() {
    let (ch, inbox) = channels();
    let full = Batch {
        source: cpu::SOURCE,
        at: Ts(1),
        samples: vec![],
    };
    for _ in 0..DATA_BOUND {
        ch.data.try_send(full.clone()).unwrap();
    }
    assert!(
        ch.data.try_send(full.clone()).is_err(),
        "data channel should be full"
    );
    let ev = AlertEvent {
        id: AlertId::new("pins/overload"),
        source: SourceId("pins"),
        severity: Severity::Crit,
        transition: Transition::Raised,
        title: Arc::from("pin 3 overload"),
        detail: Arc::from("9.4 A > 9.2 A"),
        at: Ts(2),
    };
    ch.control.send(ControlMsg::Alert(ev.clone())).unwrap();
    let msg = Msg::Control(inbox.control.try_recv().unwrap());
    let mut store = Store::default();
    let out = store.apply(&msg);
    assert_eq!(out.len(), 1);
    assert_eq!(store.alerts().worst_active(), Some(Severity::Crit));
    let resolved = AlertEvent {
        transition: Transition::Resolved,
        at: Ts(3),
        ..ev
    };
    store.apply(&Msg::Control(ControlMsg::Alert(resolved)));
    assert_eq!(store.alerts().worst_active(), None);
    assert_eq!(store.alerts().events().count(), 2);
}

#[test]
fn synth_is_deterministic_per_seed() {
    let run = |seed| {
        let mut s = demo::CpuSynth::new(seed);
        (0..5)
            .map(|i| s.tick(Ts(i * 1_500_000_000)))
            .collect::<Vec<_>>()
    };
    let a = run(42);
    let b = run(42);
    let c = run(43);
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.samples.len(), y.samples.len());
        for (sx, sy) in x.samples.iter().zip(&y.samples) {
            assert_eq!(sx.id, sy.id);
            if let (Datum::Scalar(vx), Datum::Scalar(vy)) = (&sx.datum, &sy.datum) {
                assert!((vx - vy).abs() < f64::EPSILON);
            }
        }
    }
    let differs =
        a[0].samples
            .iter()
            .zip(&c[0].samples)
            .any(|(sx, sy)| match (&sx.datum, &sy.datum) {
                (Datum::Scalar(vx), Datum::Scalar(vy)) => (vx - vy).abs() > f64::EPSILON,
                _ => false,
            });
    assert!(differs);
}

#[test]
fn demand_and_cadence_follow_levels() {
    let d = Demand::default();
    assert_eq!(d.level(), Level::Hidden);
    d.set(Level::Focused, Detail::Table);
    assert_eq!(d.level(), Level::Focused);
    assert_eq!(d.detail(), Detail::Table);
    let c = demo::cpu_info().cadence;
    assert_eq!(c.for_level(Level::Paused), None);
    assert_eq!(c.for_level(Level::Hidden), Some(Duration::from_secs(3)));
    assert_eq!(
        c.for_level(Level::Visible),
        Some(Duration::from_millis(1500))
    );
    let pins_like = Cadence {
        hidden: Some(Duration::from_secs(1)),
        visible: Duration::from_millis(500),
        focused: Duration::from_millis(500),
        always_on: true,
    };
    // always_on keeps alert rules fed at the *hidden* cadence — an unwatched
    // source never earns its visible budget (arc-1a review, perf-budget lens).
    assert_eq!(
        pins_like.for_level(Level::Paused),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        pins_like.for_level(Level::Hidden),
        Some(Duration::from_secs(1))
    );
}

/// Arc 7b: a `[[rules]]` entry raises through the normal alert path, so the
/// banner, the alerts tile and `a` need no change — the store's own
/// `apply` produces the event and the log records it.
#[test]
fn a_rule_raises_and_resolves_through_the_alert_log() {
    use gridwatch_store::rules::{Rules, parse_all};
    let toml_text = r#"
[[rules]]
name = "gpu-hot"
key = "gpu.temp_c"
op = ">"
value = 84
for_s = 2
clear_s = 2
severity = "crit"
message = "the gpu is {value}°C"
"#;
    #[derive(serde::Deserialize)]
    struct Wrapper {
        rules: Vec<toml::Table>,
    }
    let w: Wrapper = toml::from_str(toml_text).unwrap();
    let (rules, errors) = parse_all(&w.rules, &|k| gridwatch_store::key::lookup(k).is_some());
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rules.len(), 1);
    let mut store = Store::default();
    store.set_rules(Rules::new(rules));

    let hot = |t: u64, v: f64| {
        Msg::Batch(Batch {
            source: SourceId("gpu"),
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::gpu::TEMP_C.idx(0).id,
                datum: Datum::Scalar(v),
            }],
        })
    };
    // Hot, but inside the hold.
    assert!(store.apply(&hot(1, 90.0)).is_empty());
    assert!(store.apply(&hot(2, 90.0)).is_empty());
    // Past it: one Crit, and the log has it.
    let ev = store.apply(&hot(3, 90.0));
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].severity, gridwatch_store::Severity::Crit);
    assert_eq!(ev[0].transition, gridwatch_store::Transition::Raised);
    assert!(ev[0].detail.contains("90.0"), "{}", ev[0].detail);
    assert_eq!(store.alerts().active().count(), 1);
    // Cool: resolved after the clear hold, and the log lets it go.
    assert!(store.apply(&hot(4, 50.0)).is_empty());
    let ev = store.apply(&hot(6, 50.0));
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].transition, gridwatch_store::Transition::Resolved);
    assert_eq!(store.alerts().active().count(), 0);
    // A store with no rules costs nothing and produces nothing.
    let mut plain = Store::default();
    assert!(plain.apply(&hot(9, 200.0)).is_empty());
    assert!(plain.rules().is_empty());
}

/// Arc 7b P18: the rules' per-batch cost. Ten rules over a batch of forty
/// scalars, the shape a busy source publishes, measured the way the other
/// performance rows are — as a wall-clock number this test prints and a
/// ceiling it enforces loosely enough not to flake on a loaded machine.
#[test]
fn ten_rules_cost_microseconds_per_batch() {
    use gridwatch_store::rules::{Rules, parse_all};
    let tables: Vec<toml::Table> = (0..10)
        .map(|i| {
            toml::from_str(&format!(
                "name = \"r{i}\"\nkey = \"sensor.temp_c\"\nop = \">\"\nvalue = {}\nfor_s = 5\n",
                50 + i
            ))
            .unwrap()
        })
        .collect();
    let (rules, errors) = parse_all(&tables, &|k| gridwatch_store::key::lookup(k).is_some());
    assert!(errors.is_empty(), "{errors:?}");
    let mut store = Store::default();
    store.set_rules(Rules::new(rules));
    let samples: Vec<Sample> = (0..40)
        .map(|i| Sample {
            id: gridwatch_store::keys::sensors::TEMP_C
                .named(&Arc::from(format!("chip{i}:Sensor").as_str()))
                .id,
            datum: Datum::Scalar(60.0),
        })
        .collect();
    let n = 200;
    let t0 = std::time::Instant::now();
    for i in 0..n {
        store.apply(&Msg::Batch(Batch {
            source: SourceId("sensors"),
            at: Ts(i * 1_000_000_000),
            samples: samples.clone(),
        }));
    }
    let per_batch = t0.elapsed() / n as u32;
    println!("rules: {per_batch:?} per batch of 40 samples against 10 rules");
    assert!(
        per_batch < Duration::from_micros(500),
        "the rules cost {per_batch:?} a batch — the ceiling is 0.5 ms (P18)"
    );
    // And they raised: forty labels over ten thresholds, each once.
    assert_eq!(store.alerts().active().count(), 400);
}

/// Arc 7b, review finding: `Store::tick_rules` had no coverage — the
/// `absent` tests called the engine directly with a hand-written closure.
/// This drives the real one: a key that arrives and then stops, a key that
/// never arrives at all, and the cost of asking every frame.
#[test]
fn tick_rules_notices_a_key_that_stops_and_one_that_never_came() {
    use gridwatch_store::rules::{Rules, parse_all};
    let tables: Vec<toml::Table> = [
        r#"name = "eno1 quiet"
key = "net.rx_bps{eno1}"
op = "absent"
for_s = 10"#,
        r#"name = "wlp7s0 quiet"
key = "net.rx_bps{wlp7s0}"
op = "absent"
for_s = 10"#,
    ]
    .iter()
    .map(|t| toml::from_str(t).unwrap())
    .collect();
    let (rules, errors) = parse_all(&tables, &|k| gridwatch_store::key::lookup(k).is_some());
    assert!(errors.is_empty(), "{errors:?}");
    let mut store = Store::default();
    store.set_rules(Rules::new(rules));

    let sample = |t: u64| {
        Msg::Batch(Batch {
            source: SourceId("net"),
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::net::RX_BPS
                    .named(&Arc::from("eno1"))
                    .id,
                datum: Datum::Scalar(1000.0),
            }],
        })
    };
    // eno1 publishes; wlp7s0 never does. The first tick starts the clock.
    store.apply(&sample(100));
    assert!(store.tick_rules(Ts(100_000_000_000)).is_empty());
    assert!(store.tick_rules(Ts(105_000_000_000)).is_empty());
    // Ten seconds on: eno1 is still fresh only if it kept publishing.
    store.apply(&sample(110));
    let ev = store.tick_rules(Ts(111_000_000_000));
    assert_eq!(ev.len(), 1, "the radio that never appeared: {ev:?}");
    assert_eq!(ev[0].title.as_ref(), "wlp7s0 quiet");
    assert_eq!(ev[0].source, gridwatch_store::source::RULES);
    // Now eno1 stops too.
    let ev = store.tick_rules(Ts(121_000_000_000));
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0].title.as_ref(), "eno1 quiet");
    assert_eq!(store.alerts().active().count(), 2);
    // It comes back and resolves.
    store.apply(&sample(130));
    let ev = store.tick_rules(Ts(130_000_000_000));
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].transition, gridwatch_store::Transition::Resolved);

    // The cost of asking, with a store holding a realistic number of
    // series: this runs every frame, so it may not walk the store.
    for i in 0..2000u32 {
        store.apply(&Msg::Batch(Batch {
            source: SourceId("sensors"),
            at: Ts(200_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::sensors::TEMP_C
                    .named(&Arc::from(format!("chip{i}:Sensor").as_str()))
                    .id,
                datum: Datum::Scalar(40.0),
            }],
        }));
    }
    let n = 500;
    let t0 = std::time::Instant::now();
    for i in 0..n {
        store.tick_rules(Ts(200_000_000_000 + i * 1_000_000));
    }
    let per_tick = t0.elapsed() / n as u32;
    println!("tick_rules: {per_tick:?} per frame with 2 absent rules over 2000+ series");
    assert!(
        per_tick < Duration::from_micros(200),
        "the absent rules cost {per_tick:?} a frame — they must range-seek, not walk"
    );
    // A store whose rules are all comparisons does no per-frame work.
    let mut plain = Store::default();
    let (only_gt, _) = parse_all(
        &[toml::from_str("name = \"g\"\nkey = \"gpu.temp_c\"\nop = \">\"\nvalue = 1").unwrap()],
        &|k| gridwatch_store::key::lookup(k).is_some(),
    );
    plain.set_rules(Rules::new(only_gt));
    assert!(!plain.rules().has_absent());
    assert!(plain.tick_rules(Ts(1)).is_empty());
}

/// Arc 7b review: a config reload rebuilds the rule set, and used to lose
/// every rule's state with it — re-firing an active alert or stranding it
/// until restart. A rule that survives keeps its state; a rule that is
/// removed has its alert resolved.
#[test]
fn reloading_the_rules_keeps_what_is_raised_and_resolves_what_is_gone() {
    use gridwatch_store::rules::{Rules, parse_all};
    let known = |k: &str| gridwatch_store::key::lookup(k).is_some();
    let rule = |name: &str, value: i64| -> toml::Table {
        toml::from_str(&format!(
            "name = \"{name}\"\nkey = \"gpu.temp_c\"\nop = \">\"\nvalue = {value}\nfor_s = 1\nclear_s = 1"
        ))
        .unwrap()
    };
    let hot = |t: u64, v: f64| {
        Msg::Batch(Batch {
            source: SourceId("gpu"),
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::gpu::TEMP_C.idx(0).id,
                datum: Datum::Scalar(v),
            }],
        })
    };

    let mut store = Store::default();
    let (rules, _) = parse_all(&[rule("hot", 84), rule("warm", 40)], &known);
    assert!(store.set_rules(Rules::new(rules)).is_empty());
    store.apply(&hot(10, 90.0));
    let raised = store.apply(&hot(12, 90.0));
    assert_eq!(raised.len(), 2, "both rules hold: {raised:?}");
    assert_eq!(store.alerts().active().count(), 2);

    // A reload that keeps `hot` and drops `warm`: nothing re-raises, and
    // `warm` is resolved because nothing else ever could.
    let (rules, _) = parse_all(&[rule("hot", 84)], &known);
    let resolved = store.set_rules(Rules::new(rules));
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(resolved[0].title.as_ref(), "warm");
    assert_eq!(
        resolved[0].transition,
        gridwatch_store::Transition::Resolved
    );
    assert_eq!(store.alerts().active().count(), 1, "only `hot` is left");
    // Still hot, still raised, and it does not raise a second time.
    for t in 13..20 {
        assert!(
            store.apply(&hot(t, 90.0)).is_empty(),
            "a surviving rule re-raised at {t}s"
        );
    }
    assert_eq!(store.rules().raised(), vec![("hot".into(), "0".into())]);
    // And it can still resolve normally.
    store.apply(&hot(21, 50.0));
    let ev = store.apply(&hot(23, 50.0));
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].transition, gridwatch_store::Transition::Resolved);
    assert_eq!(store.alerts().active().count(), 0);
}

/// Arc 7b review: the config surface refuses what it cannot honour.
#[test]
fn a_rule_that_cannot_work_is_refused_at_parse_time() {
    use gridwatch_store::rules::parse_all;
    let known = |k: &str| gridwatch_store::key::lookup(k).is_some();
    let problem = |text: &str| -> String {
        let (rules, errors) = parse_all(&[toml::from_str(text).unwrap()], &known);
        assert!(rules.is_empty(), "{text} should not have parsed");
        errors[0].problem.clone()
    };
    // `absent` against a frame clock is always true without a hold.
    assert!(
        problem("name = \"x\"\nkey = \"gpu.temp_c\"\nop = \"absent\"").contains("for_s"),
        "an absent rule with no hold must be refused"
    );
    // A hold that is not a number was silently becoming zero.
    assert!(
        problem("name = \"x\"\nkey = \"gpu.temp_c\"\nop = \">\"\nvalue = 1\nfor_s = \"30\"")
            .contains("seconds"),
    );
    assert!(
        problem("name = \"x\"\nkey = \"gpu.temp_c\"\nop = \">\"\nvalue = 1\nclear_s = -5")
            .contains("seconds"),
    );
}
