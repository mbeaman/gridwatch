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
