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
        max_uncatalogued: 512,
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
    // Measured against a **control**, not against an absolute number. The
    // absolute form asserted 500 µs, took 180 µs on this box and 500.9 µs on
    // a shared CI runner in a debug build — a coin toss, for the reason D59
    // gave when it refused to make the benches a gate. What the test is
    // named for is the *marginal* cost of ten rules, and a control run over
    // the same batches with no rules scales with the runner the same way.
    let n = 200;
    let time = |store: &mut Store| {
        let t0 = std::time::Instant::now();
        for i in 0..n {
            store.apply(&Msg::Batch(Batch {
                source: SourceId("sensors"),
                at: Ts(i * 1_000_000_000),
                samples: samples.clone(),
            }));
        }
        t0.elapsed() / n as u32
    };
    let mut control = Store::default();
    let without = time(&mut control);
    let with = time(&mut store);
    let cost = with.saturating_sub(without);
    // Per rule-evaluation is the quantity that should be stable: ten rules
    // over forty labelled samples is 400 of them a batch.
    let each = cost / 400;
    println!(
        "rules: {with:?} per batch of 40 samples against 10 rules, {without:?} with none — \
         the rules cost {cost:?}, {each:?} per evaluation"
    );
    // 5 µs is ~11x this box's debug figure (0.45 µs) and ~4x a shared CI
    // runner's. The old assertion was 500 µs against a batch measuring 180 µs
    // here and **500.9 µs on CI** — under 3x headroom for a debug build on a
    // two-core shared runner, which is not a gate but a coin toss. A real
    // regression in the rules engine is an order of magnitude, not 2 %, and
    // this still catches one. PERFORMANCE.md's 24 µs is the *release* figure.
    assert!(
        each < Duration::from_micros(5),
        "a rule evaluation cost {each:?} ({cost:?} for 400 of them) — the ceiling is 5 µs"
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

// ---------------------------------------------------------------- arc 10b

/// Arc 10b (D60), corrected by the arc-10 review — retention's other half.
/// `Series::push` prunes a scalar ring by `max_age`, but pruning is
/// **push-driven**: a series nothing publishes to any more never prunes, so
/// it kept up to `max_len` points and its map entry for the life of the
/// process. Bounded for every catalogued key except `net.*{iface}` on a
/// machine that makes and destroys interfaces.
#[test]
fn retention_shrinks_a_label_that_stopped_arriving_without_losing_its_value() {
    let mut store = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    let bps = |iface: &str, t: u64, v: f64| {
        Msg::Batch(Batch {
            source: SourceId("net"),
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::net::RX_BPS
                    .named(&Arc::from(iface))
                    .id,
                datum: Datum::Scalar(v),
            }],
        })
    };
    let veth = gridwatch_store::keys::net::RX_BPS.named(&Arc::from("veth9a1"));
    let all = Duration::from_secs(u32::MAX as u64);
    let stored = |s: &Store| s.window(&veth, all).count();

    // A permanent interface and a container's veth, both publishing.
    for t in 0..40 {
        store.apply(&bps("eno1", t, 1000.0));
        store.apply(&bps("veth9a1", t, 20.0));
    }
    assert_eq!(stored(&store), 40);

    // The container goes away; eno1 keeps publishing. Nothing is dropped
    // while the veth's points are still inside the window a chart draws.
    for t in 40..90 {
        store.apply(&bps("eno1", t, 1000.0));
    }
    assert!(
        stored(&store) > 1,
        "a merely quiet series is still on the chart and must keep its points"
    );

    // Far past `max_age` the label is dead — nothing in the `net` domain has
    // published to `veth9a1` for longer than retention — and `net.*` keys
    // are `Dynamic` (D61), so the series is removed outright rather than
    // shrunk: the interface is gone, and a tile listing labels must not
    // show it. (Arc 10b shrank it to one point; D61 lets the catalogue say
    // which labels can die.)
    for t in 90..300 {
        store.apply(&bps("eno1", t, 1000.0));
    }
    assert_eq!(stored(&store), 0, "a dead dynamic label holds no series");
    assert!(store.last(&veth).is_none());
    assert!(
        !store
            .labels("net.rx_bps")
            .map(gridwatch_store::rules::label_text)
            .any(|l| l == "veth9a1"),
        "the label is gone from `labels()`, which is what the net tile lists"
    );
    // The permanent interface is untouched.
    let eno1 = gridwatch_store::keys::net::RX_BPS.named(&Arc::from("eno1"));
    assert_eq!(store.last(&eno1).map(|(_, v)| v), Some(1000.0));
}

/// D61's rule is per **label**, never per series — the trap R6 fell into.
/// `net.speed_mbps{eno1}` and `net.link{eno1}` are published once for an
/// interface that lives forever; they survive because `rx_bps{eno1}` keeps
/// arriving. The veth's copies of the same keys go with its label.
#[test]
fn a_dead_dynamic_label_is_removed_whole_and_a_live_one_keeps_its_once_published_series() {
    use gridwatch_store::keys::net::{self, Link};
    let mut store = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    let at = |t: u64| Ts(t * 1_000_000_000);
    let net_batch = |t: u64, samples: Vec<Sample>| {
        Msg::Batch(Batch {
            source: net::SOURCE,
            at: at(t),
            samples,
        })
    };
    let once = |iface: &str| {
        vec![
            scalar(&net::SPEED_MBPS.named(&Arc::from(iface)), 2500.0),
            Sample {
                id: net::LINK.named(&Arc::from(iface)).id,
                datum: Datum::Record(Arc::new(Link::default())),
            },
        ]
    };
    store.apply(&net_batch(0, once("eno1")));
    store.apply(&net_batch(0, once("veth9a1")));
    for t in 0..40 {
        store.apply(&net_batch(
            t,
            vec![
                scalar(&net::RX_BPS.named(&Arc::from("eno1")), 1000.0),
                scalar(&net::RX_BPS.named(&Arc::from("veth9a1")), 20.0),
            ],
        ));
    }
    for t in 40..300 {
        store.apply(&net_batch(
            t,
            vec![scalar(&net::RX_BPS.named(&Arc::from("eno1")), 1000.0)],
        ));
    }
    let eno1 = Arc::from("eno1");
    let veth = Arc::from("veth9a1");
    assert_eq!(
        store.last(&net::SPEED_MBPS.named(&eno1)).map(|(_, v)| v),
        Some(2500.0),
        "published once, 300 s ago, for a label that is alive: kept"
    );
    assert!(store.record(&net::LINK.named(&eno1)).is_some());
    assert!(store.last(&net::SPEED_MBPS.named(&veth)).is_none());
    assert!(store.last(&net::RX_BPS.named(&veth)).is_none());
    assert!(
        store.record(&net::LINK.named(&veth)).is_none(),
        "a Record goes with its dead label — the kind does not matter"
    );
    assert!(
        !store
            .labels("net.link")
            .map(gridwatch_store::rules::label_text)
            .any(|l| l == "veth9a1")
    );
}

/// A `Static` key's labels are cores, devices, pins and channels: a quiet
/// one is a core that reported nothing, not a core that left, and its
/// series is never removed.
#[test]
fn a_static_key_never_loses_a_quiet_label() {
    let mut store = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    for t in 0..10u64 {
        store.apply(&batch(
            t * 1000,
            vec![
                scalar(&cpu::CORE_PCT.idx(0), 10.0),
                scalar(&cpu::CORE_PCT.idx(7), 90.0),
            ],
        ));
    }
    for t in 10..300u64 {
        store.apply(&batch(t * 1000, vec![scalar(&cpu::CORE_PCT.idx(0), 10.0)]));
    }
    assert_eq!(
        store.last(&cpu::CORE_PCT.idx(7)).map(|(_, v)| v),
        Some(90.0),
        "quiet for 290 s, shrunk to one point, still there"
    );
    assert!(store.labels("cpu.core_pct").any(|l| *l == Label::Index(7)));
}

/// An uncatalogued name is a plugin's: its labels are `Dynamic`, so they are
/// evicted like `net`'s, and `Retention::max_uncatalogued` refuses a sample
/// that would create one series more than the domain may hold — until the
/// sweep gives the room back. `capped` counts what was refused and is never
/// reset. A `Label::None` series is bounded by the name cap and is kept.
/// Trap 3 (D61): liveness is per `(domain, label)`, and the domain is the
/// name's prefix before the first `.` — not the batch's source and not
/// `KeyMeta.source`. Every catalogued pair agrees on all three, so only an
/// uncatalogued name can tell them apart: a plugin's `weather.temp{eno1}`
/// goes quiet while `net.rx_bps{eno1}` keeps arriving under the *same label
/// text*. A rule keyed on the label alone would keep the plugin's series
/// alive off the back of the interface's traffic (arc 11 review).
#[test]
fn a_label_that_is_alive_in_one_domain_does_not_keep_another_alive() {
    let mut store = Store::new(Retention {
        max_len: 64,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    let iface: Arc<str> = Arc::from("eno1");
    let weather: Key<f64> = Key::new("weather.temp");
    // Both domains carry the label at t = 0.
    store.apply(&Msg::Batch(Batch {
        source: SourceId("weather"),
        at: Ts(0),
        samples: vec![scalar(&weather.named(&iface), 21.0)],
    }));
    // Only `net` keeps publishing it, for well past `max_age`.
    for t in 0..200u64 {
        store.apply(&Msg::Batch(Batch {
            source: SourceId("net"),
            at: Ts(t * 1_000_000_000),
            samples: vec![scalar(
                &gridwatch_store::keys::net::RX_BPS.named(&iface),
                1000.0,
            )],
        }));
    }
    assert!(
        store
            .last(&gridwatch_store::keys::net::RX_BPS.named(&iface))
            .is_some(),
        "the live interface must survive its own traffic"
    );
    assert!(
        store.last(&weather.named(&iface)).is_none(),
        "`weather.temp{{eno1}}` went quiet at t=0 and must be evicted — it is only \
         still here if liveness is keyed on the label without its domain"
    );
}

#[test]
fn an_uncatalogued_domain_is_capped_and_eviction_frees_the_room() {
    let mut store = Store::new(Retention {
        max_len: 64,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 3,
    });
    let weather = SourceId("weather");
    let city = |c: &str, t: u64| {
        Msg::Batch(Batch {
            source: weather,
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: MetricId {
                    name: "weather.temp",
                    label: Label::Name(Arc::from(c)),
                },
                datum: Datum::Scalar(20.0),
            }],
        })
    };
    let sun: Key<f64> = Key::new("weather.sun");
    store.apply(&Msg::Batch(Batch {
        source: weather,
        at: Ts(0),
        samples: vec![scalar(&sun, 1.0)],
    }));
    for c in ["c1", "c2", "c3", "c4"] {
        store.apply(&city(c, 0));
    }
    // The cap is on *series* of the domain, and `weather.sun` is one of
    // them: two cities fit beside it, the third and fourth are refused.
    assert_eq!(
        store.labels("weather.temp").count(),
        2,
        "the third is refused"
    );
    assert_eq!(store.capped(weather), 2);
    assert_eq!(store.capped(cpu::SOURCE), 0);
    // Nothing more from the plugin; the cpu source drives the clock past
    // retention, and the sweep evicts the dead plugin labels.
    for t in 1..300u64 {
        store.apply(&batch(t * 1000, vec![scalar(&cpu::TOTAL_PCT, 5.0)]));
    }
    assert_eq!(store.labels("weather.temp").count(), 0);
    assert_eq!(
        store.last(&sun).map(|(_, v)| v),
        Some(1.0),
        "an unlabelled series is one per name and is never removed"
    );
    store.apply(&city("c5", 300));
    assert_eq!(store.labels("weather.temp").count(), 1, "room was freed");
    assert_eq!(
        store.capped(weather),
        2,
        "never reset — the count is the evidence"
    );
}

/// The sweep runs on **store time**, so a replay shrinks at exactly the same
/// message the live run did — arc 2a's determinism test compares frame hashes
/// across two replays, and a wall-clock sweep would make that a coin toss.
///
/// The oracle is the **whole surviving inventory**, not two counts: every
/// label of every key the run touched with the number of points it kept, so
/// a sweep that evicted a different label — or the same label at a different
/// message — is a difference rather than a coincidence. This is the arc-11
/// replay-determinism case (ROADMAP arc 11): the run crosses six sweep
/// boundaries and the last of them evicts.
#[test]
fn the_sweep_is_driven_by_store_time_not_the_wall_clock() {
    // Two interfaces and a threshold published once for each, so a
    // per-series rule and a per-label rule would disagree — and so would two
    // runs, if either depended on the wall clock.
    let inventory = |store: &Store| -> Vec<(String, usize)> {
        let all = Duration::from_secs(u32::MAX as u64);
        let mut out = Vec::new();
        for name in ["net.rx_bps", "net.speed_mbps"] {
            for label in store.labels(name).cloned().collect::<Vec<_>>() {
                let base: Key<f64> = Key::new(name);
                let key = match &label {
                    Label::Name(s) => base.named(s),
                    Label::Index(i) => base.idx(*i),
                    Label::None => base,
                };
                out.push((key.id.to_string(), store.window(&key, all).count()));
            }
        }
        out
    };
    let feed = |store: &mut Store| {
        for iface in ["veth9a1", "eno1"] {
            store.apply(&Msg::Batch(Batch {
                source: SourceId("net"),
                at: Ts(0),
                samples: vec![scalar(
                    &gridwatch_store::keys::net::SPEED_MBPS.named(&Arc::from(iface)),
                    2500.0,
                )],
            }));
        }
        for t in 0..300u64 {
            store.apply(&Msg::Batch(Batch {
                source: SourceId("net"),
                at: Ts(t * 1_000_000_000),
                samples: vec![Sample {
                    id: gridwatch_store::keys::net::RX_BPS
                        .named(&Arc::from(if t < 40 { "veth9a1" } else { "eno1" }))
                        .id,
                    datum: Datum::Scalar(1000.0),
                }],
            }));
        }
        inventory(store)
    };
    let retention = Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    };
    let mut a = Store::new(retention);
    let mut b = Store::new(retention);
    let first = feed(&mut a);
    assert!(
        !first.iter().any(|(id, _)| id.contains("veth9a1")),
        "the veth's label died and every series of it went, the threshold \
         published once included (D61): {first:?}"
    );
    assert!(
        first.contains(&("net.speed_mbps{eno1}".to_string(), 1)),
        "the live label keeps its once-published scalar: {first:?}"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(first, feed(&mut b), "two runs must evict identically");
}

/// A `Record` is never touched, and neither is a scalar's newest point. Six
/// catalogued scalars are published **once** — `sensor.max_c`/`crit_c`,
/// `net.speed_mbps` and the three static gpu clocks — so an age rule that
/// could empty a ring would delete them eleven minutes into any default run.
#[test]
fn a_value_published_once_survives_any_amount_of_silence() {
    let mut store = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    store.apply(&Msg::Batch(Batch {
        source: SourceId("cpu"),
        at: Ts(0),
        samples: vec![Sample {
            id: cpu::PROC_TABLE.id.clone(),
            datum: Datum::Record(Arc::new(cpu::ProcTable::default())),
        }],
    }));
    for t in 1..300u64 {
        store.apply(&batch(t * 1000, vec![scalar(&cpu::TOTAL_PCT, 10.0)]));
    }
    assert!(
        store.record(&cpu::PROC_TABLE).is_some(),
        "a publish-once record must outlive every sweep"
    );
}

/// The sweep is not free, so it runs on a retention boundary rather than per
/// `apply` — the rules pass alone measures 24 µs (arc 7b) and the sweep walks
/// every series. There is no P-row for per-batch apply cost; 500 µs is this
/// suite's own assertion, kept in step with `ten_rules_cost_microseconds_per_batch`.
/// This pins both halves: the amortised cost of a store that is sweeping, and
/// the sweep's own walk over a store far larger than torch's.
#[test]
fn the_retention_sweep_stays_inside_the_batch_budget() {
    let retention = Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    };
    let mut store = Store::new(retention);
    // 400 labelled series — torch runs about 150 with every source live.
    let samples: Vec<Sample> = (0..400)
        .map(|i| Sample {
            id: gridwatch_store::keys::sensors::TEMP_C
                .named(&Arc::from(format!("chip{i}:Sensor").as_str()))
                .id,
            datum: Datum::Scalar(60.0),
        })
        .collect();
    let n = 300u64;
    let time = |store: &mut Store| {
        let t0 = std::time::Instant::now();
        for i in 0..n {
            store.apply(&Msg::Batch(Batch {
                source: SourceId("sensors"),
                at: Ts(i * 1_000_000_000),
                samples: samples.clone(),
            }));
        }
        t0.elapsed() / n as u32
    };
    // The control never sweeps: `sweep_every` is `max_age / 10`, so a
    // max_age far past the run's store time means no boundary is crossed.
    // Comparing against it rather than against an absolute number is what
    // keeps this from being a coin toss on a shared CI runner.
    let mut control = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(100_000),
        max_uncatalogued: 512,
    });
    let without = time(&mut control);
    let with = time(&mut store);
    let cost = with.saturating_sub(without);
    println!(
        "sweep: {with:?} per batch of 400 samples sweeping every 10 s of store time,          {without:?} never sweeping — the sweep costs {cost:?}"
    );
    assert!(
        cost < Duration::from_micros(500),
        "the sweep costs {cost:?} a batch over the same work without it — the ceiling is 0.5 ms"
    );
    // Nothing was evicted: every series kept publishing.
    assert_eq!(store.labels("sensor.temp_c").count(), 400);
}

/// D61's cap is the only thing `Store::apply` gained, and it lives in the
/// `Entry::Vacant` arm — so it is paid when a series is **created** and never
/// on a push to one that exists. This measures all three paths so nobody has
/// to wonder (ROADMAP arc 11's performance gate).
///
/// `max_len` is 16 here on purpose: the shipped 2 400 makes `Ring::new`'s
/// 38 KB allocation dwarf everything the cap does, and the question is what
/// the cap costs, not what the allocator does.
#[test]
#[ignore = "diagnostic; the PERFORMANCE row is taken with --release"]
fn the_cap_costs_a_catalogue_miss_on_a_new_series_and_nothing_after() {
    let retention = Retention {
        max_len: 16,
        max_age: Duration::from_secs(600),
        max_uncatalogued: 100_000,
    };
    let n = 2_000usize;
    let plugin: Vec<Sample> = (0..n)
        .map(|i| Sample {
            id: MetricId {
                name: "weather.temp",
                label: Label::Name(Arc::from(format!("city{i}").as_str())),
            },
            datum: Datum::Scalar(20.0),
        })
        .collect();
    let catalogued: Vec<Sample> = (0..n)
        .map(|i| {
            scalar(
                &gridwatch_store::keys::sensors::TEMP_C
                    .named(&Arc::from(format!("chip{i}:Sensor").as_str())),
                60.0,
            )
        })
        .collect();
    let feed = |store: &mut Store, samples: &[Sample], at: u64| {
        let t0 = std::time::Instant::now();
        store.apply(&Msg::Batch(Batch {
            source: SourceId("weather"),
            at: Ts(at),
            samples: samples.to_vec(),
        }));
        t0.elapsed() / n as u32
    };
    let mut a = Store::new(retention);
    let create_uncatalogued = feed(&mut a, &plugin, 0);
    // The same samples again: every series exists, so the `Vacant` arm — and
    // with it the whole cap — is never reached.
    let push_existing = feed(&mut a, &plugin, 1_000_000_000);
    let mut b = Store::new(retention);
    let create_catalogued = feed(&mut b, &catalogued, 0);

    // The miss on its own: `key::lookup` walks the catalogue and an
    // uncatalogued name never short-circuits, which is the cap's real cost —
    // not the `HashMap` increment the design note guessed at.
    let t0 = std::time::Instant::now();
    let mut hits = 0usize;
    for _ in 0..n {
        if gridwatch_store::key::lookup("weather.temp").is_some() {
            hits += 1;
        }
    }
    let miss = t0.elapsed() / n as u32;
    assert_eq!(hits, 0);

    println!(
        "cap: new uncatalogued series {create_uncatalogued:?} each, new catalogued series \
         {create_catalogued:?}, push to an existing series {push_existing:?}; the catalogue \
         miss alone is {miss:?}"
    );
    assert!(
        push_existing < create_uncatalogued,
        "the cap must cost nothing once the series exists: {push_existing:?} against \
         {create_uncatalogued:?}"
    );
    assert!(
        create_uncatalogued.saturating_sub(create_catalogued) < Duration::from_micros(5),
        "the uncatalogued path costs {:?} more than the catalogued one",
        create_uncatalogued.saturating_sub(create_catalogued)
    );
}

/// Arc 10 review — the sweep must not delete a value that is only ever
/// published once. D60 amendment 7 spared `Record` series because
/// `sensor.info` is published once, and then swept scalars — two of which
/// (`sensor.max_c`, `sensor.crit_c`) are published once by the same function,
/// three lines above `sensor.info`. `net.speed_mbps` and the three static gpu
/// clocks have the same shape. Eleven minutes into any default run they
/// vanished, permanently.
#[test]
fn a_scalar_published_once_survives_the_sweep() {
    let mut store = Store::default(); // the shipped Retention: 600 s
    let once = |key: &str, at: u64| {
        Msg::Batch(Batch {
            source: SourceId("sensors"),
            at: Ts(at * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::sensors::MAX_C
                    .named(&Arc::from(key))
                    .id,
                datum: Datum::Scalar(100.0),
            }],
        })
    };
    let tick = |at: u64| {
        Msg::Batch(Batch {
            source: SourceId("sensors"),
            at: Ts(at * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::sensors::TEMP_C
                    .named(&Arc::from("k10temp:Tctl"))
                    .id,
                datum: Datum::Scalar(50.0),
            }],
        })
    };
    store.apply(&once("k10temp:Tctl", 0));
    let max = gridwatch_store::keys::sensors::MAX_C.named(&Arc::from("k10temp:Tctl"));
    assert!(store.last(&max).is_some(), "published at t=0");
    // Twenty minutes of ordinary sampling; the threshold is never re-sent.
    for t in 1..1200u64 {
        store.apply(&tick(t));
    }
    assert!(
        store.last(&max).is_some(),
        "a threshold published once must outlive every sweep — the sensors tile \
         reads it for `over_max`, the sort key, and a rule may use it as its \
         right-hand side"
    );
    assert_eq!(store.last(&max).map(|(_, v)| v), Some(100.0));
}

// ----------------------------------------------------------------- arc 11

/// D61 from the other end: a chip's **thresholds** are published once per
/// generation and its readings arrive every second, so per-series liveness
/// would delete the thresholds while the chip is plugged in, and per-label
/// liveness must delete them when it is unplugged. Both halves, on the same
/// store.
///
/// The k10temp half is why the domain is the key name's prefix and not
/// `KeyMeta.source` (D61 trap 3): `sensor.temp_c`'s catalogue row says
/// `sensors`, but when the sensors feature is off the **cpu** source
/// publishes it (§16) — same name, same `chip:label` vocabulary. A rule that
/// asked which source a *batch* came from would let an nvme's label be kept
/// alive by the cpu's, or the reverse.
#[test]
fn a_chips_thresholds_live_and_die_with_its_readings() {
    use gridwatch_store::keys::sensors;
    let mut store = Store::new(Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    });
    let at = |t: u64| Ts(t * 1_000_000_000);
    let send = |store: &mut Store, source: SourceId, t: u64, samples: Vec<Sample>| {
        store.apply(&Msg::Batch(Batch {
            source,
            at: at(t),
            samples,
        }));
    };
    let nvme = Arc::from("nvme:Composite");
    let tctl = Arc::from("k10temp:Tctl");

    // Once per generation, at t = 0: the nvme's limits from the sensors
    // source, the k10temp's from the **cpu** source's handover.
    send(
        &mut store,
        sensors::SOURCE,
        0,
        vec![
            scalar(&sensors::MAX_C.named(&nvme), 84.0),
            scalar(&sensors::CRIT_C.named(&nvme), 88.0),
        ],
    );
    send(
        &mut store,
        cpu::SOURCE,
        0,
        vec![scalar(&sensors::MAX_C.named(&tctl), 95.0)],
    );
    // Both chips report for 200 s — three sweeps past `max_age`.
    for t in 0..200u64 {
        send(
            &mut store,
            sensors::SOURCE,
            t,
            vec![scalar(&sensors::TEMP_C.named(&nvme), 41.0)],
        );
        send(
            &mut store,
            cpu::SOURCE,
            t,
            vec![scalar(&sensors::TEMP_C.named(&tctl), 55.0)],
        );
    }
    assert_eq!(
        store.last(&sensors::MAX_C.named(&nvme)).map(|(_, v)| v),
        Some(84.0),
        "published once 200 s ago, for a chip that is still reporting: kept"
    );
    assert_eq!(
        store.last(&sensors::CRIT_C.named(&nvme)).map(|(_, v)| v),
        Some(88.0)
    );

    // The drive is pulled. The k10temp keeps reporting — under the *cpu*
    // source, which is the point.
    for t in 200..500u64 {
        send(
            &mut store,
            cpu::SOURCE,
            t,
            vec![scalar(&sensors::TEMP_C.named(&tctl), 55.0)],
        );
    }
    assert!(
        store.last(&sensors::TEMP_C.named(&nvme)).is_none(),
        "the chip stopped: its readings go"
    );
    assert!(
        store.last(&sensors::MAX_C.named(&nvme)).is_none(),
        "and so do its thresholds — the label is dead, not the series"
    );
    assert!(store.last(&sensors::CRIT_C.named(&nvme)).is_none());
    assert!(
        !store
            .labels("sensor.temp_c")
            .map(gridwatch_store::rules::label_text)
            .any(|l| l == "nvme:Composite"),
        "the sensors tile lists labels, and must not list a drive that is gone"
    );
    assert_eq!(
        store.last(&sensors::MAX_C.named(&tctl)).map(|(_, v)| v),
        Some(95.0),
        "a threshold published once at t=0 under `cpu`, kept alive 500 s by \
         readings of the same *name prefix* — the domain is `sensor`, not the \
         batch's source and not `KeyMeta.source`'s `sensors`"
    );
}

/// The shipped number, not a test-sized one: `Retention::max_uncatalogued` is
/// **512** series per domain of uncatalogued names, so a plugin publishing a
/// fresh label per message cannot take 11 GB in the ten minutes before a
/// sweep (D61). The 513th is refused and counted; nothing is queued.
///
/// This allocates 512 rings of `max_len` slots on purpose — ≈ 19 MB, which is
/// the worst case the default is chosen for.
#[test]
fn the_shipped_cap_is_512_series_of_one_uncatalogued_domain() {
    let retention = Retention::default();
    assert_eq!(retention.max_uncatalogued, 512, "the shipped default");
    let mut store = Store::new(retention);
    let weather = SourceId("weather");
    let samples: Vec<Sample> = (0..513)
        .map(|i| Sample {
            id: MetricId {
                name: "weather.temp",
                label: Label::Name(Arc::from(format!("city{i}").as_str())),
            },
            datum: Datum::Scalar(20.0),
        })
        .collect();
    store.apply(&Msg::Batch(Batch {
        source: weather,
        at: Ts(0),
        samples,
    }));
    assert_eq!(store.labels("weather.temp").count(), 512);
    assert_eq!(store.capped(weather), 1, "the 513th was refused");
    // Refused, never queued: sending it again refuses it again rather than
    // finding room that was silently held for it.
    store.apply(&Msg::Batch(Batch {
        source: weather,
        at: Ts(1_000_000_000),
        samples: vec![Sample {
            id: MetricId {
                name: "weather.temp",
                label: Label::Name(Arc::from("city512")),
            },
            datum: Datum::Scalar(21.0),
        }],
    }));
    assert_eq!(store.labels("weather.temp").count(), 512);
    assert_eq!(store.capped(weather), 2);
}

/// The `Rules::states` half of the same leak (arc 7b review): a `*`-labelled
/// rule accrued one state per label ever seen. States are evictable where
/// values are not — losing one is not losing data, because a label that comes
/// back gets a fresh state, which is what it would have had anyway. The
/// exception is a **raised** state: an `absent` rule steps from
/// `Series::last_at()`, so forgetting it under a live alert would leave the
/// alert unable to resolve.
#[test]
fn a_quiet_label_loses_its_rule_state_but_never_a_raised_one() {
    use gridwatch_store::rules::{Rules, parse_all};
    let mk = |src: &str| -> Rules {
        let tables: Vec<toml::Table> = [src].iter().map(|t| toml::from_str(t).unwrap()).collect();
        let (rules, errors) = parse_all(&tables, &|k| gridwatch_store::key::lookup(k).is_some());
        assert!(errors.is_empty(), "{errors:?}");
        Rules::new(rules)
    };
    let bps = |iface: &str, t: u64| {
        Msg::Batch(Batch {
            source: SourceId("net"),
            at: Ts(t * 1_000_000_000),
            samples: vec![Sample {
                id: gridwatch_store::keys::net::RX_BPS
                    .named(&Arc::from(iface))
                    .id,
                datum: Datum::Scalar(2000.0),
            }],
        })
    };
    let retention = Retention {
        max_len: 2400,
        max_age: Duration::from_secs(60),
        max_uncatalogued: 512,
    };

    // A threshold rule over every interface: three veths appear, publish and
    // vanish, and each leaves a state behind.
    let mut store = Store::new(retention);
    store.set_rules(mk(r#"
name = "busy link"
key = "net.rx_bps"
label = "*"
op = ">"
value = 1000000
for_s = 5
severity = "warn""#));
    for (i, veth) in ["veth1", "veth2", "veth3"].iter().enumerate() {
        for t in 0..5u64 {
            store.apply(&bps(veth, i as u64 * 5 + t));
        }
    }
    assert_eq!(
        store.rules().state_count(),
        3,
        "one state per interface seen"
    );
    for t in 20..300u64 {
        store.apply(&bps("eno1", t));
    }
    assert_eq!(
        store.rules().state_count(),
        1,
        "the states of labels that went quiet are forgotten"
    );

    // Now an `absent` rule that is actually raised.
    let mut store = Store::new(retention);
    store.set_rules(mk(r#"
name = "link quiet"
key = "net.rx_bps"
label = "veth*"
op = "absent"
for_s = 5"#));
    for t in 0..5u64 {
        store.apply(&bps("veth1", t));
    }
    let ev = store.tick_rules(Ts(30 * 1_000_000_000));
    assert_eq!(ev.len(), 1, "the veth went quiet: {ev:?}");
    assert_eq!(store.alerts().active().count(), 1);
    for t in 30..300u64 {
        store.apply(&bps("eno1", t));
    }
    assert_eq!(
        store.alerts().active().count(),
        1,
        "a raised alert keeps its state through every sweep"
    );
    // And the **series** is pinned too, not only the state (D61 trap 4): the
    // sweep skips a label a rule is raised for, because an `absent` rule
    // steps from `Series::last_at()` and an evicted series would freeze the
    // alert at raised for the rest of the run.
    assert!(
        store
            .last(&gridwatch_store::keys::net::RX_BPS.named(&Arc::from("veth1")))
            .is_some(),
        "the raised label's series was evicted; the alert can never resolve"
    );
    // And it resolves when the interface comes back — the assertion that
    // fails if `raised_for` is not asked before `forget`.
    store.apply(&bps("veth1", 300));
    let ev = store.tick_rules(Ts(301 * 1_000_000_000));
    assert_eq!(ev.len(), 1, "it must be able to resolve: {ev:?}");
    assert_eq!(store.alerts().active().count(), 0);
}

// ───────────────────── arc 13 seams (D63) ─────────────────────

/// The shipped `history = "10m"` must resolve to exactly today's retention,
/// field for field — that is what lets D63 promise no snapshot, replay or P18
/// number moves. 600 s / 250 ms = 2400, which is the `max_len` that shipped.
#[test]
fn ten_minutes_of_history_is_the_retention_that_already_shipped() {
    let d = Retention::default();
    let h = Retention::for_history(Duration::from_secs(600));
    assert_eq!(h.max_len, d.max_len, "max_len");
    assert_eq!(h.max_age, d.max_age, "max_age");
    assert_eq!(h.max_uncatalogued, d.max_uncatalogued, "max_uncatalogued");
}

/// Four points a second, by integer arithmetic at both ends of the range D63
/// allows — never a float, because determinism is config, not clock.
#[test]
fn history_derives_four_points_a_second() {
    assert_eq!(Retention::for_history(Duration::from_secs(60)).max_len, 240);
    assert_eq!(
        Retention::for_history(Duration::from_secs(3600)).max_len,
        14_400
    );
    // Never zero, however short the window.
    assert_eq!(Retention::for_history(Duration::from_millis(1)).max_len, 1);
}

/// `footprint()` counts scalar points and nothing else: a Record and a Vector
/// hold no ring of points, so they add to `series` alone.
#[test]
fn the_footprint_counts_the_points_a_scalar_holds() {
    let mut store = Store::new(Retention::default());
    for t in 0..10u64 {
        store.apply(&Msg::Batch(Batch {
            source: SourceId("net"),
            at: Ts(t * 1_000_000_000),
            samples: vec![
                scalar(
                    &gridwatch_store::keys::net::RX_BPS.named(&Arc::from("eno1")),
                    1.0,
                ),
                scalar(
                    &gridwatch_store::keys::net::TX_BPS.named(&Arc::from("eno1")),
                    2.0,
                ),
            ],
        }));
    }
    let f = store.footprint();
    assert_eq!(f.series, 2);
    assert_eq!(f.scalar_points, 20);
    assert_eq!(f.scalar_bytes, 320, "a scalar point is a Ts and an f64");
    assert_eq!(store.retention().max_age, Retention::default().max_age);
}
